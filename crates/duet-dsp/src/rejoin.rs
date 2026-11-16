//! The same-key rejoin: a pass over a reading, after either reader has chosen its notes, that
//! rejoins one note its reader split around an ornamental dip (the 2026-11-16 arrivals and
//! ornaments design, question 2, ruled by the owner).
//!
//! Two adjacent notes on one key are rejoined into one when:
//! - **no onset** falls between the first note's last held frame and the second's `held_from`;
//! - **every frame between them is voiced:** the two notes meet, so no frame between them was
//!   left unread as silence, and the pitch track decoded none of the frames from the first's end
//!   to the second's `held_from` unvoiced;
//! - **the dip stays within a whole tone** ([`REJOIN_CENTS`]) of their key: no frame between the
//!   first's end and the second's `held_from` sits further from it.
//!
//! A split at a consonant or an onset stays two notes, and so does a dip deeper than a whole
//! tone: a voiced consonant between two syllables, as the #28 ruling's witnesses model it, falls
//! further. This narrows #28's "a dip is two notes" to a dip an onset or a consonant marks. Notes
//! on two keys are never joined, so a real step stays two notes.
//!
//! The rejoined note starts where the first did and is held from where the first was, so the
//! dip's frames are held frames: the residual measures them as any held frame, the loss prices
//! them by their pitch, and the dip is an excursion of the note ([`Excursion`]): the residual's
//! own where the dip holds somewhere, and else the run of frames about its bottom, which the
//! pass states. The reader decoded the dip as the second note's slide or
//! attack, which costs a flat [`ATTACK_COST`](crate::ATTACK_COST) a frame, so a rejoin can
//! raise the loss: that is stated, with each rejoin, and never hidden.
//!
//! **Each rejoin is stated** in the reading's `rejoins`, with the notes as the reader decoded
//! them, so the capture keeps what was read and a later choice can undo the pass (the decision
//! "intent is recorded beside the capture, never over it").

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::notes::median;
use crate::reading::{PerformanceFrames, ReadNote, Reading};
use crate::residual::{
    EXCURSION_CENTS, Excursion, NOTE_COST, NoteResidual, SHORTEST_EXCURSION_FRAMES,
    residual_with_cost,
};

/// How far, in cents, from its key a dip between two notes on that key may go and the two still
/// be rejoined: a whole tone, as a neighbour ornament moves (owner, 2026-11-16, option (a)). A
/// dip that goes further is two notes, as the #28 ruling reads a voiced consonant.
pub const REJOIN_CENTS: f64 = 200.0;
/// How far past [`REJOIN_CENTS`] a dip's furthest frame may sit and the dip still be within a
/// whole tone: the tracker's jitter. A turn sung to the whole tone and held there reads up to a
/// few tenths of a cent past it, and task A holds a turn's excursion at +200 ± 10 cents.
pub const REJOIN_TOLERANCE_CENTS: f64 = 10.0;

/// One rejoin the pass made: the notes the reader decoded on one key, which it read as one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Rejoin {
    /// The notes as the reader decoded them, in order, their residuals left out: the rejoined
    /// note starts where the first does and ends where the last does.
    pub decoded: Vec<ReadNote>,
    /// Each dip between them, in order: one fewer than the notes.
    pub dips: Vec<Dip>,
    /// What the rejoin changes the loss's total by, in nats: the rejoined note's loss less the
    /// decoded notes'. One note's cost fewer, and the dip's frames priced as held rather than
    /// as the second note's slide, so it can be above zero: a rejoin that raises the loss.
    pub loss_change: f64,
}

/// A dip between two decoded notes on one key: the frames from where the first ends to where
/// the second's held pitch begins.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Dip {
    /// Where it starts, on the take's timeline: the first note's end.
    pub start: u64,
    /// Where it ends: the second note's `held_from`.
    pub end: u64,
    /// Its frame furthest from the key, in cents from it: below zero for a dip, above for a
    /// rise.
    pub cents: f32,
}

