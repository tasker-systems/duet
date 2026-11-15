//! Every v1 format decodes to the same tone, and what cannot be decoded is refused by name.
//!
//! The fixtures are one second of a 440 Hz sine, made by `scripts/capture-fixtures.sh`.

use std::fs;
use std::path::{Path, PathBuf};

use duet_capture::{ANALYSIS_RATE, Error, Event, FileSource, Mono, Onsets, Prepare, Source, Stage};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A scratch file under cargo's per-run target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(dir).unwrap();
    dir.join(name)
}

/// A file through `FileSource` and `Prepare`, to the end of the stream.
fn prepared(path: &Path) -> Vec<Mono> {
    let mut source = FileSource::open(path).unwrap();
    let mut prepare = Prepare::new(ANALYSIS_RATE);
    let mut out = Vec::new();
    while let Some(chunk) = source.next_chunk().unwrap() {
        prepare.push(chunk, &mut out).unwrap();
    }
    prepare.finish(&mut out).unwrap();
    out
}

/// The strongest frequency in a window of the stream, and the width of one bin.
fn dominant_hz(mono: &[Mono]) -> (f32, f32) {
    const WINDOW: usize = 8192;
    let samples: Vec<f32> = mono
        .iter()
        .flat_map(|m| m.samples.iter().copied())
        .collect();
    // Skip the first 0.1 s: lossy codecs start with a ramp.
    let start = ANALYSIS_RATE as usize / 10;
    let mut buffer: Vec<Complex<f32>> = samples[start..start + WINDOW]
        .iter()
        .enumerate()
        .map(|(n, s)| {
            let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / WINDOW as f32).cos();
            Complex::new(s * hann, 0.0)
        })
        .collect();
    FftPlanner::new()
        .plan_fft_forward(WINDOW)
        .process(&mut buffer);
    let (bin, _) = buffer[..WINDOW / 2]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.norm().total_cmp(&b.1.norm()))
        .unwrap();
    let bin_hz = ANALYSIS_RATE as f32 / WINDOW as f32;
    (bin as f32 * bin_hz, bin_hz)
}

fn assert_one_second_of_440(name: &str) {
    let mono = prepared(&fixture(name));
    assert!(
        mono.iter().all(|m| m.rate == ANALYSIS_RATE),
        "{name}: wrong rate"
    );
    let frames: usize = mono.iter().map(|m| m.samples.len()).sum();
    let want = f64::from(ANALYSIS_RATE);
    assert!(
        (frames as f64 - want).abs() <= want * 0.01,
        "{name}: {frames} frames, wanted {want} within 1%"
    );
    let (hz, bin_hz) = dominant_hz(&mono);
    assert!(
        (hz - 440.0).abs() <= bin_hz,
        "{name}: dominant {hz} Hz, wanted 440 within {bin_hz}"
    );
}

#[test]
fn every_v1_format_decodes_to_the_same_tone() {
    for name in [
        "tone-mono44.wav",
        "tone-mono44.flac",
        "tone-mono44.mp3",
        "tone-mono44.ogg",
        "tone-mono44.m4a",
        "tone-stereo48.flac",
        "tone-stereo48.mp3",
        "tone-stereo48.ogg",
        "tone-stereo48.m4a",
    ] {
        assert_one_second_of_440(name);
    }
}

#[test]
fn stereo_and_48k_give_what_mono_44k_gives() {
    assert_one_second_of_440("tone-stereo48.wav");
}

#[test]
fn he_aac_is_refused_by_name() {
    let err = FileSource::open(&fixture("he-aac.m4a")).unwrap_err();
    assert!(matches!(err, Error::Undecodable { .. }), "{err:?}");
    let message = err.to_string();
    assert!(message.contains("he-aac.m4a"), "{message}");
    assert!(message.contains("HE-AAC"), "{message}");
}

#[test]
fn a_text_file_is_refused_by_name() {
    let path = scratch("not-really-audio.mp3");
    fs::write(&path, "this is words, not music\n".repeat(200)).unwrap();
    let err = FileSource::open(&path).unwrap_err();
    assert!(matches!(err, Error::Undecodable { .. }), "{err:?}");
    assert!(err.to_string().contains("not-really-audio.mp3"), "{err}");
}

