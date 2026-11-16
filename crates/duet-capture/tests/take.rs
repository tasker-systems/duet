//! A take written by `TakeWriter` reads back through `FileSource` as the samples written: exactly,
//! at each depth, at the rate and channels given; with silence where input was lost; and from a
//! writer that never finished. A path that exists is refused and left as it was.

use std::fs;
use std::path::{Path, PathBuf};

use duet_capture::{Depth, Error, FileSource, Source, TakeWriter};

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
    assert_eq!(written.full_scale, 1, "the one -1.0");

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
    writer.write(&[1.5, -2.0, 1.0, 0.5, f32::NAN]).unwrap();
    let written = writer.finish().unwrap();
    assert_eq!(written.full_scale, 3);

    let (_, _, back) = read_back(&path);
    let top = 32_767.0 / 32_768.0;
    assert_eq!(back, [top, -1.0, top, 0.5, 0.0]);
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
