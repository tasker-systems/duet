//! The residual: what a reading's notes leave of the performance, split per note into centre,
//! drift, modulation, transition and rest, and summed over the take into the loss.
//!
//! On every voiced frame a note covers, the residual is `performance − (tuning + 100·key)`, in
//! cents: how far the singing is from the note's key on the take's tuning. A slide into a note
//! is measured against the note it arrives at. A voiced frame no note covers has no residual:
//! it is **unread**, and is counted apart. The per-frame residual is never written: the
//! performance and the reading rebuild it.
//!
//! # The loss, and the note HMM's objective
//!
//! The loss prices each frame as the note HMM ([`NoteModel`](crate::NoteModel)) decodes it, in
//! nats, from the same functions: so the reading the decoder chooses is the one of least loss,
//! up to the differences [`Loss`] names. Each frame's price is taken relative to the cheapest
//! the frame could be, read or unread, so every term is at least zero:
//! - a voiced frame of voicing `v` costs `−ln v + ln(σ√2π)` in a note ([`sounding_cost`]) and
//!   `−ln(1 − v)` left unread ([`silent_cost`]): the decoder's observation of it in a note
//!   state and in silence. What exceeds the cheaper of the two goes in the loss's `voicing`
//!   term for a read frame, and in its `unread` term ([`unread_cost`]) for an unread one. So a
//!   strongly voiced frame is dear to leave out and a breath is free to, and a breath read as
//!   a note costs what its voicing says it is not;
//! - a held frame `d` semitones from its key costs `d²/2σ²` ([`held_cost`]) more, in `pitch`;
//! - a slide frame, one a note covers before its held pitch begins (its `held_from`), costs a
//!   flat [`ATTACK_COST`](crate::ATTACK_COST) more, in `pitch`: it is measured as slide, as the
//!   decoder's attack is, and not as though held on the key it arrives at. It is still the
//!   note's, by the #28 ruling;
//! - a frame the pitch track decoded unvoiced costs `−ln DROPOUT` inside a note, in `voicing`,
//!   and nothing outside one;
//! - every note costs `c`, in `notes`.
//!
//! All of it is plain arithmetic over one note's frames at a time (a least-squares line, a
//! two-term sinusoid fit and an autocorrelation over a few dozen lags), so no array library is
//! needed for it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::hmm::{ATTACK_COST, DROPOUT, STABLE_SIGMA_SEMITONES, VOICING_FLOOR};
use crate::reading::{PerformanceFrames, ReadNote, Tuning};

/// The held-pitch tolerance of the loss, `σ`, in cents: the note HMM's
/// [`STABLE_SIGMA_SEMITONES`], half a key. A frame held this far from its key costs half a nat
/// in the loss's `pitch` term, as it does in the decoder.
pub const LOSS_SIGMA_CENTS: f64 = STABLE_SIGMA_SEMITONES * 100.0;
/// The cost of a note, `c`, in nats: the note HMM's cost per note ([`NoteModel`](crate::NoteModel)),
/// the negative log probability of leaving a held note, and the loss's `notes` term per note.
/// pYIN's note HMM leaves its stable state with probability 0.01, a cost of 4.6. Raising it
/// merges notes; lowering it splits them. It is set by ear and on the synthetic witnesses of
/// the three rulings, never by the residual alone, since one note per frame has the least
/// residual of all.
///
/// It was derived as the shortest note's 4 frames times a flat unread cost of 1. The unread
/// cost is now the frame's voicing's ([`unread_cost`]), about 2 nats for a frame voiced at 0.9,
/// so 4 is two such frames' worth; it stands until the owner re-judges it by ear.
pub const NOTE_COST: f64 = 4.0;

/// A voicing as the decoder and the loss take it: floored at [`VOICING_FLOOR`] either way.
fn floored(voicing: f64) -> f64 {
    voicing.clamp(VOICING_FLOOR, 1.0 - VOICING_FLOOR)
}

/// What a voiced frame of `voicing` costs in a note, before its pitch, in nats: `−ln v` less
/// the held pitch's log density at its peak, `ln(σ√2π)` with `σ` in semitones. A frame held
/// exactly on its key costs this; an attack frame costs [`ATTACK_COST`] more.
#[must_use]
pub fn sounding_cost(voicing: f64) -> f64 {
    -floored(voicing).ln() + (STABLE_SIGMA_SEMITONES * std::f64::consts::TAU.sqrt()).ln()
}

/// What a voiced frame of `voicing` costs left unread, in nats: `−ln(1 − v)`, the decoder's
/// silent state.
#[must_use]
pub fn silent_cost(voicing: f64) -> f64 {
    -(1.0 - floored(voicing)).ln()
}