/// `reading` with every same-key split the module's rules allow rejoined, each rejoin stated in
/// its `rejoins`, and every note's residual and the take's measured again. `onsets` are the
/// take's (other events are ignored), and `frames` the performance `reading` was read from.
#[must_use]
pub fn rejoin(reading: Reading, frames: &PerformanceFrames, onsets: &[Event]) -> Reading {
    let note_cost = reading.reader.map_or(NOTE_COST, |r| r.note_cost);
    let onsets: Vec<u64> = onsets
        .iter()
        .filter_map(|e| match *e {
            Event::Onset { frame, .. } => Some(frame),
            _ => None,
        })
        .collect();
    let offset = reading.tuning.offset_cents();
    let mut notes: Vec<ReadNote> = Vec::with_capacity(reading.notes.len());
    // For each note, the decoded notes it rejoins and their dips; empty when it is as decoded.
    let mut groups: Vec<(Vec<ReadNote>, Vec<Dip>)> = Vec::with_capacity(reading.notes.len());
    for note in reading.notes.iter().cloned() {
        let joined = notes.last().and_then(|first| {
            let dip = dip(frames, &onsets, offset, first, &note)?;
            Some((joined(frames, offset, first, &note)?, dip))
        });
        match joined {
            Some((one, dip)) => {
                let (decoded, dips) = groups.last_mut().expect("a group per note");
                if decoded.is_empty() {
                    decoded.push(bare(notes.last().expect("the first note")));
                }
                decoded.push(bare(&note));
                dips.push(dip);
                *notes.last_mut().expect("the first note") = one;
            }
            None => {
                notes.push(note);
                groups.push((Vec::new(), Vec::new()));
            }
        }
    }
    if groups.iter().all(|(decoded, _)| decoded.is_empty()) {
        return reading;
    }
    let total = |notes: &[ReadNote]| {
        residual_with_cost(frames, &reading.tuning, notes, note_cost)
            .1
            .loss
            .total()
    };
    let (mut parts, take) = residual_with_cost(frames, &reading.tuning, &notes, note_cost);
    let mut rejoins = Vec::new();
    for ((note, part), (decoded, dips)) in notes.iter().zip(&mut parts).zip(groups) {
        if decoded.is_empty() {
            continue;
        }
        for dip in &dips {
            if let Some(excursion) = dip_excursion(frames, offset, note, part, dip) {
                part.excursions.push(excursion);
            }
        }
        part.excursions.sort_by_key(|e| e.start);
        rejoins.push(Rejoin {
            loss_change: total(std::slice::from_ref(note)) - total(&decoded),
            decoded,
            dips,
        });
    }
    let reader = reading.reader.unwrap_or_else(crate::Reader::rules);
    let mut out = Reading::of(reading.tuning, reader, notes, parts, take);
    out.rejoins = rejoins;
    out
}

/// The excursion a rejoined `dip` is in `note`, whose residual is `part`, when the residual's
/// own excursions hold none at its furthest frame: a dip that turns as it reaches its bottom, as
/// a V does, never holds, so the residual finds no excursion in it, and the rejoin states it.
/// It is the run of voiced frames about that furthest frame, inside the dip, each at least
/// [`EXCURSION_CENTS`] from the note's centre on the dip's side, kept when it holds at least
/// [`SHORTEST_EXCURSION_FRAMES`]; its `cents` the run's median residual, as an excursion's are.
fn dip_excursion(
    frames: &PerformanceFrames,
    offset: f64,
    note: &ReadNote,
    part: &NoteResidual,
    dip: &Dip,
) -> Option<Excursion> {
    let hop = u64::from(frames.hop);
    let key = key_cents(offset, note.key);
    let span = indices(frames, dip.start, dip.end);
    let residual = |k: usize| f64::from(frames.cents[k]) - key;
    let bottom = span
        .clone()
        .filter(|&k| !frames.cents[k].is_nan())
        .max_by(|&a, &b| residual(a).abs().total_cmp(&residual(b).abs()))?;
    let at = frames.frame(bottom);
    if part
        .excursions
        .iter()
        .any(|e| (e.start..e.end).contains(&at))
    {
        return None;
    }
    let side = residual(bottom).signum();
    let centre = f64::from(part.centre);
    let away =
        |k: usize| !frames.cents[k].is_nan() && (residual(k) - centre) * side >= EXCURSION_CENTS;
    if !away(bottom) {
        return None;
    }
    let mut first = bottom;
    while first > span.start && away(first - 1) {
        first -= 1;
    }
    let mut last = bottom;
    while last + 1 < span.end && away(last + 1) {
        last += 1;
    }
    if last + 1 - first < SHORTEST_EXCURSION_FRAMES {
        return None;
    }
    let cents: Vec<f32> = (first..=last).map(|k| residual(k) as f32).collect();
    Some(Excursion {
        start: frames.frame(first),
        end: (frames.frame(last) + hop).min(note.end),
        cents: median(&cents),
    })
}

