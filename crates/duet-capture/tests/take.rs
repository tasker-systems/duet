//! A take written by `TakeWriter` reads back through `FileSource` as the samples written: exactly,
//! at each depth, at the rate and channels given; with silence where input was lost; and from a
//! writer that never finished. A path that exists is refused and left as it was.
//!
//! And a take's digest ([`Digester`]) is of those samples, not the file: one for the same
//! samples written as FLAC and as WAV, at 16 bits or re-encoded at 24, with or without tags, and
//! another once one sample moves by one step.

use std::fs;
use std::path::{Path, PathBuf};

use duet_capture::{Depth, Digester, Error, FileSource, Source, Take, TakeWriter};

/// A fresh scratch path under cargo's per-run target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("take");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = fs::remove_file(&path);
    path
}

/// Every sample of a file, with its rate and channels.
fn read_back(path: &Path) -> (u32, u16, Vec<f32>) {
    let mut source = FileSource::open(path).unwrap();
    let (mut rate, mut channels, mut samples) = (0, 0, Vec::new());
    let mut next = 0;
    while let Some(chunk) = source.next_chunk().unwrap() {
        assert_eq!(chunk.start_frame, next, "a gap in the chunks");
        next += (chunk.samples.len() / usize::from(chunk.channels)) as u64;
        (rate, channels) = (chunk.rate, chunk.channels);
        samples.extend(chunk.samples);
    }
    (rate, channels, samples)
}

/// `count` samples, each an exact step at `depth`, spread over the whole range with full
/// scale at both ends: a deterministic walk, so a failure reproduces.
fn steps(count: usize, depth: Depth) -> Vec<f32> {
    let scale = (1i64 << (depth.bits() - 1)) as f32;
    let mut state = 0x2545_f491_u32;
    let mut out: Vec<f32> = (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let step = (state >> (32 - depth.bits())) as i32 - scale as i32;
            step as f32 / scale
        })
        .collect();
    out[0] = -1.0;
    out[1] = (scale - 1.0) / scale;
    out
}

/// Writes `samples` in uneven pieces, so frame boundaries fall inside the writes.
fn write_in_pieces(writer: &mut TakeWriter, samples: &[f32], channels: usize) {
    let mut at = 0;
    for piece in [1usize, 511, 4096, 4097, 100].iter().cycle() {
        if at == samples.len() {
            break;
        }
        let len = (piece * channels).min(samples.len() - at);
        writer.write(&samples[at..at + len]).unwrap();
        at += len;
    }
}

#[test]
fn a_24_bit_mono_take_reads_back_exactly() {
    let path = scratch("mono24.flac");
    // Not a whole number of FLAC frames: the last is short.
    let samples = steps(3 * 4096 + 1234, Depth::TwentyFour);
    let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::TwentyFour).unwrap();
    write_in_pieces(&mut writer, &samples, 1);
    let written = writer.finish().unwrap();
    assert_eq!(written.frames, samples.len() as u64);
    let top = (8_388_607.0 / 8_388_608.0) as f32;
    let rails = samples.iter().filter(|&&s| s == -1.0 || s == top).count();
    assert!(rails >= 2, "the walk starts on both rails");
    assert_eq!(written.full_scale, rails as u64);

    let (rate, channels, back) = read_back(&path);
    assert_eq!((rate, channels), (48_000, 1));
    assert_eq!(back, samples);
}

#[test]
fn a_16_bit_stereo_take_reads_back_exactly() {
    let path = scratch("stereo16.flac");
    let samples = steps(2 * (2 * 4096 + 77), Depth::Sixteen);
    let mut writer = TakeWriter::create(&path, 44_100, 2, Depth::Sixteen).unwrap();
    write_in_pieces(&mut writer, &samples, 2);
    writer.finish().unwrap();

    let (rate, channels, back) = read_back(&path);
    assert_eq!((rate, channels), (44_100, 2));
    assert_eq!(back, samples);
}

