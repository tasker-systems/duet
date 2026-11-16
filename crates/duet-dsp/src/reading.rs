//! The reading: a take's notes read from its kept performance, on the take's own tuning.
//!
//! The order is tuning, then notes (then, in assembly, the key):
//!
//! 1. **The tuning** is where A4 sits, given (`--tuning`) or estimated from the frames
//!    ([`estimate_tuning`]).
//! 2. **The notes** are segmented by [`Melody`](crate::Melody)'s rules, over the performance's
//!    pitch in cents and its onsets, and each is given the key nearest its held pitch on that
//!    tuning, with its `centre`: how far, in cents, its held pitch sits from that key.
//! 3. **The residual** of each note, and of the take, with the loss, is measured against
//!    those keys on that tuning ([`residual`](crate::residual)).
//!
//! A reading is a function over the settled performance, as `estimate` and `assemble` are over
//! settled events: the whole take is kept as frames (about 86 a second), never as samples.
//!
//! # Unvoiced frames
//!
//! The performance writes an unvoiced frame's pitch as `null`. Loaded here
//! ([`PerformanceFrames`]), it is `NaN`: one `f32` column, as an array library would hold it,
//! with no mask to keep in step. Every use of the column tests for `NaN` first, so no unvoiced
//! frame is ever binned, averaged or segmented as a pitch.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::breathy::{Breathy, breathe};
use crate::error::{Error, Result};
use crate::event::Event;
use crate::notes::{Segment, Segmenter};
use crate::performance::{FrameBlock, PerformanceHead, absolute_cents};
use crate::rejoin::{Rejoin, rejoin};
use crate::residual::{NOTE_COST, NoteResidual, TakeResidual, residual};
use crate::timing::Origin;

/// A4 at 440 Hz, in absolute cents.
pub const A440_CENTS: f64 = 6900.0;
/// A note whose held pitch is further than this, in cents, from its key on the take's tuning is
/// **between keys** (owner, 2026-11-15).
pub const BETWEEN_KEYS_CENTS: f32 = 35.0;
/// How long a frame's pitch must be steady to count toward the tuning: slides and scoops
/// would otherwise smear the histogram's peak.
const STEADY_SECS: f64 = 0.05;
/// How far, in cents, a steady stretch's smoothed pitch may move.
const STEADY_CENTS: f64 = 10.0;
/// How long a frame's pitch is smoothed over before it is binned: the mean of the voiced frames
/// over this span, so that a vibrato's swing cancels and its extremes do not make two peaks.
const SMOOTH_SECS: f64 = 0.2;
/// How far, in cents, either side of a histogram bin its window reaches: the peak is the bin
/// whose window holds the most weight, and is refined by the frames in that window. It is also
/// how far from the estimate a frame may be and count toward its confidence.
const WINDOW_CENTS: usize = 15;
/// The share of the weight a window holds when the frames are spread flat across the hundred
/// cents: its width over the hundred. A tuning's confidence is measured above it.
const FLAT_SHARE: f64 = (2 * WINDOW_CENTS + 1) as f64 / 100.0;

/// A performance's frames as columns of numbers: an unvoiced frame's pitch is `NaN`.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceFrames {
    pub rate: u32,
    pub hop: u32,
    pub first_frame: u64,
    /// Each frame's pitch in absolute cents, `NaN` when unvoiced.
    pub cents: Vec<f32>,
    pub voicing: Vec<f32>,
    pub level: Vec<f32>,
}

impl PerformanceFrames {
    /// No frames yet, for the performance `head` describes.
    #[must_use]
    pub fn new(head: &PerformanceHead) -> PerformanceFrames {
        PerformanceFrames {
            rate: head.rate,
            hop: head.hop,
            first_frame: head.first_frame,
            cents: Vec::new(),
            voicing: Vec::new(),
            level: Vec::new(),
        }
    }