/// What a held frame `semitones` from its key costs beyond [`sounding_cost`], in nats:
/// `d²/2σ²`, the held pitch's Normal density.
#[must_use]
pub fn held_cost(semitones: f64) -> f64 {
    semitones * semitones / (2.0 * STABLE_SIGMA_SEMITONES * STABLE_SIGMA_SEMITONES)
}

/// What a voiced frame of `voicing` costs the loss left unread, in nats: what leaving it out
/// costs beyond the cheaper of leaving it out and holding it exactly on a key. About 2 at a
/// voicing of 0.9 and 4.4 at the floor's 0.99; nothing below a voicing of about 0.56, where
/// leaving the frame out is already the cheaper.
#[must_use]
pub fn unread_cost(voicing: f64) -> f64 {
    (silent_cost(voicing) - sounding_cost(voicing)).max(0.0)
}

/// What a voiced frame of `voicing` costs the loss read, beyond its pitch, in nats: what
/// sounding it costs beyond the cheaper of leaving it out and holding it on a key.
fn read_voicing_cost(voicing: f64) -> f64 {
    (sounding_cost(voicing) - silent_cost(voicing)).max(0.0)
}
/// The slowest and fastest vibrato looked for, in cycles a second.
const VIBRATO_HZ: (f64, f64) = (3.0, 9.0);
/// The least normalized autocorrelation at a vibrato's period for it to be one: a held note's
/// jitter and a tracker's noise correlate well below it.
const VIBRATO_CORRELATION: f64 = 0.5;
/// The shallowest vibrato, in cents either way: a tracker's jitter can correlate at some lag,
/// but it does not swing this far.
const SHALLOWEST_VIBRATO_CENTS: f64 = 5.0;
/// How far, in cents, beyond a vibrato's depth the first or last frame of a note must be from
/// its centre to make a slide in or a fall off.
const TRANSITION_CENTS: f32 = 30.0;
/// The fewest held frames a note's parts are measured from; fewer give only its centre.
const FEWEST_HELD: usize = 4;
/// The width of a bin of the take's centre histogram, in cents.
const CENTRE_BIN_CENTS: f32 = 10.0;

/// A slow move across a note's held span: a least-squares line through its held residual.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Drift {
    /// The line's slope, in cents a second.
    pub cents_per_sec: f32,
    /// How far the line moves over the held span, in cents.
    pub range_cents: f32,
}

/// A vibrato: the held residual, less its drift, swinging at a steady rate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Modulation {
    /// Cycles a second, where the autocorrelation peaks between 3 and 9.
    pub rate_hz: f32,
    /// Half its peak-to-peak swing, in cents: the amplitude of the sinusoid fitted at the rate.
    pub depth_cents: f32,
}

/// A slide into a note, or a fall off its end.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Transition {
    /// The residual where it is furthest from the note: at the note's first voiced frame for a
    /// slide in, its last for a fall, in cents from the key.
    pub cents: f32,
    /// How long it lasts, in milliseconds: from the note's start to where the pitch first
    /// reaches its centre, or from where it last leaves the centre to the note's end.
    pub ms: f32,
}

/// One note's residual, split as Melodyne splits a note (a concept here, not code).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoteResidual {
    /// Where the note is held, against its key, in cents: the mean held residual over a whole
    /// number of the vibrato's cycles, centred on the held span, or the plain mean without one.
    /// The note's own `centre` is its held frames' median, as segmentation reads it.
    pub centre: f32,
    pub drift: Drift,
    /// Absent when no vibrato is found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulation: Option<Modulation>,
    /// The slide in. Absent when the note starts on its pitch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoop: Option<Transition>,
    /// The fall off its end. Absent when it ends on its pitch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fall: Option<Transition>,
    /// What is left: the root mean square of the held residual less centre, drift and
    /// modulation, in cents.
    pub rest_cents: f32,
    /// The voiced frames the note covers, its slide and fall included.
    pub frames: u32,
}