/// A sample between steps is stored at the nearest one: never more than half a step away.
#[test]
fn a_sample_between_steps_is_stored_at_the_nearest() {
    let path = scratch("between.flac");
    let samples: Vec<f32> = (0..5000).map(|n| (n as f32 * 0.013).sin() * 0.7).collect();
    let mut writer = TakeWriter::create(&path, 22_050, 1, Depth::Sixteen).unwrap();
    writer.write(&samples).unwrap();
    writer.finish().unwrap();

    let (_, _, back) = read_back(&path);
    let half_step = 0.5 / 32_768.0;
    for (n, (a, b)) in samples.iter().zip(&back).enumerate() {
        assert!(
            (a - b).abs() <= half_step,
            "sample {n}: wrote {a}, read {b}"
        );
    }
}

#[test]
fn beyond_full_scale_is_stored_at_full_scale_and_counted() {
    let path = scratch("loud.flac");
    let mut writer = TakeWriter::create(&path, 22_050, 1, Depth::Sixteen).unwrap();
    let top = 32_767.0 / 32_768.0;
    // The last two are a 16-bit device's own rails, as cpal converts them: the top one
    // arrives just under 1.0 and still counts.
    writer
        .write(&[1.5, -2.0, 1.0, 0.5, f32::NAN, top, -1.0])
        .unwrap();
    let written = writer.finish().unwrap();
    assert_eq!(written.full_scale, 5);

    let (_, _, back) = read_back(&path);
    assert_eq!(back, [top, -1.0, top, 0.5, 0.0, top, -1.0]);
}

/// Lost input is written as silence, so what follows keeps its time.
#[test]
fn silence_stands_in_for_lost_input() {
    let path = scratch("gap.flac");
    let mut writer = TakeWriter::create(&path, 22_050, 2, Depth::Sixteen).unwrap();
    writer.write(&[0.25, -0.25]).unwrap();
    writer.silence(3).unwrap();
    writer.write(&[0.5, -0.5]).unwrap();
    assert_eq!(writer.frames(), 5);
    let written = writer.finish().unwrap();
    assert_eq!(written.frames, 5);

    let (_, _, back) = read_back(&path);
    assert_eq!(back, [0.25, -0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, -0.5]);
}

/// A writer dropped without finishing, as when the process dies mid-take, leaves a file that
/// reads: every whole FLAC frame written so far.
#[test]
fn an_unfinished_take_still_reads() {
    let path = scratch("unfinished.flac");
    let samples = steps(3 * 4096 + 500, Depth::TwentyFour);
    let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::TwentyFour).unwrap();
    writer.write(&samples).unwrap();
    drop(writer);

    let (rate, channels, back) = read_back(&path);
    assert_eq!((rate, channels), (48_000, 1));
    assert_eq!(back, samples[..3 * 4096]);
}

#[test]
fn a_take_never_replaces_a_file() {
    let path = scratch("exists.flac");
    fs::write(&path, b"not mine to replace").unwrap();
    let refused = TakeWriter::create(&path, 48_000, 1, Depth::TwentyFour).unwrap_err();
    assert!(
        matches!(&refused, Error::Invalid(msg) if msg.contains("exists")),
        "{refused}"
    );
    assert_eq!(fs::read(&path).unwrap(), b"not mine to replace");
}

#[test]
fn a_partial_frame_is_refused() {
    let path = scratch("partial.flac");
    let mut writer = TakeWriter::create(&path, 48_000, 2, Depth::Sixteen).unwrap();
    assert!(matches!(writer.write(&[0.0; 3]), Err(Error::Invalid(_))));
}

/// The take's digest, as `duet capture` computes it: every chunk `FileSource` yields.
fn digest(path: &Path) -> Take {
    let mut source = FileSource::open(path).unwrap();
    let mut digester = Digester::new();
    while let Some(chunk) = source.next_chunk().unwrap() {
        digester.push(&chunk).unwrap();
    }
    digester.finish().expect("the file holds audio")
}