    /// Appends `block`'s frames.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when its columns differ in length, or it does not start where the
    /// frames so far end.
    pub fn push(&mut self, block: &FrameBlock) -> Result<()> {
        block.check()?;
        let next = self.frame(self.cents.len());
        if block.start != next {
            return Err(Error::Invalid(format!(
                "a frame block starts at {}, where the performance's next frame is {next}",
                block.start
            )));
        }
        self.cents
            .extend(block.pitch.iter().map(|p| p.unwrap_or(f32::NAN)));
        self.voicing.extend_from_slice(&block.voicing);
        self.level.extend_from_slice(&block.level);
        Ok(())
    }

    /// The number of frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cents.len()
    }

    /// Whether there are no frames.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cents.is_empty()
    }

    /// Where frame `k` sits, on the take's timeline.
    #[must_use]
    pub fn frame(&self, k: usize) -> u64 {
        self.first_frame + k as u64 * u64::from(self.hop)
    }

    /// Frames spanning `secs`, at least one.
    fn frames_in(&self, secs: f64) -> usize {
        ((secs * f64::from(self.rate) / f64::from(self.hop)).round() as usize).max(1)
    }
}

/// Where a take's A4 sits: given, used exactly, or estimated from its frames.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Tuning {
    /// A4, in absolute cents: 6900.0 is 440 Hz. An estimate's confidence is how far the share
    /// of the steady frames within 15 cents of it stands above the share a flat spread across
    /// the hundred cents would give: 0 for a flat spread, 1 when every steady frame is that
    /// near (since capture-analysis 1.5; before, it was the share itself).
    pub a4: Origin<f64>,
    /// How widely, in cents, the steady frames spread about an estimate: their root mean square
    /// distance from it. Absent for a given tuning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread_cents: Option<f64>,
}

impl Tuning {
    /// A tuning given as A4 in absolute cents, used exactly.
    #[must_use]
    pub const fn given(a4_cents: f64) -> Tuning {
        Tuning {
            a4: Origin::Given(a4_cents),
            spread_cents: None,
        }
    }

    /// How far A4 sits from 440 Hz, in cents.
    #[must_use]
    pub fn offset_cents(&self) -> f64 {
        self.a4.value() - A440_CENTS
    }

    /// A4, in Hz.
    #[must_use]
    pub fn a4_hz(&self) -> f64 {
        440.0 * (self.offset_cents() / 1200.0).exp2()
    }
}

/// `cents` folded into `-50..50`: its distance from the nearest key.
fn fold(cents: f64) -> f64 {
    (cents + 50.0).rem_euclid(100.0) - 50.0
}

/// Each steady voiced frame's distance from the nearest key on A440, in cents (`-50..50`),
/// with its voicing as its weight.
///
/// A frame's pitch is first smoothed: the mean of the voiced frames within `SMOOTH_SECS` of
/// it, so a vibrato's swing cancels. It is steady when every frame within `STEADY_SECS` of it
/// is voiced and their smoothed pitch moves no more than `STEADY_CENTS`. An unvoiced frame
/// (`NaN`) is never one, and never enters a mean.
fn steady_distances(frames: &PerformanceFrames) -> Vec<(f64, f64)> {
    let n = frames.len();
    let smooth_half = frames.frames_in(SMOOTH_SECS) / 2;
    let steady_half = (frames.frames_in(STEADY_SECS) / 2).max(1);
    let voiced = |k: usize| !frames.cents[k].is_nan();
    let smoothed: Vec<f64> = (0..n)
        .map(|k| {
            if !voiced(k) {
                return f64::NAN;
            }
            let (sum, count) = (k.saturating_sub(smooth_half)..=(k + smooth_half).min(n - 1))
                .filter(|&j| voiced(j))
                .fold((0.0, 0.0), |(sum, count), j| {
                    (sum + f64::from(frames.cents[j]), count + 1.0)
                });
            sum / count
        })
        .collect();
    (0..n)
        .filter(|&k| k >= steady_half && k + steady_half < n)
        .filter_map(|k| {
            let window = &smoothed[k - steady_half..=k + steady_half];
            if window.iter().any(|c| c.is_nan()) {
                return None;
            }
            let (low, high) = window
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), &c| (l.min(c), h.max(c)));
            (high - low <= STEADY_CENTS)
                .then(|| (fold(smoothed[k] - A440_CENTS), f64::from(frames.voicing[k])))
        })
        .collect()
}