/// The loss of a reading, in nats, as its terms: `pitch`, `voicing`, `unread` and `notes`
/// (the module's doc gives each frame's price). It is never reported as one number alone, so
/// that a reading that wins by adding notes, or by leaving singing out, shows it.
///
/// It is the note HMM's objective: for two readings of one take, the difference of their
/// totals is the difference of their paths' costs to the decoder, but for these, each named
/// so that a reader of the loss knows where the two part:
/// - **onsets:** the loss is the same whatever the onsets; the decoder leaves a held note at
///   an onset for `−ln 0.9` ([`ONSET_LEAVE`](crate::ONSET_LEAVE)) rather than `c`, and holds
///   one across an onset for `−ln 0.1`. The loss is reckoned from the frames and the notes
///   alone, so that any reading of a performance can be scored, with or without its onsets;
/// - **staying held:** the decoder charges `−ln(1 − e^−c)` (0.018 at `c` = 4) for every frame
///   a note holds past the shortest note's; the loss leaves it out, as too small to move a
///   note;
/// - **the decoder's notes the reading drops:** a decoded note shorter than the shortest note,
///   or less voiced than the rules allow one, is dropped from the reading, as the rules drop
///   it, and its frames are unread in the loss;
/// - **a slide is what its reader says:** the frames before a note's `held_from` are its
///   slide. For the HMM that is its attack; for the rules it is until the pitch arrives and
///   holds for 50 ms.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Loss {
    /// `Σ d²/2σ²` over every held frame a note covers, its fall included, with `σ`
    /// [`LOSS_SIGMA_CENTS`], and [`ATTACK_COST`](crate::ATTACK_COST) for every frame of its
    /// slide, before its `held_from`. Before `capture-analysis` 1.4 it was `Σ r²/σ²` over every
    /// frame, the slide's measured against the key as though held.
    pub pitch: f64,
    /// For every frame a note covers, what sounding it costs beyond leaving it out: nothing for
    /// a voicing above about 0.56, and `−ln DROPOUT` for a frame the pitch track decoded
    /// unvoiced. Added in `capture-analysis` 1.4; absent, and taken as 0, before.
    #[serde(default)]
    pub voicing: f64,
    /// [`unread_cost`] for every voiced frame no note covers: what leaving it out costs, by its
    /// voicing. Before `capture-analysis` 1.4 it was 1 a frame, whatever the voicing.
    pub unread: f64,
    /// `c` for every note: [`NOTE_COST`] unless the reader was given another, the reading's
    /// `reader.note_cost`.
    pub notes: f64,
}

impl Loss {
    /// The terms' sum, for comparing two readings of one take.
    #[must_use]
    pub fn total(&self) -> f64 {
        self.pitch + self.voicing + self.unread + self.notes
    }
}

/// How the take's notes' centres spread: a histogram in 10-cent bins from -50.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CentreHistogram {
    /// Where the first bin starts, in cents.
    pub from_cents: f32,
    /// How wide each bin is, in cents.
    pub bin_cents: f32,
    /// How many notes' centres fall in each bin; one beyond the ends counts in the end bin.
    pub counts: Vec<u32>,
}

/// What a reading's residual says about the take as a whole. The tuning and its spread are the
/// reading's `tuning`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TakeResidual {
    pub centres: CentreHistogram,
    /// How the notes' centres move over the take, in cents a second: a least-squares line
    /// through each note's centre against the middle of its held span. Absent with fewer than
    /// two notes. A reference that drifts within the take shows here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre_cents_per_sec: Option<f32>,
    /// How many notes are between keys.
    pub between_keys: u32,
    /// The voiced frames the notes cover.
    pub read_frames: u32,
    /// The voiced frames no note covers.
    pub unread_frames: u32,
    pub loss: Loss,
}

/// A least-squares line through `(x, y)`: its slope and intercept. A flat line at the mean for
/// fewer than two points or no spread in `x`.
fn line(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    if points.is_empty() {
        return (0.0, 0.0);
    }
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let (sxy, sxx) = points.iter().fold((0.0, 0.0), |(sxy, sxx), (x, y)| {
        (sxy + (x - mx) * (y - my), sxx + (x - mx) * (x - mx))
    });
    if sxx <= f64::EPSILON {
        return (0.0, my);
    }
    let slope = sxy / sxx;
    (slope, my - slope * mx)
}

/// The vibrato's rate in `detrended`, a note's held residual less a line through it, at `fps`
/// frames a second: the shortest lag between 3 and 9 Hz where the normalized autocorrelation
/// has a local peak of at least `VIBRATO_CORRELATION`, placed between lags by a parabola. The
/// shortest, not the highest: a vibrato correlates as well at two periods as at one.
fn vibrato_rate(detrended: &[f64], fps: f64) -> Option<f64> {
    let n = detrended.len();
    let energy: f64 = detrended.iter().map(|x| x * x).sum();
    let shortest = (fps / VIBRATO_HZ.1).floor() as usize;
    let longest = ((fps / VIBRATO_HZ.0).ceil() as usize).min(n / 2);
    if energy <= f64::EPSILON || shortest < 2 || longest <= shortest {
        return None;
    }
    let correlation = |lag: usize| -> f64 {
        let sum: f64 = (0..n - lag)
            .map(|i| detrended[i] * detrended[i + lag])
            .sum();
        sum / energy * n as f64 / (n - lag) as f64
    };
    let lags: Vec<f64> = (shortest - 1..=longest + 1).map(correlation).collect();
    let best = (1..lags.len() - 1).find(|&i| {
        lags[i] >= VIBRATO_CORRELATION && lags[i] >= lags[i - 1] && lags[i] >= lags[i + 1]
    })?;
    let (a, b, c) = (lags[best - 1], lags[best], lags[best + 1]);
    let denominator = a - 2.0 * b + c;
    let shift = if denominator.abs() > f64::EPSILON {
        (0.5 * (a - c) / denominator).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(fps / ((best + shortest - 1) as f64 + shift))
}

/// Solves `a·x = b` by Gaussian elimination with partial pivoting. `None` when `a` is singular.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let pivot = (col..N).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..N {
            let f = a[row][col] / a[col][col];
            for k in col..N {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let sum: f64 = (row + 1..N).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - sum) / a[row][row];
    }
    Some(x)
}