/// A PCM WAV of `samples` (16-bit steps, `-1.0..1.0`), at `bits` (16 or 24), `channels`
/// interleaved; with a `LIST` chunk of tags before the audio when `tagged`.
fn write_wav(path: &Path, samples: &[f32], rate: u32, channels: u16, bits: u16, tagged: bool) {
    let width = usize::from(bits / 8);
    let mut data = Vec::with_capacity(samples.len() * width);
    for &sample in samples {
        let step = (sample * 32_768.0) as i32;
        assert_eq!(step as f32 / 32_768.0, sample, "{sample} is no 16-bit step");
        let value = step << (bits - 16);
        data.extend_from_slice(&value.to_le_bytes()[..width]);
    }
    let mut tags = Vec::new();
    if tagged {
        let title = b"Wayfaring, take 3\0";
        let mut info = b"INFO".to_vec();
        info.extend(b"INAM");
        info.extend((title.len() as u32).to_le_bytes());
        info.extend(title);
        tags.extend(b"LIST");
        tags.extend((info.len() as u32).to_le_bytes());
        tags.extend(info);
    }
    let block = channels * (bits / 8);
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((4 + 24 + tags.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * u32::from(block)).to_le_bytes());
    bytes.extend(block.to_le_bytes());
    bytes.extend(bits.to_le_bytes());
    bytes.extend(tags);
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    fs::write(path, bytes).unwrap();
}

/// A FLAC of `samples`, written by `TakeWriter` at `depth`.
fn write_flac(path: &Path, samples: &[f32], rate: u32, channels: u16, depth: Depth) {
    let mut writer = TakeWriter::create(path, rate, channels, depth).unwrap();
    write_in_pieces(&mut writer, samples, usize::from(channels));
    writer.finish().unwrap();
}

#[test]
fn the_same_samples_as_flac_and_as_wav_have_one_digest() {
    // Stereo, not a whole number of FLAC frames, every 16-bit step from rail to rail.
    let samples = steps(2 * (2 * 4096 + 777), Depth::Sixteen);
    let flac = scratch("digest16.flac");
    let wav = scratch("digest16.wav");
    write_flac(&flac, &samples, 44_100, 2, Depth::Sixteen);
    write_wav(&wav, &samples, 44_100, 2, 16, false);
    let take = digest(&flac);
    assert_eq!(digest(&wav), take);
    assert_eq!(
        (take.rate, take.channels, take.frames),
        (44_100, 2, (samples.len() / 2) as u64)
    );
    // Tags before the audio change the file, not the take.
    let tagged = scratch("digest16-tagged.wav");
    write_wav(&tagged, &samples, 44_100, 2, 16, true);
    assert_ne!(fs::read(&tagged).unwrap(), fs::read(&wav).unwrap());
    assert_eq!(digest(&tagged), take);
}

#[test]
fn one_sample_moved_by_one_step_changes_the_digest() {
    let samples = steps(3000, Depth::Sixteen);
    let base = scratch("digest-base.flac");
    write_flac(&base, &samples, 22_050, 1, Depth::Sixteen);
    let mut moved = samples.clone();
    // Up one step, from a sample well below the top rail.
    let i = (1000..moved.len()).find(|&i| moved[i] < 0.5).unwrap();
    moved[i] += 1.0 / 32_768.0;
    let one_step = scratch("digest-step.flac");
    write_flac(&one_step, &moved, 22_050, 1, Depth::Sixteen);
    let (before, after) = (digest(&base), digest(&one_step));
    assert_ne!(after.digest, before.digest);
    assert_eq!(
        (after.rate, after.channels, after.frames),
        (before.rate, before.channels, before.frames)
    );
    // A take of the same samples at another rate is another take.
    let other_rate = scratch("digest-rate.flac");
    write_flac(&other_rate, &samples, 44_100, 1, Depth::Sixteen);
    assert_ne!(digest(&other_rate).digest, before.digest);
}

#[test]
fn sixteen_bit_samples_re_encoded_at_24_bits_keep_their_digest() {
    // Symphonia reads an integer sample as the sample over 2^(bits-1), exact for 16 and 24 bits
    // alike, so a 16-bit step shifted up 8 bits is the same f32, and the same take: the
    // digester's docs state it. Checked here as WAV and as FLAC.
    let samples = steps(2 * 5000, Depth::Sixteen);
    let sixteen = scratch("depth16.wav");
    write_wav(&sixteen, &samples, 48_000, 2, 16, false);
    let take = digest(&sixteen);
    let wav24 = scratch("depth24.wav");
    write_wav(&wav24, &samples, 48_000, 2, 24, false);
    let flac24 = scratch("depth24.flac");
    write_flac(&flac24, &samples, 48_000, 2, Depth::TwentyFour);
    assert_eq!(read_back(&wav24).2, samples);
    assert_eq!(digest(&wav24), take);
    assert_eq!(digest(&flac24), take);
}