/// A 16-bit mono WAV of `seconds` of a 440 Hz sine at 44.1 kHz.
fn write_sine_wav(path: &Path, seconds: u32) {
    const RATE: u32 = 44_100;
    let frames = RATE * seconds;
    let data_len = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_len).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes()); // PCM
    bytes.extend(1u16.to_le_bytes()); // mono
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_len.to_le_bytes());
    for n in 0..frames {
        let x = (2.0 * std::f32::consts::PI * 440.0 * n as f32 / RATE as f32).sin();
        bytes.extend(((x * 16384.0) as i16).to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn chunks_are_bounded() {
    let path = scratch("sixty-seconds.wav");
    write_sine_wav(&path, 60);
    let mut source = FileSource::open(&path).unwrap();
    let (mut chunks, mut frames) = (0, 0usize);
    while let Some(chunk) = source.next_chunk().unwrap() {
        let in_chunk = chunk.samples.len() / usize::from(chunk.channels);
        assert!(
            in_chunk <= chunk.rate as usize,
            "a chunk holds {in_chunk} frames at {} Hz",
            chunk.rate
        );
        chunks += 1;
        frames += in_chunk;
    }
    assert!(chunks > 1, "the file came as one chunk");
    assert_eq!(frames, 60 * 44_100);
}

#[test]
fn a_damaged_packet_leaves_a_gap_in_start_frames_not_a_drift() {
    let mut bytes = fs::read(fixture("tone-mono44.flac")).unwrap();
    for byte in &mut bytes[10_000..10_200] {
        *byte ^= 0xff;
    }
    let path = scratch("damaged.flac");
    fs::write(&path, bytes).unwrap();

    let mut source = FileSource::open(&path).unwrap();
    let (mut decoded, mut end) = (0u64, 0u64);
    while let Some(chunk) = source.next_chunk().unwrap() {
        let frames = chunk.samples.len() as u64 / u64::from(chunk.channels);
        decoded += frames;
        end = chunk.start_frame + frames;
    }
    assert!(decoded < 44_100, "nothing was damaged: {decoded} frames");
    assert_eq!(end, 44_100, "the stream's end drifted");
}

/// Every undamaged fixture comes as chunks that follow on from frame 0 with no gap and end at
/// exactly its length: a gap counted where there is none would move every note after it.
#[test]
fn an_undamaged_file_comes_whole_from_frame_0() {
    let tones = ["wav", "flac", "mp3", "ogg", "m4a"];
    let mut files: Vec<(String, u64)> = Vec::new();
    for (stem, rate) in [("tone-mono44", 44_100), ("tone-stereo48", 48_000)] {
        files.extend(tones.iter().map(|ext| (format!("{stem}.{ext}"), rate)));
    }
    files.push(("notes22.flac".into(), 55_125));
    files.push(("notes44.flac".into(), 110_250));
    for (name, total) in files {
        let mut source = FileSource::open(&fixture(&name)).unwrap();
        let mut next = 0;
        while let Some(chunk) = source.next_chunk().unwrap() {
            assert_eq!(chunk.start_frame, next, "{name}: a chunk out of place");
            next += chunk.samples.len() as u64 / u64::from(chunk.channels);
        }
        assert_eq!(next, total, "{name}: the stream ends at the wrong frame");
    }
}

/// A copy of a fixture with 200 bytes inverted, three fifths of the way into the file. In the
/// two `notes` fixtures that costs the FLAC reader a few thousand frames before 1 s, which it
/// skips, leaving a gap in the chunks' start frames.
fn damaged(name: &str) -> PathBuf {
    let mut bytes = fs::read(fixture(name)).unwrap();
    let from = bytes.len() * 3 / 5;
    for byte in &mut bytes[from..from + 200] {
        *byte ^= 0xff;
    }
    let path = scratch(&format!("damaged-{name}"));
    fs::write(&path, bytes).unwrap();
    path
}

/// The frames a file decodes to, counted chunk by chunk.
fn decoded_frames(path: &Path) -> u64 {
    let mut source = FileSource::open(path).unwrap();
    let mut frames = 0;
    while let Some(chunk) = source.next_chunk().unwrap() {
        frames += chunk.samples.len() as u64 / u64::from(chunk.channels);
    }
    frames
}

/// The onset frames found in a file, through `FileSource`, `Prepare` and `Onsets`.
fn onsets_in(path: &Path) -> Vec<u64> {
    let mut onsets = Onsets::new(ANALYSIS_RATE);
    let mut events = Vec::new();
    for mono in prepared(path) {
        onsets.push(mono, &mut events).unwrap();
    }
    onsets.finish(&mut events).unwrap();
    events
        .iter()
        .filter_map(|e| match e {
            Event::Onset { frame, .. } => Some(*frame),
            _ => None,
        })
        .collect()
}

/// A damaged file, passed through at the analysis rate or resampled to it, still reaches the
/// analysis whole: the tones after the damage (the last two, from 1.75 s) have their onsets
/// where the undamaged file has them, within a hop of the onset stage's 128 frames, and
/// nothing fails.
#[test]
fn a_damaged_file_keeps_its_later_onsets_where_they_were() {
    const HOP: u64 = 128;
    let after = (1.7 * f64::from(ANALYSIS_RATE)) as u64;
    for name in ["notes22.flac", "notes44.flac"] {
        let clean: Vec<u64> = onsets_in(&fixture(name))
            .into_iter()
            .filter(|&f| f >= after)
            .collect();
        assert!(clean.len() >= 2, "{name}: {clean:?}");
        let path = damaged(name);
        assert!(
            decoded_frames(&path) < decoded_frames(&fixture(name)),
            "{name}: nothing was damaged"
        );
        let found = onsets_in(&path);
        for &want in &clean {
            assert!(
                found.iter().any(|f| f.abs_diff(want) <= HOP),
                "{name}: no onset within a hop of frame {want}: {found:?}"
            );
        }
    }
}