/// A least-squares fit of `y` on the columns of `basis`, row by row: its coefficients, and the
/// root mean square of what it leaves.
fn fit<const N: usize>(rows: &[([f64; N], f64)]) -> Option<([f64; N], f64)> {
    let mut a = [[0.0; N]; N];
    let mut b = [0.0; N];
    for (x, y) in rows {
        for i in 0..N {
            b[i] += x[i] * y;
            for j in 0..N {
                a[i][j] += x[i] * x[j];
            }
        }
    }
    let coefficients = solve(a, b)?;
    let left: f64 = rows
        .iter()
        .map(|(x, y)| (y - (0..N).map(|i| coefficients[i] * x[i]).sum::<f64>()).powi(2))
        .sum();
    Some((coefficients, (left / rows.len().max(1) as f64).sqrt()))
}

/// The mean of `held`, over a whole number of `period`-frame cycles centred on it, or of all
/// of it when no cycle fits.
fn cycle_mean(held: &[f64], period: Option<f64>) -> f64 {
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len().max(1) as f64;
    let Some(period) = period else {
        return mean(held);
    };
    let cycles = (held.len() as f64 / period).floor();
    if cycles < 1.0 {
        return mean(held);
    }
    let span = ((cycles * period).round() as usize).clamp(1, held.len());
    let from = (held.len() - span) / 2;
    mean(&held[from..from + span])
}

/// Where a slide toward `centre` along `residual` (its frames in order from where it starts)
/// first reaches the centre: the index of the first frame at or past it. `None` when the first
/// frame is already within `band` of the centre.
fn arrival(residual: &[f64], centre: f64, band: f64) -> Option<usize> {
    let first = *residual.first()?;
    if (first - centre).abs() <= band {
        return None;
    }
    let below = first < centre;
    residual
        .iter()
        .position(|&r| if below { r >= centre } else { r <= centre })
}

fn median(xs: &[f64]) -> f64 {
    let mut sorted = xs.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    match sorted.len() {
        0 => 0.0,
        n if n % 2 == 1 => sorted[mid],
        _ => (sorted[mid - 1] + sorted[mid]) / 2.0,
    }
}

/// One note's parts, from its voiced frames' `(time in seconds, residual in cents)`, in order,
/// and where segmentation found its held pitch to begin, in seconds.
///
/// The held span is found in two passes. The first takes the frames from where segmentation
/// says the pitch is held, for the vibrato's depth, and their median for where the note sits: a
/// median, which a fall at the end cannot drag. The slide in is then the frames from the start
/// until the pitch first reaches that, when the first frame is further from it than the
/// vibrato's depth and [`TRANSITION_CENTS`]; the fall is the same from the end. The parts are
/// measured on what lies between.
fn measure(frames: &[(f64, f64)], held_from_secs: f64, fps: f64) -> NoteResidual {
    let residuals: Vec<f64> = frames.iter().map(|f| f.1).collect();
    let n = frames.len();
    let segmented = frames
        .iter()
        .position(|f| f.0 >= held_from_secs)
        .filter(|&i| n - i >= FEWEST_HELD)
        .unwrap_or(0);
    let first_pass = parts(&frames[segmented..], fps);
    let band = f64::from(first_pass.modulation.map_or(0.0, |m| m.depth_cents) + TRANSITION_CENTS);
    let centre = median(&residuals[segmented..]);
    if n < FEWEST_HELD {
        return parts(frames, fps);
    }
    let held_from = arrival(&residuals, centre, band).unwrap_or(0);
    let mut reversed = residuals.clone();
    reversed.reverse();
    let held_to = n - arrival(&reversed, centre, band).unwrap_or(0);
    if held_to <= held_from + FEWEST_HELD {
        let mut out = parts(frames, fps);
        out.frames = n as u32;
        return out;
    }
    let mut out = parts(&frames[held_from..held_to], fps);
    let ms = |a: usize, b: usize| ((frames[b].0 - frames[a].0) * 1000.0) as f32;
    if held_from > 0 {
        out.scoop = Some(Transition {
            cents: residuals[0] as f32,
            ms: ms(0, held_from),
        });
    }
    if held_to < n {
        out.fall = Some(Transition {
            cents: residuals[n - 1] as f32,
            ms: ms(held_to - 1, n - 1),
        });
    }
    out.frames = n as u32;
    out
}

