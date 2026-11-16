//! The comparison's audio: the take beside the reading, as a two-channel WAV, and the reading
//! rendered as a sine that bridges short gaps and falls silent at rests ([`Gaps`]). Hand-written, with no dependency.

use std::f64::consts::TAU;

use duet_capture::{ReadNote, Reading};

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

/// Where a note's slide in ends, as a frame at `rate`: its scoop's end when its residual
/// measured one (capture-analysis 1.2), which is where the pitch first reaches the note, or
/// else where its held pitch begins (`held_from`). Never past the note's end.
pub fn slide_end(note: &ReadNote, rate: u32) -> u64 {
    note.residual
        .and_then(|r| r.scoop)
        .map_or(note.held_from, |scoop| {
            note.start + (f64::from(scoop.ms) / 1000.0 * f64::from(rate)).round() as u64
        })
        .min(note.end)
}

/// The longest gap between two notes that the sine bridges, in seconds: a gap shorter than this
/// is held through, and one this long or longer is silent.
///
/// A fifth of a second sits between the two things a gap can be. A consonant between two sung
/// vowels, or a catch-breath inside a phrase, lasts about 0.05–0.15 s, and the sine silent
/// through each would break a phrase into the beeps of a gapped rendering. A breath between
/// phrases, or a rest, lasts 0.25 s or more, and the sine must fall silent there, so a note the
/// reading drops is heard as a gap and not as the note before it held on.
pub const BRIDGE_SECS: f64 = 0.2;

/// What the sine does in a gap between two notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gaps {
    /// It holds the note before through a gap shorter than [`BRIDGE_SECS`], and is silent
    /// through a longer one: the default.
    Bridged,
    /// It holds the note before through every gap: one tone from the first note to the last.
    Legato,
    /// It is silent through every gap, sounding only inside a note.
    Gapped,
}

impl Gaps {
    /// Whether the sine holds through a gap of `frames` at `rate`.
    fn bridges(self, frames: u64, rate: u32) -> bool {
        match self {
            Gaps::Legato => true,
            Gaps::Gapped => frames == 0,
            Gaps::Bridged => (frames as f64) < BRIDGE_SECS * f64::from(rate),
        }
    }
}

/// The spans the sine sounds through, as `(first note, start, end)`: each runs from a note's
/// start to the end of the last note it bridges to.
fn phrases(reading: &Reading, rate: u32, gaps: Gaps) -> Vec<(usize, u64, u64)> {
    let mut out: Vec<(usize, u64, u64)> = Vec::new();
    for (i, note) in reading.notes.iter().enumerate() {
        match out.last_mut() {
            Some((_, _, end)) if gaps.bridges(note.start.saturating_sub(*end), rate) => {
                *end = (*end).max(note.end);
            }
            _ => out.push((i, note.start, note.end)),
        }
    }
    out
}

/// The pitch the reading's sine is at on sample frame `at` (at `rate`), in absolute cents, or
/// `None` where it is silent: before the first note, after the last, and in each gap between
/// `phrases` ([`phrases`]).
///
/// Within a phrase (the notes it bridges between), the sine is one continuous tone. It holds
/// each note's key, on the take's tuning, from where its slide in ends ([`slide_end`]) until the
/// next note starts, through any gap it bridges, and glides in a straight line in cents only
/// through a note's slide in, from the note before to this one. A phrase's first note has
/// nothing to glide from, and holds its key from its start.
fn sine_cents(reading: &Reading, rate: u32, phrases: &[(usize, u64, u64)], at: u64) -> Option<f64> {
    let p = phrases
        .partition_point(|&(_, start, _)| start <= at)
        .checked_sub(1)?;
    let (first, _, end) = phrases[p];
    if at >= end {
        return None;
    }
    let notes = &reading.notes;
    // The last note that has started by `at`.
    let i = notes.partition_point(|n| n.start <= at).saturating_sub(1);
    let note = &notes[i];
    let here = key_cents(reading, note.key);
    let slid = slide_end(note, rate);
    if i == first || at >= slid || slid <= note.start {
        return Some(here);
    }
    let before = key_cents(reading, notes[i - 1].key);
    let along = (at - note.start) as f64 / (slid - note.start) as f64;
    Some(before + (here - before) * along)
}

/// The reading as a sine at `rate`, one sample for each frame of `from..to`, silent in each gap
/// `gaps` does not bridge. Each phrase fades in at its start and out at its end.
pub fn sine(reading: &Reading, rate: u32, gaps: Gaps, from: u64, to: u64) -> Vec<f32> {
    let fade = FADE_SECS * f64::from(rate);
    let phrases = phrases(reading, rate, gaps);
    let mut phase = 0.0_f64;
    (from..to)
        .map(|at| {
            let Some(cents) = sine_cents(reading, rate, &phrases, at) else {
                return 0.0;
            };
            let p = phrases.partition_point(|&(_, start, _)| start <= at) - 1;
            let (_, start, end) = phrases[p];
            let envelope = ((at - start) as f64 / fade)
                .min((end - at) as f64 / fade)
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