/// The take's tuning, estimated from its frames, after librosa's `pitch_tuning`: each steady
/// voiced frame's distance from the nearest key on A440 is binned at 1 cent over `-50..50`,
/// weighted by its voicing. The peak is the bin whose window, `WINDOW_CENTS` either side and
/// wrapping round at ±50, holds the most weight, and the weighted mean of the frames in that
/// window is how far A4 sits from 440 Hz.
///
/// - **A peak, not a mean:** on a take sung wide, a mean is pulled by the tails, where the peak
///   finds where most of the singing sits.
/// - **A window, not a single bin:** a take sung 40 cents flat whose flattest notes sit 52 flat
///   has its single heaviest bin at +48, a key away; the heaviest window is where it is sung.
/// - **Confidence** is how far the share of the weight within `WINDOW_CENTS` of the estimate
///   stands above `FLAT_SHARE`, the share a flat spread would give, scaled so that a flat
///   spread is 0 and every frame within the window is 1. A take whose held pitch wanders
///   across the hundred cents is therefore weak, although half its frames sit near the
///   estimate.
/// - **Spread** is the weighted root mean square distance of every steady frame from it.
///
/// A take with no steady voiced frame is estimated at 440 Hz with no confidence.
#[must_use]
pub fn estimate_tuning(frames: &PerformanceFrames) -> Tuning {
    let distances = steady_distances(frames);
    let total: f64 = distances.iter().map(|d| d.1).sum();
    if distances.is_empty() || total <= 0.0 {
        return Tuning {
            a4: Origin::Estimated {
                value: A440_CENTS,
                confidence: 0.0,
            },
            spread_cents: Some(0.0),
        };
    }
    let mut histogram = [0.0f64; 100];
    for &(d, w) in &distances {
        histogram[((d + 50.0).floor() as usize).min(99)] += w;
    }
    let window = |b: usize| -> f64 {
        (0..=2 * WINDOW_CENTS)
            .map(|i| histogram[(b + 100 + i - WINDOW_CENTS) % 100])
            .sum()
    };
    let peak = (0..100).fold(0, |best, b| if window(b) > window(best) { b } else { best });
    let centre = peak as f64 - 50.0 + 0.5;
    let reach = WINDOW_CENTS as f64 + 0.5;
    let within = |centre: f64| {
        distances
            .iter()
            .map(move |&(d, w)| (fold(d - centre), w))
            .filter(move |(diff, _)| diff.abs() <= reach)
    };
    let (sum, weight) = within(centre).fold((0.0, 0.0), |(s, t), (diff, w)| (s + diff * w, t + w));
    let offset = fold(centre + sum / weight);
    let near: f64 = within(offset).map(|(_, w)| w).sum();
    let spread = (distances
        .iter()
        .map(|&(d, w)| fold(d - offset).powi(2) * w)
        .sum::<f64>()
        / total)
        .sqrt();
    Tuning {
        a4: Origin::Estimated {
            value: A440_CENTS + offset,
            confidence: ((near / total - FLAT_SHARE) / (1.0 - FLAT_SHARE)).clamp(0.0, 1.0) as f32,
        },
        spread_cents: Some(spread),
    }
}