/// Centre, drift, modulation and rest of `held`, its frames' `(time, residual)`.
fn parts(held: &[(f64, f64)], fps: f64) -> NoteResidual {
    let residuals: Vec<f64> = held.iter().map(|f| f.1).collect();
    let mean = residuals.iter().sum::<f64>() / residuals.len().max(1) as f64;
    let mut out = NoteResidual {
        centre: mean as f32,
        drift: Drift {
            cents_per_sec: 0.0,
            range_cents: 0.0,
        },
        modulation: None,
        scoop: None,
        fall: None,
        rest_cents: 0.0,
        frames: held.len() as u32,
    };
    if held.len() < FEWEST_HELD {
        return out;
    }
    let t0 = held[0].0;
    let span = held[held.len() - 1].0 - t0;
    let (slope, intercept) = line(held);
    let detrended: Vec<f64> = held
        .iter()
        .map(|&(t, r)| r - (slope * t + intercept))
        .collect();
    // The line alone is pulled by a vibrato across a span of no whole number of its cycles, so
    // with one the line and the sinusoid at its rate are fitted together.
    let vibrato = vibrato_rate(&detrended, fps).and_then(|rate| {
        let w = std::f64::consts::TAU * rate;
        let rows: Vec<([f64; 4], f64)> = held
            .iter()
            .map(|&(t, r)| {
                let (s, c) = (w * (t - t0)).sin_cos();
                ([1.0, t - t0, s, c], r)
            })
            .collect();
        let ([_, slope, a, b], rest) = fit(&rows)?;
        let depth = a.hypot(b);
        (depth >= SHALLOWEST_VIBRATO_CENTS).then_some((rate, depth, slope, rest))
    });
    let (slope, rest) = match vibrato {
        Some((_, _, slope, rest)) => (slope, rest),
        None => {
            let rows: Vec<([f64; 2], f64)> =
                held.iter().map(|&(t, r)| ([1.0, t - t0], r)).collect();
            fit(&rows).map_or((slope, 0.0), |([_, slope], rest)| (slope, rest))
        }
    };
    out.drift = Drift {
        cents_per_sec: slope as f32,
        range_cents: (slope * span).abs() as f32,
    };
    out.centre = cycle_mean(&residuals, vibrato.map(|(rate, ..)| fps / rate)) as f32;
    out.rest_cents = rest as f32;
    out.modulation = vibrato.map(|(rate, depth, ..)| Modulation {
        rate_hz: rate as f32,
        depth_cents: depth as f32,
    });
    out
}

/// Each note's residual and the take's, for `notes` read on `tuning` from `frames`, with the
/// loss counting [`NOTE_COST`] per note.
#[must_use]
pub fn residual(
    frames: &PerformanceFrames,
    tuning: &Tuning,
    notes: &[ReadNote],
) -> (Vec<NoteResidual>, TakeResidual) {
    residual_with_cost(frames, tuning, notes, NOTE_COST)
}