/// One 4,096-sample block of 24-bit steps: a quiet tone at `bed` of full scale, with a burst
/// over `burst` of a tone at `pitch` of the Nyquist rate and `level` times full scale, so that
/// it clips at both rails. A recorded beatbox take clips like this.
fn clipped_block(bed: f32, pitch: f32, level: f32, burst: std::ops::Range<usize>) -> Vec<f32> {
    (0..4096)
        .map(|n| {
            step24(if burst.contains(&n) {
                (std::f32::consts::PI * pitch * n as f32).sin() * level
            } else {
                (n as f32 * 0.05).sin() * bed
            })
        })
        .collect()
}

/// `x` at the nearest 24-bit step, full scale where it goes beyond.
fn step24(x: f32) -> f32 {
    let scale = 8_388_608.0f32;
    (x * scale).round().clamp(-scale, scale - 1.0) / scale
}

/// Clipped 24-bit blocks, each with the bytes the reference encoder's frame of it takes:
/// libFLAC 1.5.0's `flac -8 --no-padding --no-seektable`, the frame's bits as
/// `flac --analyze` gives them, over 8.
fn clipped_blocks() -> Vec<(&'static str, u16, Vec<f32>, u64)> {
    let burst = clipped_block(0.001, 0.3, 2.0, 2048..2816);
    let tone = clipped_block(0.0, 0.3, 4.0, 2048..4096);
    let wide = clipped_block(0.1, 0.25, 4.0, 2048..4096);
    // Stereo: the second channel the first a little later and half as loud.
    let stereo: Vec<f32> = (0..4096)
        .flat_map(|n| [wide[n], step24(wide[(n + 7) % 4096] * 0.5)])
        .collect();
    vec![
        (
            "a clipped burst in a quiet block",
            1,
            burst,
            BURST_REFERENCE,
        ),
        (
            "half a block of a clipped tone in silence",
            1,
            tone,
            TONE_REFERENCE,
        ),
        (
            "a clipped tone over a louder one, stereo",
            2,
            stereo,
            STEREO_REFERENCE,
        ),
    ]
}

const BURST_REFERENCE: u64 = 3620;
const TONE_REFERENCE: u64 = 3041;
const STEREO_REFERENCE: u64 = 8686;

/// The bytes before a take's first frame: the stream marker and `STREAMINFO`.
const STREAM_HEADER: u64 = 4 + 4 + 34;

/// A clipped full-scale 24-bit block is written in no more than twice the bytes of the reference
/// encoder's frame of it, and never in more than its samples stored as they are. flacenc 0.5.1
/// alone wrote the first block here as a 139 KB frame and a take's like it as 100 MB.
#[test]
fn a_clipped_block_encodes_to_a_sane_size() {
    for (name, channels, samples, reference) in clipped_blocks() {
        let path = scratch(&format!("clipped-{channels}-{reference}.flac"));
        let mut writer = TakeWriter::create(&path, 48_000, channels, Depth::TwentyFour).unwrap();
        writer.write(&samples).unwrap();
        writer.finish().unwrap();
        let frame = fs::metadata(&path).unwrap().len() - STREAM_HEADER;
        assert!(
            frame <= 2 * reference,
            "{name}: a {frame}-byte frame, the reference's {reference}"
        );
        // A verbatim frame: each sample in 3 bytes, a subframe header byte for each channel,
        // and at most 18 bytes of frame header and checksum.
        let verbatim = samples.len() as u64 * 3 + u64::from(channels) + 18;
        assert!(
            frame <= verbatim,
            "{name}: a {frame}-byte frame, raw {verbatim}"
        );
    }
}

/// What is written for a clipped block reads back as the samples given.
#[test]
fn a_clipped_block_reads_back_exactly() {
    for (name, channels, samples, _) in clipped_blocks() {
        let path = scratch(&format!("clipped-back-{channels}-{}.flac", name.len()));
        let mut writer = TakeWriter::create(&path, 48_000, channels, Depth::TwentyFour).unwrap();
        write_in_pieces(&mut writer, &samples, usize::from(channels));
        writer.finish().unwrap();
        let (rate, read_channels, back) = read_back(&path);
        assert_eq!((rate, read_channels), (48_000, channels), "{name}");
        assert!(back == samples, "{name}: read back differs");
    }
}