/// A note as the reading reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadNote {
    /// Where the note starts, its slide or scoop in included, as a frame on the take's timeline.
    pub start: u64,
    /// Where it ends.
    pub end: u64,
    /// Where its held pitch begins, after any slide or scoop into it.
    pub held_from: u64,
    /// The MIDI key nearest its held pitch, on the take's tuning.
    #[schemars(range(max = 127))]
    pub key: u8,
    /// How far its held pitch (the median of its held frames) sits from `key`, in cents.
    #[schemars(range(min = -50.0, max = 50.0))]
    pub centre: f32,
    /// More than 35 cents from its key ([`BETWEEN_KEYS_CENTS`]).
    pub between_keys: bool,
    /// Its frames' mean voicing.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
    /// As [`Event::Note`]'s.
    pub strength: f32,
    /// What the note leaves of the performance: centre, drift, vibrato, slide, fall and rest.
    /// Added in `capture-analysis` 1.2; absent before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual: Option<NoteResidual>,
}

impl ReadNote {
    /// The note as an [`Event::Note`], keyed on the take's tuning.
    #[must_use]
    pub fn event(&self) -> Event {
        Event::Note {
            start: self.start,
            end: self.end,
            midi: self.key,
            confidence: self.confidence,
            strength: self.strength,
        }
    }
}

/// Which reader chose a reading's notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReaderName {
    /// [`Melody`](crate::Melody)'s segmentation rules, over the performance ([`read`]): the
    /// baseline.
    #[default]
    Rules,
    /// The note HMM ([`NoteModel`](crate::NoteModel)).
    Hmm,
    /// Spotify's Basic Pitch, an outside model: it chooses where notes start and end, and the
    /// take's performance gives each its key, centre and residual. Added in `capture-analysis`
    /// 1.8. A build without the `basic-pitch` feature still reads it.
    #[serde(rename = "basic-pitch")]
    BasicPitch,
}

/// The reader that chose a reading's notes, and the cost per note it chose them with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Reader {
    pub name: ReaderName,
    /// The cost of a note, `c`, which the loss's `notes` term counts per note. For the note HMM
    /// it is also the cost, in nats, of leaving a held note: the negative log of the
    /// probability of leaving its stable state. The rules have no cost of their own, so theirs
    /// is the loss's [`NOTE_COST`].
    pub note_cost: f64,
}

impl Reader {
    /// The rules, with the loss's cost per note.
    #[must_use]
    pub const fn rules() -> Reader {
        Reader {
            name: ReaderName::Rules,
            note_cost: NOTE_COST,
        }
    }

    /// The note HMM, with `note_cost` per note.
    #[must_use]
    pub const fn hmm(note_cost: f64) -> Reader {
        Reader {
            name: ReaderName::Hmm,
            note_cost,
        }
    }
    /// Basic Pitch, with the loss's cost per note: like the rules, it has no cost of its own.
    #[must_use]
    pub const fn basic_pitch() -> Reader {
        Reader {
            name: ReaderName::BasicPitch,
            note_cost: NOTE_COST,
        }
    }
}

/// A take's notes, read on its tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Reading {
    pub tuning: Tuning,
    /// Which reader chose the notes, and its cost per note. Added in `capture-analysis` 1.3;
    /// absent before, when the rules were the only reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<Reader>,
    pub notes: Vec<ReadNote>,
    /// What the notes leave of the take as a whole, with the loss as its terms. Added in
    /// `capture-analysis` 1.2; absent before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual: Option<TakeResidual>,
    /// Each same-key split the reading's pass rejoined into one note, with the notes as its
    /// reader decoded them, in order. Added in `capture-analysis` 1.7; omitted
    /// when there is none, and absent before, when no reading was rejoined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejoins: Vec<Rejoin>,
    /// Each note the reading's breathy pass carried on over the weakly voiced frames after it,
    /// with the note as its reader decoded it, in order ([`breathe`](crate::breathe)). Added in
    /// `capture-analysis` 1.9; omitted when there is none, and absent before, when no reading
    /// read a breathy frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breathy: Vec<Breathy>,
}

impl Reading {
    /// How many notes are between keys.
    #[must_use]
    pub fn between_keys(&self) -> usize {
        self.notes.iter().filter(|n| n.between_keys).count()
    }
}