/// Where `key` sits on the take's tuning, `offset` cents from A440, in absolute cents.
fn key_cents(offset: f64, key: u8) -> f64 {
    crate::A440_CENTS + offset + 100.0 * (f64::from(key) - 69.0)
}

/// `note` with its residual left out, as a rejoin states a decoded note.
fn bare(note: &ReadNote) -> ReadNote {
    let mut out = note.clone();
    out.residual = None;
    out
}

/// The frames' indices whose place on the take's timeline is in `from..to`.
fn indices(frames: &PerformanceFrames, from: u64, to: u64) -> std::ops::Range<usize> {
    let index = |at: u64| -> usize {
        let hop = u64::from(frames.hop);
        (at.saturating_sub(frames.first_frame).div_ceil(hop) as usize).min(frames.len())
    };
    index(from)..index(to)
}

/// The dip between `first` and `second`, when the two may be rejoined across it: one key, no
/// onset from the first's last held frame to the second's `held_from`, the two meeting, every
/// frame between voiced, and none further than [`REJOIN_CENTS`] (and the tracker's jitter) from
/// the key. `offset` is the
/// tuning's, in cents from A440.
fn dip(
    frames: &PerformanceFrames,
    onsets: &[u64],
    offset: f64,
    first: &ReadNote,
    second: &ReadNote,
) -> Option<Dip> {
    let hop = u64::from(frames.hop);
    if first.key != second.key || first.end < second.start {
        return None;
    }
    // The first note's last held frame is the last whose place is before its end. A note held
    // from its first frame is held from its start, and an onset in that frame is its own: the
    // readers give a note an onset up to its first frame's place, half a hop past its start.
    let last_held = first.end.saturating_sub(hop);
    let until = second.held_from.max(second.start + hop);
    if onsets.iter().any(|&f| f >= last_held && f <= until) {
        return None;
    }
    let key = key_cents(offset, first.key);
    let mut furthest: Option<f64> = None;
    for k in indices(frames, first.end, second.held_from) {
        let cents = f64::from(frames.cents[k]);
        if cents.is_nan() {
            return None;
        }
        let from = cents - key;
        if furthest.is_none_or(|f| from.abs() > f.abs()) {
            furthest = Some(from);
        }
    }
    let cents = furthest.unwrap_or(0.0);
    (cents.abs() <= REJOIN_CENTS + REJOIN_TOLERANCE_CENTS).then_some(Dip {
        start: first.end,
        end: second.held_from,
        cents: cents as f32,
    })
}

/// `first` and `second` as one note: from where the first starts, held from where it is, to
/// where the second ends, its held pitch the median of every voiced frame from there, the dip's
/// included; its confidence the two notes' by their frames, and its strength the first's.
/// `None` when that median would put it on another key: the pass never moves a note's key.
fn joined(
    frames: &PerformanceFrames,
    offset: f64,
    first: &ReadNote,
    second: &ReadNote,
) -> Option<ReadNote> {
    let held: Vec<f32> = indices(frames, first.held_from, second.end)
        .map(|k| f64::from(frames.cents[k]))
        .filter(|c| !c.is_nan())
        .map(|c| ((c - offset - crate::A440_CENTS) / 100.0 + 69.0) as f32)
        .collect();
    if held.is_empty() {
        return None;
    }
    let one = ReadNote::held_at(median(&held), first.start, second.end, first.held_from);
    if one.key != first.key {
        return None;
    }
    let length = |n: &ReadNote| (n.end - n.start) as f32;
    let (a, b) = (length(first), length(second));
    let confidence = (first.confidence * a + second.confidence * b) / (a + b).max(1.0);
    Some(one.with(confidence, first.strength))
}

#[cfg(test)]
mod tests;