/// [`residual`], with the loss counting `note_cost` per note: the cost the reader read the notes
/// with.
#[must_use]
pub fn residual_with_cost(
    frames: &PerformanceFrames,
    tuning: &Tuning,
    notes: &[ReadNote],
    note_cost: f64,
) -> (Vec<NoteResidual>, TakeResidual) {
    let fps = f64::from(frames.rate) / f64::from(frames.hop);
    let rate = f64::from(frames.rate);
    let mut covered = vec![false; frames.len()];
    let (mut pitch, mut voicing) = (0.0, 0.0);
    let mut per_note = Vec::with_capacity(notes.len());
    for note in notes {
        let target = tuning.a4.value() + 100.0 * (f64::from(note.key) - 69.0);
        let mut voiced = Vec::new();
        for (k, &cents) in frames.cents.iter().enumerate() {
            let at = frames.frame(k);
            if at < note.start || at >= note.end {
                continue;
            }
            // A frame two notes cover is priced once, by the first.
            let priced = !std::mem::replace(&mut covered[k], true);
            if cents.is_nan() {
                if priced {
                    voicing += -DROPOUT.ln();
                }
                continue;
            }
            let r = f64::from(cents) - target;
            if priced {
                pitch += if at < note.held_from {
                    ATTACK_COST
                } else {
                    held_cost(r / 100.0)
                };
                voicing += read_voicing_cost(f64::from(frames.voicing[k]));
            }
            voiced.push((at as f64 / rate, r));
        }
        per_note.push(measure(&voiced, note.held_from as f64 / rate, fps));
    }
    let (mut read, mut unread, mut unread_loss) = (0, 0, 0.0);
    for (k, &cents) in frames.cents.iter().enumerate() {
        match (cents.is_nan(), covered[k]) {
            (true, _) => {}
            (false, true) => read += 1,
            (false, false) => {
                unread += 1;
                unread_loss += unread_cost(f64::from(frames.voicing[k]));
            }
        }
    }
    let loss = Loss {
        pitch,
        voicing,
        unread: unread_loss,
        notes: notes.len() as f64 * note_cost,
    };
    let bins = (100.0 / CENTRE_BIN_CENTS) as usize;
    let mut counts = vec![0u32; bins];
    for part in &per_note {
        let bin = ((part.centre + 50.0) / CENTRE_BIN_CENTS).floor();
        counts[(bin.max(0.0) as usize).min(bins - 1)] += 1;
    }
    let centres: Vec<(f64, f64)> = notes
        .iter()
        .zip(&per_note)
        .map(|(note, part)| {
            let middle = (note.held_from + note.end) as f64 / 2.0 / rate;
            (middle, f64::from(part.centre))
        })
        .collect();
    let take = TakeResidual {
        centres: CentreHistogram {
            from_cents: -50.0,
            bin_cents: CENTRE_BIN_CENTS,
            counts,
        },
        centre_cents_per_sec: (centres.len() >= 2).then(|| line(&centres).0 as f32),
        between_keys: notes.iter().filter(|n| n.between_keys).count() as u32,
        read_frames: read as u32,
        unread_frames: unread as u32,
        loss,
    };
    (per_note, take)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::{FrameBlock, PerformanceHead};
    use crate::reading::{A440_CENTS, read};
    use crate::stage::{Mono, Stage};
    use crate::{Event, Layer1, Performance, synth};

    const RATE: u32 = 22_050;
    const HOP: u32 = 256;
    const KEY: u8 = 60;

    fn secs(k: usize) -> f64 {
        f64::from(k as u32 * HOP) / f64::from(RATE)
    }

    fn frame_at(t: f64) -> u64 {
        (t * f64::from(RATE)).round() as u64
    }

    /// Frames over `total` seconds whose pitch, in cents from key 60 on A440, is `pitch(t)`, or
    /// unvoiced where it gives `None`.
    fn frames(total: f64, pitch: impl Fn(f64) -> Option<f64>) -> PerformanceFrames {
        let head = PerformanceHead::new(RATE, HOP, 0);
        let n = (total * f64::from(RATE) / f64::from(HOP)) as usize;
        let column: Vec<Option<f32>> = (0..n)
            .map(|k| pitch(secs(k)).map(|c| (A440_CENTS - 900.0 + c) as f32))
            .collect();
        let mut out = PerformanceFrames::new(&head);
        out.push(&FrameBlock::new(0, column, vec![0.9; n], vec![0.1; n]).unwrap())
            .unwrap();
        out
    }

    fn note(start: f64, held_from: f64, end: f64) -> ReadNote {
        ReadNote {
            start: frame_at(start),
            end: frame_at(end),
            held_from: frame_at(held_from),
            key: KEY,
            centre: 0.0,
            between_keys: false,
            confidence: 0.9,
            strength: 0.5,
            residual: None,
        }
    }

    fn on_a440() -> Tuning {
        Tuning::given(A440_CENTS)
    }

    fn within(found: f32, want: f64, share: f64, what: &str) {
        let tolerance = (want.abs() * share).max(0.5);
        assert!(
            (f64::from(found) - want).abs() <= tolerance,
            "{what}: {found}, wanted {want} ± {tolerance}"
        );
    }

    #[test]
    fn a_held_note_is_split_into_its_parts_within_ten_percent() {
        // A 300 ms scoop from 300 cents below, then held 1.5 s at +20 cents with a 6 Hz
        // vibrato of ±40 cents and a drift of 10 cents a second, centred on the held span.
        let (start, arrive, end) = (0.2, 0.5, 2.0);
        let middle = (arrive + end) / 2.0;
        let held = |t: f64| {
            20.0 + 10.0 * (t - middle) + 40.0 * (std::f64::consts::TAU * 6.0 * (t - arrive)).sin()
        };
        let frames = frames(2.3, |t| {
            if t < start || t >= end {
                None
            } else if t < arrive {
                let from = -300.0;
                Some(from + (held(arrive) - from) * (t - start) / (arrive - start))
            } else {
                Some(held(t))
            }
        });
        // Segmentation finds the held pitch a little after the scoop arrives.
        let notes = [note(start, arrive + 0.06, end)];
        let (parts, take) = residual(&frames, &on_a440(), &notes);
        let part = parts[0];
        within(part.centre, 20.0, 0.1, "centre");
        within(part.drift.cents_per_sec, 10.0, 0.1, "drift");
        within(part.drift.range_cents, 15.0, 0.1, "drift range");
        let vibrato = part.modulation.expect("a vibrato");
        within(vibrato.rate_hz, 6.0, 0.1, "vibrato rate");
        within(vibrato.depth_cents, 40.0, 0.1, "vibrato depth");
        let scoop = part.scoop.expect("a scoop");
        within(scoop.cents, -300.0, 0.1, "scoop depth");
        within(scoop.ms, 300.0, 0.1, "scoop length");
        assert_eq!(part.fall, None);
        assert!(part.rest_cents < 4.0, "{part:?}");
        assert_eq!(take.unread_frames, 0);
        assert_eq!(take.read_frames, part.frames);
    }

    #[test]
    fn a_note_with_no_vibrato_reports_none() {
        // Held at +5 cents with a tracker's jitter of up to 3 cents, frame to frame.
        let jitter = |t: f64| {
            let k = (t * 1000.0).round() as u64;
            let x = k
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((x >> 33) as f64 / f64::from(1u32 << 31) - 0.5) * 6.0
        };
        let frames = frames(1.4, |t| (0.2..1.2).contains(&t).then(|| 5.0 + jitter(t)));
        let (parts, _) = residual(&frames, &on_a440(), &[note(0.2, 0.2, 1.2)]);
        let part = parts[0];
        assert_eq!(part.modulation, None, "{part:?}");
        assert_eq!((part.scoop, part.fall), (None, None), "{part:?}");
        within(part.centre, 5.0, 0.3, "centre");
        assert!(part.drift.cents_per_sec.abs() < 3.0, "{part:?}");
        assert!(part.rest_cents < 3.0, "{part:?}");
    }

    #[test]
    fn a_fall_off_the_end_is_measured() {
        let frames = frames(1.6, |t| match t {
            t if (0.2..1.2).contains(&t) => Some(0.0),
            t if (1.2..1.35).contains(&t) => Some(-200.0 * (t - 1.2) / 0.15),
            _ => None,
        });
        let (parts, _) = residual(&frames, &on_a440(), &[note(0.2, 0.2, 1.35)]);
        let fall = parts[0].fall.expect("a fall");
        within(fall.cents, -200.0, 0.1, "fall depth");
        within(fall.ms, 150.0, 0.1, "fall length");
        assert_eq!(parts[0].scoop, None);
        within(parts[0].centre, 0.0, 0.1, "centre");
    }

    #[test]
    fn splitting_one_held_note_in_two_raises_the_loss() {
        let frames = frames(1.4, |t| (0.2..1.2).contains(&t).then_some(8.0));
        let whole = [note(0.2, 0.2, 1.2)];
        let split = [note(0.2, 0.2, 0.7), note(0.7, 0.7, 1.2)];
        let (_, one) = residual(&frames, &on_a440(), &whole);
        let (_, two) = residual(&frames, &on_a440(), &split);
        // The same frames, held the same distance from the same key: only the notes' cost
        // differs, so the split is worse by one note's.
        assert!((one.loss.pitch - two.loss.pitch).abs() < 1e-9);
        assert_eq!((one.loss.unread, two.loss.unread), (0.0, 0.0));
        assert!(two.loss.total() > one.loss.total(), "{one:?} {two:?}");
        assert_eq!(two.loss.notes - one.loss.notes, (0.05 * 86.13f64).round());
        // And leaving singing out is worse than reading it near its key.
        let short = [note(0.2, 0.2, 0.9)];
        let (_, less) = residual(&frames, &on_a440(), &short);
        assert!(less.unread_frames > 0);
        assert!(less.loss.total() > one.loss.total(), "{less:?} {one:?}");
    }

    /// Frames as [`frames`] makes them, with each frame's voicing `voicing(t)`.
    fn voiced_as(
        total: f64,
        pitch: impl Fn(f64) -> Option<f64>,
        voicing: impl Fn(f64) -> f32,
    ) -> PerformanceFrames {
        let mut out = frames(total, pitch);
        for (k, v) in out.voicing.iter_mut().enumerate() {
            *v = voicing(secs(k));
        }
        out
    }

    #[test]
    fn dropping_a_held_note_raises_the_loss() {
        // Strongly voiced singing: held on the key, held 45 cents off it, and arriving by a
        // 150 ms scoop from 300 cents below. Each is dearer left out than read, its slide
        // measured as slide.
        let scoop = |t: f64| -300.0 * (1.0 - (t - 0.2) / 0.15).max(0.0);
        type Pitch<'a> = &'a dyn Fn(f64) -> f64;
        let takes: [(&str, Pitch, f64); 3] = [
            ("on its key", &|_| 0.0, 0.2),
            ("45 cents off", &|_| 45.0, 0.2),
            ("scooped", &scoop, 0.35),
        ];
        for (name, pitch, held_from) in takes {
            for secs in [0.2, 1.0] {
                let end = 0.35 + secs;
                let frames = frames(end + 0.2, |t| (0.2..end).contains(&t).then(|| pitch(t)));
                let (_, read) = residual(&frames, &on_a440(), &[note(0.2, held_from, end)]);
                let (_, dropped) = residual(&frames, &on_a440(), &[]);
                assert_eq!(dropped.read_frames, 0);
                assert!(
                    dropped.loss.total() > read.loss.total(),
                    "{name}, {secs} s: dropped {:?}, read {:?}",
                    dropped.loss,
                    read.loss
                );
            }
        }
    }

    #[test]
    fn a_breath_is_cheaper_left_out_than_read() {
        let frames = voiced_as(0.8, |t| (0.2..0.6).contains(&t).then_some(0.0), |_| 0.02);
        let (_, read) = residual(&frames, &on_a440(), &[note(0.2, 0.2, 0.6)]);
        let (_, dropped) = residual(&frames, &on_a440(), &[]);
        assert_eq!(dropped.loss.unread, 0.0, "{dropped:?}");
        assert!(read.loss.voicing > 0.0, "{read:?}");
        assert!(dropped.loss.total() < read.loss.total());
    }

    #[test]
    fn an_unread_frame_costs_by_its_voicing() {
        // Leaving a strongly voiced frame out costs more than reading it anywhere within half
        // a key, held or as slide; leaving a breath out costs nothing.
        for v in [0.7, 0.9, 0.99] {
            assert!(unread_cost(v) >= held_cost(0.5).max(ATTACK_COST), "{v}");
        }
        assert_eq!(unread_cost(0.02), 0.0);
        assert!(unread_cost(0.9) < unread_cost(0.95));
    }

    #[test]
    fn only_voiced_frames_are_read_or_unread() {
        // Voiced in two stretches with silence between, one note over the first.
        let frames = frames(2.0, |t| {
            ((0.2..0.6).contains(&t) || (1.0..1.3).contains(&t)).then_some(0.0)
        });
        let (_, take) = residual(&frames, &on_a440(), &[note(0.1, 0.2, 0.8)]);
        let voiced = frames.cents.iter().filter(|c| !c.is_nan()).count() as u32;
        assert_eq!(take.read_frames + take.unread_frames, voiced);
        assert!(take.read_frames > 0 && take.unread_frames > 0);
        // Every frame is voiced at 0.9, so each unread one costs that voicing's price.
        let each = unread_cost(f64::from(0.9f32));
        assert!((take.loss.unread - f64::from(take.unread_frames) * each).abs() < 1e-9);
    }

    #[test]
    fn a_take_summarizes_its_notes_centres() {
        // Three notes held at -20, +5 and +30 cents: the centres drift sharp over the take.
        let frames = frames(3.4, |t| match t {
            t if (0.2..1.0).contains(&t) => Some(-20.0),
            t if (1.2..2.0).contains(&t) => Some(5.0),
            t if (2.2..3.0).contains(&t) => Some(30.0),
            _ => None,
        });
        let notes = [
            note(0.2, 0.2, 1.0),
            note(1.2, 1.2, 2.0),
            note(2.2, 2.2, 3.0),
        ];
        let (_, take) = residual(&frames, &on_a440(), &notes);
        // Bins of 10 cents from -50: -20 is in [-20, -10), +5 in [0, 10), +30 in [30, 40).
        assert_eq!(take.centres.counts, vec![0, 0, 0, 1, 0, 1, 0, 0, 1, 0]);
        within(
            take.centre_cents_per_sec.unwrap(),
            25.0,
            0.1,
            "centre slope",
        );
    }

    #[test]
    fn a_sung_vibrato_is_measured_through_the_whole_reading() {
        let signal = [
            synth::silence(RATE, 0.25),
            synth::vibrato(RATE, 261.63, 1.5, 40.0, 6.0, 4, 0.3),
            synth::silence(RATE, 0.25),
        ]
        .concat();
        let mut stage = Performance::new(RATE);
        let mut out = Vec::new();
        stage
            .push(
                Mono {
                    rate: RATE,
                    start_frame: 0,
                    samples: signal,
                },
                &mut out,
            )
            .unwrap();
        stage.finish(&mut out).unwrap();
        let mut frames = None;
        let mut onsets = Vec::new();
        for layer in out {
            match layer {
                Layer1::Head(head) => frames = Some(PerformanceFrames::new(&head)),
                Layer1::Frames(block) => frames.as_mut().unwrap().push(&block).unwrap(),
                Layer1::Onset { frame, strength } => onsets.push(Event::Onset { frame, strength }),
            }
        }
        let reading = read(&frames.unwrap(), &onsets, Some(A440_CENTS));
        assert_eq!(reading.notes.len(), 1, "{reading:?}");
        let part = reading.notes[0].residual.expect("a residual");
        let vibrato = part.modulation.expect("a vibrato");
        within(vibrato.rate_hz, 6.0, 0.1, "rate");
        within(vibrato.depth_cents, 40.0, 0.15, "depth");
        let take = reading.residual.expect("a take residual");
        assert_eq!(take.read_frames, part.frames);
    }
}