/// Reads `frames` and `onsets` (their [`Event::Onset`]s; other events are ignored) on `given`,
/// A4 in absolute cents, or on the tuning estimated from the frames.
///
/// The notes are segmented by the rules [`Melody`](crate::Melody) segments by, over the
/// performance's pitch: its refined pitch, written to 0.1 cent. `Melody` segments the 10-cent
/// pitch track, so a boundary or a median can differ from its by a frame or a few cents. Then
/// the reading's interpretive passes are run over them ([`interpret`]): [`read_as_decoded`] is
/// the reading before them.
#[must_use]
pub fn read(frames: &PerformanceFrames, onsets: &[Event], given: Option<f64>) -> Reading {
    interpret(read_as_decoded(frames, onsets, given), frames, onsets)
}

/// A reading's interpretive passes, run over any reader's notes as it decoded them, in order:
/// the breathy ornament ([`breathe`](crate::breathe)), then the same-key rejoin
/// ([`rejoin`](crate::rejoin)), which needs the two notes it joins to meet, and so sees a note
/// the first has carried over a breathy dip. Each states what it did, so the reading keeps
/// what was decoded. `onsets` are the take's, and `frames` the performance `reading` was read
/// from.
#[must_use]
pub fn interpret(reading: Reading, frames: &PerformanceFrames, onsets: &[Event]) -> Reading {
    rejoin(breathe(reading, frames, onsets), frames, onsets)
}

/// [`read`], before its interpretive passes: the notes as the rules segment them.
#[must_use]
pub fn read_as_decoded(
    frames: &PerformanceFrames,
    onsets: &[Event],
    given: Option<f64>,
) -> Reading {
    let tuning = tuning_of(frames, given);
    let offset = tuning.offset_cents();
    let mut segmenter = Segmenter::new(frames.rate, u64::from(frames.hop));
    for event in onsets {
        if let Event::Onset { frame, strength } = *event {
            segmenter.onset(frame, strength);
        }
    }
    let mut segments = Vec::new();
    for k in 0..frames.len() {
        let cents = f64::from(frames.cents[k]);
        let pitch = (!cents.is_nan()).then(|| ((cents - offset) / 100.0) as f32);
        segmenter.take_frame(
            frames.frame(k),
            pitch,
            frames.voicing[k],
            frames.level[k],
            &mut segments,
        );
    }
    segmenter.finish_segments(&mut segments);
    let notes: Vec<ReadNote> = segments.iter().map(read_note).collect();
    let (parts, take) = residual(frames, &tuning, &notes);
    Reading::of(tuning, Reader::rules(), notes, parts, take)
}

impl Reading {
    /// A reading of `notes` on `tuning` by `reader`, each note given its part of the residual.
    pub(crate) fn of(
        tuning: Tuning,
        reader: Reader,
        mut notes: Vec<ReadNote>,
        parts: Vec<NoteResidual>,
        take: TakeResidual,
    ) -> Reading {
        for (note, part) in notes.iter_mut().zip(parts) {
            note.residual = Some(part);
        }
        Reading {
            tuning,
            reader: Some(reader),
            notes,
            residual: Some(take),
            rejoins: Vec::new(),
            breathy: Vec::new(),
        }
    }
}

/// The tuning a reading is read on: `given`, A4 in absolute cents, or estimated from `frames`.
pub(crate) fn tuning_of(frames: &PerformanceFrames, given: Option<f64>) -> Tuning {
    given.map_or_else(|| estimate_tuning(frames), Tuning::given)
}

fn read_note(segment: &Segment) -> ReadNote {
    ReadNote::held_at(
        segment.median,
        segment.start,
        segment.end,
        segment.held_from,
    )
    .with(segment.confidence, segment.strength)
}

