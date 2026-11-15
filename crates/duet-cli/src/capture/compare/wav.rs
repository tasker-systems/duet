//! The comparison's audio: the take beside the reading, as a two-channel WAV, and the reading
//! rendered as one sine. Hand-written, with no dependency.

use std::f64::consts::TAU;

use duet_capture::Reading;

/// How loud the reading's sine is: below a full-scale take, so the two sit together.
const SINE_LEVEL: f64 = 0.3;
/// How long the sine fades in at its first note and out at its last, in seconds, so it starts
/// and stops without a click.
const FADE_SECS: f64 = 0.01;

/// Where key `key` sits on the reading's tuning, in absolute cents: `tuning + 100·key`, with
/// A4 (key 69) at the tuning.
pub fn key_cents(reading: &Reading, key: u8) -> f64 {
    reading.tuning.a4.value() + 100.0 * (f64::from(key) - 69.0)
}

/// Absolute cents as Hz: 6900 is 440 Hz.
pub fn hz(cents: f64) -> f64 {
    440.0 * ((cents - 6900.0) / 1200.0).exp2()
}

/// The pitch the reading's sine is at on sample frame `at`, in absolute cents, or `None` before
/// its first note and after its last.
///
/// The sine is legato: one continuous tone from the first note's start to the last note's end.
/// It holds each note's key, on the take's tuning, from where its held pitch begins
/// (`held_from`) until the next note starts, through any gap between them, and glides in a
/// straight line in cents only through a note's slide in (`start..held_from`), from the note
/// before to this one. The first note has nothing to glide from, and holds its key from its
/// start.
pub fn sine_cents(reading: &Reading, at: u64) -> Option<f64> {
    let notes = &reading.notes;
    let first = notes.first()?;
    let last = notes.last()?;
    if at < first.start || at >= last.end {
        return None;
    }
    // The last note that has started by `at`.
    let i = notes.partition_point(|n| n.start <= at).saturating_sub(1);
    let note = &notes[i];
    let here = key_cents(reading, note.key);
    if i == 0 || at >= note.held_from || note.held_from <= note.start {
        return Some(here);
    }
    let before = key_cents(reading, notes[i - 1].key);
    let along = (at - note.start) as f64 / (note.held_from - note.start) as f64;
    Some(before + (here - before) * along)
}

/// The reading as a sine at `rate`, one sample for each frame of `from..to`.
pub fn sine(reading: &Reading, rate: u32, from: u64, to: u64) -> Vec<f32> {
    let fade = FADE_SECS * f64::from(rate);
    let (Some(first), Some(last)) = (reading.notes.first(), reading.notes.last()) else {
        return vec![0.0; (to - from) as usize];
    };
    let mut phase = 0.0_f64;
    (from..to)
        .map(|at| {
            let Some(cents) = sine_cents(reading, at) else {
                return 0.0;
            };
            let envelope = ((at - first.start) as f64 / fade)
                .min((last.end - at) as f64 / fade)
                .min(1.0);
            let sample = SINE_LEVEL * envelope * phase.sin();
            phase = (phase + TAU * hz(cents) / f64::from(rate)) % TAU;
            sample as f32
        })
        .collect()
}

/// A two-channel WAV of 32-bit float samples at `rate`: `left` and `right`, which are one
/// length. Float keeps the take's samples exactly as the analysis heard them.
pub fn stereo(rate: u32, left: &[f32], right: &[f32]) -> Vec<u8> {
    debug_assert_eq!(left.len(), right.len());
    const CHANNELS: u16 = 2;
    const BYTES: u16 = 4;
    let frames = left.len() as u32;
    let data = frames * u32::from(CHANNELS * BYTES);
    let mut out = Vec::with_capacity(58 + data as usize);
    out.extend(b"RIFF");
    out.extend((50 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(18u32.to_le_bytes());
    out.extend(3u16.to_le_bytes()); // IEEE float
    out.extend(CHANNELS.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * u32::from(CHANNELS * BYTES)).to_le_bytes());
    out.extend((CHANNELS * BYTES).to_le_bytes());
    out.extend((BYTES * 8).to_le_bytes());
    out.extend(0u16.to_le_bytes()); // no extension
    out.extend(b"fact");
    out.extend(4u32.to_le_bytes());
    out.extend(frames.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for (l, r) in left.iter().zip(right) {
        out.extend(l.to_le_bytes());
        out.extend(r.to_le_bytes());
    }
    out
}