impl ReadNote {
    /// A note from `start` to `end`, held from `held_from` at `held`, its held pitch in
    /// fractional key numbers on the take's tuning: its key the nearest, its centre the rest.
    pub(crate) fn held_at(held: f32, start: u64, end: u64, held_from: u64) -> ReadNote {
        let key = held.round().clamp(0.0, 127.0);
        let centre = (held - key) * 100.0;
        ReadNote {
            start,
            end,
            held_from,
            key: key as u8,
            centre,
            between_keys: centre.abs() > BETWEEN_KEYS_CENTS,
            confidence: 0.0,
            strength: 0.0,
            residual: None,
        }
    }

    pub(crate) fn with(mut self, confidence: f32, strength: f32) -> ReadNote {
        self.confidence = confidence;
        self.strength = strength;
        self
    }
}

/// `hz` as A4 in absolute cents.
#[must_use]
pub fn a4_cents_of_hz(hz: f64) -> f64 {
    absolute_cents(hz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::{Layer1, Performance};
    use crate::stage::{Mono, Stage};
    use crate::timing::WEAK_CONFIDENCE;
    use crate::{Melody, synth};

    const RATE: u32 = 22_050;

    /// A melody in C major, one key number per note, each `secs` long.
    const TUNE: [(u8, f64); 9] = [
        (60, 0.4),
        (62, 0.3),
        (64, 0.5),
        (65, 0.3),
        (67, 0.6),
        (69, 0.3),
        (67, 0.4),
        (64, 0.4),
        (60, 0.7),
    ];

    fn hz(key: u8, cents: f64) -> f64 {
        440.0 * ((f64::from(key) - 69.0 + cents / 100.0) / 12.0).exp2()
    }

    /// `notes` (key, cents off it, secs) with a quarter second of silence either side.
    fn take(notes: &[(u8, f64, f64)]) -> Vec<f32> {
        let tones: Vec<(f64, f64)> = notes.iter().map(|&(k, c, s)| (hz(k, c), s)).collect();
        [
            synth::silence(RATE, 0.25),
            synth::note_sequence(RATE, &tones, 4, 0.3),
            synth::silence(RATE, 0.25),
        ]
        .concat()
    }

    fn tune(cents: f64) -> Vec<f32> {
        take(&TUNE.map(|(k, s)| (k, cents, s)))
    }

    /// The take's performance and its onsets, as `duet capture` keeps them.
    fn performance(signal: &[f32]) -> (PerformanceFrames, Vec<Event>) {
        let mut stage = Performance::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(4096).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: (i * 4096) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
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
        (frames.unwrap(), onsets)
    }

    fn keys(reading: &Reading) -> Vec<u8> {
        reading.notes.iter().map(|n| n.key).collect()
    }

    fn estimated_offset(reading: &Reading) -> (f64, f32) {
        match reading.tuning.a4 {
            Origin::Estimated { value, confidence } => (value - A440_CENTS, confidence),
            Origin::Given(_) => panic!("the tuning was given"),
        }
    }

    #[test]
    fn a_melody_sung_sharp_or_flat_reads_the_keys_it_reads_in_tune() {
        let (frames, onsets) = performance(&tune(0.0));
        let in_tune = read(&frames, &onsets, None);
        assert_eq!(keys(&in_tune), TUNE.map(|t| t.0));
        // Each note a little off the take's own tuning, as a singer is: up to 12 cents either
        // way. On A440, a note 44 + 12 cents sharp is read a key up, and 40 + 12 flat a key
        // down; on the take's tuning, none is.
        let scatter = [0.0, 12.0, -12.0, 12.0, 0.0, -12.0, 12.0, 0.0, -12.0];
        for cents in [44.0, -40.0] {
            let notes: Vec<(u8, f64, f64)> = TUNE
                .iter()
                .zip(scatter)
                .map(|(&(k, s), off)| (k, cents + off, s))
                .collect();
            let (frames, onsets) = performance(&take(&notes));
            let reading = read(&frames, &onsets, None);
            assert_eq!(keys(&reading), keys(&in_tune), "{cents} cents");
            let (offset, _) = estimated_offset(&reading);
            assert!((offset - cents).abs() <= 12.0, "{cents}: {offset}");
            let on_a440 = read(&frames, &onsets, Some(A440_CENTS));
            assert_ne!(
                keys(&on_a440),
                keys(&in_tune),
                "{cents}: A440 reads it right"
            );
        }
    }

    #[test]
    fn an_in_tune_take_reads_as_melody_does() {
        let signal = tune(0.0);
        let (frames, onsets) = performance(&signal);
        let reading = read(&frames, &onsets, None);
        let mut melody = Melody::new(RATE);
        let mut events = Vec::new();
        melody
            .push(
                Mono {
                    rate: RATE,
                    start_frame: 0,
                    samples: signal.clone(),
                },
                &mut events,
            )
            .unwrap();
        melody.finish(&mut events).unwrap();
        let melody_notes: Vec<(u64, u64, u8)> = events
            .iter()
            .filter_map(|e| match *e {
                Event::Note {
                    start, end, midi, ..
                } => Some((start, end, midi)),
                _ => None,
            })
            .collect();
        assert_eq!(reading.notes.len(), melody_notes.len());
        let hop = u64::from(frames.hop);
        for (note, &(start, end, midi)) in reading.notes.iter().zip(&melody_notes) {
            assert_eq!(note.key, midi);
            assert!(note.start.abs_diff(start) <= hop, "{note:?} {start}");
            assert!(note.end.abs_diff(end) <= hop, "{note:?} {end}");
            assert!(note.centre.abs() < 5.0, "{note:?}");
            assert!(note.held_from >= note.start && note.held_from < note.end);
        }
    }

    #[test]
    fn a_given_tuning_is_used_exactly_where_the_estimate_differs() {
        let (frames, onsets) = performance(&tune(0.0));
        let reading = read(&frames, &onsets, Some(A440_CENTS + 30.0));
        assert_eq!(reading.tuning, Tuning::given(6930.0));
        assert_eq!(reading.tuning.spread_cents, None);
        assert_eq!(keys(&reading), TUNE.map(|t| t.0));
        for note in &reading.notes {
            assert!((note.centre + 30.0).abs() < 5.0, "{note:?}");
        }
        assert!((Tuning::given(6930.0).a4_hz() - 447.69).abs() < 0.01);
    }

    #[test]
    fn the_tuning_is_a_peak_not_a_mean() {
        // 70 % of the singing at +20 cents and 30 % at -30: a mean would say +5.
        let notes = [
            (60, 20.0, 0.7),
            (64, 20.0, 0.7),
            (67, -30.0, 0.6),
            (65, 20.0, 0.7),
            (62, -30.0, 0.6),
            (60, 20.0, 0.7),
        ];
        let (frames, onsets) = performance(&take(&notes));
        let (offset, confidence) = estimated_offset(&read(&frames, &onsets, None));
        assert!((offset - 20.0).abs() <= 3.0, "{offset}");
        assert!((0.5..0.9).contains(&confidence), "{confidence}");
    }

    #[test]
    fn a_tuning_held_by_little_more_than_half_the_singing_is_weak() {
        // Six notes in eleven held at +32 cents, and five spread evenly across the far half of
        // the octave's cents, as a take whose held pitch wanders: 55 % of the singing sits within
        // the window of +32, where a flat spread would already put 31 %.
        let spread = (0..5).map(|i| fold(32.0 + 35.0 + 7.5 * f64::from(i)));
        let cents: Vec<f64> = [32.0; 6].into_iter().chain(spread).collect();
        let keys = [60, 62, 64, 65, 67, 69, 67, 65, 64, 62, 60];
        let notes: Vec<(u8, f64, f64)> = keys
            .iter()
            .zip(&cents)
            .map(|(&k, &c)| (k, c, 0.5))
            .collect();
        let (frames, onsets) = performance(&take(&notes));
        let (offset, confidence) = estimated_offset(&read(&frames, &onsets, None));
        assert!((offset - 32.0).abs() <= 3.0, "{offset}");
        assert!(confidence < WEAK_CONFIDENCE, "{confidence}");
        // Held by every note, it is not weak.
        let (frames, onsets) = performance(&tune(32.0));
        let (_, confidence) = estimated_offset(&read(&frames, &onsets, None));
        assert!(confidence >= WEAK_CONFIDENCE, "{confidence}");
    }

    #[test]
    fn the_tuning_is_estimated_to_the_cent_from_the_refined_pitch() {
        // The pitch track's states are 10 cents apart; the refined pitch is not.
        let (frames, onsets) = performance(&tune(13.0));
        let reading = read(&frames, &onsets, None);
        let (offset, confidence) = estimated_offset(&reading);
        assert!((offset - 13.0).abs() <= 2.0, "{offset}");
        assert!(confidence > 0.9, "{confidence}");
        assert!(reading.tuning.spread_cents.unwrap() < 5.0, "{reading:?}");
    }

    #[test]
    fn a_note_far_from_its_key_on_the_tuning_is_between_keys() {
        let mut notes: Vec<(u8, f64, f64)> = TUNE.map(|(k, s)| (k, 0.0, s)).to_vec();
        notes[4].1 = 42.0;
        let (frames, onsets) = performance(&take(&notes));
        let reading = read(&frames, &onsets, None);
        assert_eq!(reading.between_keys(), 1, "{reading:?}");
        let odd = reading.notes.iter().find(|n| n.between_keys).unwrap();
        assert_eq!(odd.key, 67);
        assert!((odd.centre - 42.0).abs() < 5.0, "{odd:?}");
    }

    #[test]
    fn no_unvoiced_frame_enters_the_tuning_histogram() {
        // Steady frames at +20 cents, and between them unvoiced frames the tracker was sure of:
        // their pitch is NaN, and a NaN binned or averaged would move or poison the estimate.
        let head = PerformanceHead::new(RATE, 256, 0);
        let mut frames = PerformanceFrames::new(&head);
        let mut pitch = Vec::new();
        for k in 0..400 {
            pitch.push((k % 100 < 60).then_some(6920.0f32));
        }
        let n = pitch.len();
        let block = FrameBlock::new(0, pitch, vec![1.0; n], vec![0.1; n]).unwrap();
        frames.push(&block).unwrap();
        let distances = steady_distances(&frames);
        assert!(!distances.is_empty());
        assert!(
            distances.iter().all(|(d, _)| (d - 20.0).abs() < 1e-3),
            "{distances:?}"
        );
        // Every one is a voiced frame, and none sits at an edge of a voiced stretch.
        let voiced = frames.cents.iter().filter(|c| !c.is_nan()).count();
        assert!(distances.len() < voiced, "{} of {voiced}", distances.len());
        let tuning = estimate_tuning(&frames);
        assert!((tuning.offset_cents() - 20.0).abs() < 1e-3, "{tuning:?}");
        // With no voiced frame at all, the estimate is 440 Hz with no confidence.
        let mut silent = PerformanceFrames::new(&head);
        silent
            .push(&FrameBlock::new(0, vec![None; 50], vec![1.0; 50], vec![0.0; 50]).unwrap())
            .unwrap();
        assert_eq!(
            estimate_tuning(&silent).a4,
            Origin::Estimated {
                value: A440_CENTS,
                confidence: 0.0
            }
        );
    }

    #[test]
    fn a_block_that_does_not_follow_is_refused() {
        let head = PerformanceHead::new(RATE, 256, 128);
        let mut frames = PerformanceFrames::new(&head);
        let block = FrameBlock::new(128, vec![None; 4], vec![0.0; 4], vec![0.0; 4]).unwrap();
        frames.push(&block).unwrap();
        let gap = FrameBlock::new(128 + 5 * 256, vec![None], vec![0.0], vec![0.0]).unwrap();
        let err = frames.push(&gap).unwrap_err();
        assert!(err.to_string().contains("next frame is 1152"), "{err}");
    }
}
