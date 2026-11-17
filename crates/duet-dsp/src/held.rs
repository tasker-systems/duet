//! The held pitch: a pass over a reading, after any reader has chosen its notes, that reads a
//! pitch held before the note it reaches as a note of its own, and states where one is likely
//! an approach to that note (ruled by the owner, 2026-11-17, on the held-pitch findings).
//!
//! **The ruling.** The capture keeps every pitch the voice holds for at least the shortest note
//! ([`SHORTEST_NOTE_SECS`], 0.05 s) before the note it reaches as a note of its own: "a held
//! pitch they sang goes in the capture". Folding an approach into the note it reaches is a
//! choice made per note through the keep-or-correct marks, never the capture's (the decision
//! "intent is recorded beside the capture, never over it").
//!
//! **What is a held pitch.** Before a note's `held_from`, and over the voiced frames its reader
//! left unread that lead straight into it, a run of at least the shortest note's frames whose
//! pitch spreads no wider than a pitch that has arrived ([`ARRIVED_SEMITONES`], 35 cents), on a
//! key other than the note's: its key is the one nearest its frames' median. Under the note HMM
//! that is a grace excursion before `held_from`, or a scoop that holds; under Basic Pitch, which
//! starts its notes where they are held, it is frames left unread before the note. The rules
//! already read such a pitch as a note, so the pass seldom finds one in theirs.
//! - **Below the note,** by at least [`EXCURSION_CENTS`] from where the note is held, as a grace
//!   is. Every held pitch the owner ruled a note on "Wayfaring Stranger" sits below the note it
//!   reaches. A pitch held above it is an overshoot (2.28 and 14.87 s), a lean (19.27 s) or a
//!   turn at the note's start (Blood Sport's at 33.3 s, task A's witness, an excursion by the
//!   question 1 ruling), and stays as its reader filed it. An approach from above is the
//!   owner's to rule on.
//! - **Only before the voice is on the note.** Once a run within [`EXCURSION_CENTS`] of where
//!   the note is held, or on its key, holds, what follows before `held_from` is the note's, so
//!   an ornament inside a held note is never made a note: the rise to G3 in the second F#3 at
//!   16.39 s of "Wayfaring Stranger" follows the F#3 itself, and stays its excursion.
//! - **The lead-in** is the run of frames before the note that have a pitch and are voiced at
//!   least as a note must be ([`LEAST_CONFIDENCE`]), back to the note before and at most
//!   [`HELD_PITCH_SECS`]. A pitch already held where it reaches back to is a note its reader
//!   chose not to read (Basic Pitch's minimum note length, say), and stays unread.
//! - **Where the made note ends:** at the first frame after the held pitch nearer where the
//!   voice goes next (the next held pitch, or where the note is held) than where it was held,
//!   since a slide belongs to the note it arrives at (#28), as the breathy pass ends a note.
//!   The note it reaches then starts there; its `held_from`, key and end are as its reader
//!   decoded them.
//!
//! **The suggestion.** Over the reading's notes after the pass, each that is held before the
//! note it reaches, as the held-pitch findings chose them (at most [`HELD_PITCH_SECS`] long, the
//! next note on another key, longer, and starting within [`NEXT_NOTE_GAP_SECS`] of its end), is
//! stated a **likely approach to the next note** only when both hold:
//! - the step to the next note is a semitone;
//! - the held pitch drifts toward it by more than [`APPROACH_DRIFT_CENTS`] (+30 cents) in its
//!   last quarter.
//!
//! On vocadito that combination is a note only 6% of the time (n = 32); on "Wayfaring Stranger"
//! it fires at 12.03 s only. Nothing else is suggested: an onset points the wrong way on the
//! owner's legato singing. The drift is measured over the held pitch from the note's start to
//! where the voice **reaches** the next note: its first frame [`CHANGE_SEMITONES`] (0.7) from
//! where the note is held toward the next, within [`HELD_PITCH_SECS`] of its start, which is
//! where the rules, the reader the findings measured, end a held pitch; or, when it never moves
//! that far, where the note ends, as the findings measured it. So it reads the same
//! whoever ended the note: this pass ends a held pitch at its rise's midpoint, and the note HMM
//! holds the next note from earlier.
//!
//! **It runs last, after the breathy pass and the same-key rejoin**
//! ([`interpret`](crate::interpret)).
//! - **After the rejoin,** because the rejoin joins two notes on one key that meet with no onset
//!   between them: run after this pass, it would fold a pitch held on the key of the note before
//!   back into that note, and the ruling keeps every held pitch a note. Run after it, the pass
//!   sees each dip the rejoin settled as held frames of one note, which it never touches, so
//!   9.54 s stays one D#3. A made note is never rejoined to a note before it on its key. On
//!   "Wayfaring Stranger" the order changes no witness.
//! - **After the breathy pass,** because a note's weakly voiced tail is then already its own,
//!   so the lead-in into the next note starts where that tail ends, and no frame is read twice.
//!
//! **Each note made is stated** in the reading's `held`, with the note it reaches as its reader
//! decoded it, so the capture keeps what was read and a later choice can undo the pass. Each
//! suggestion is stated in the reading's `suggestions`, with the step, the drift and the rule
//! that fired; it changes no note.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::notes::{
    ARRIVED_SEMITONES, CHANGE_SEMITONES, LEAST_CONFIDENCE, SHORTEST_NOTE_SECS, median,
};
use crate::reading::{PerformanceFrames, ReadNote, Reading};
use crate::rejoin::{bare, indices, key_cents};
use crate::residual::{EXCURSION_CENTS, NOTE_COST, residual_with_cost};

/// The longest a note held before the note it reaches may be and be suggested an approach, in
/// seconds: the held-pitch findings measured their signals on notes no longer than this.
pub const HELD_PITCH_SECS: f64 = 0.25;
/// How long after a held note ends the note it reaches may start, in seconds, as the held-pitch
/// findings chose the pairs they measured.
pub const NEXT_NOTE_GAP_SECS: f64 = 0.05;
/// How far toward the next note, in cents, a held pitch's last quarter must sit on average,
/// with a semitone step to it, to be suggested a likely approach (owner, 2026-11-17): +30.
/// Approaches on vocadito end their held pitch a median +43 cents toward the next note, and
/// notes +15.
pub const APPROACH_DRIFT_CENTS: f64 = 30.0;

/// The notes the pass made from the pitches held before one note, and that note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Held {
    /// The note the held pitches reach, as its reader decoded it, its residual left out. A made
    /// note that ends after its `start` was taken from it, before its `held_from`; one that ends
    /// at or before its `start` was read from frames its reader left unread before it.
    pub decoded: ReadNote,
    /// The notes made, in order, as the pass made them, their residuals left out: the note
    /// reached now starts where the last of them ends, or where it did, whichever is later.
    pub notes: Vec<ReadNote>,
    /// What making them changes the loss's total by, in nats: the made notes and the note
    /// reached, less the note as decoded. A note's cost more for each, and its held frames
    /// priced as held rather than as slide or unread.
    pub loss_change: f64,
}

/// What a suggestion suggests about a note. Only one is stated yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Suggests {
    /// The note is likely an approach to the next note: a choice may fold it into that note.
    LikelyApproach,
}

/// The rule that made a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SuggestionRule {
    /// A semitone step to the next note, with the held pitch drifting toward it by more than
    /// [`APPROACH_DRIFT_CENTS`] in its last quarter.
    SemitoneWithDrift,
}

/// A suggestion about one note of the reading. It changes nothing: it is evidence a person or
/// an agent may act on, per note.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Suggestion {
    /// The note's start, on the take's timeline: it names the note.
    pub start: u64,
    /// The note's key.
    #[schemars(range(max = 127))]
    pub key: u8,
    pub suggests: Suggests,
    /// The rule that fired.
    pub rule: SuggestionRule,
    /// The next note's key.
    #[schemars(range(max = 127))]
    pub next_key: u8,
    /// The step from the note's key to the next note's, in semitones: above zero upward.
    pub step: i32,
    /// Where the held pitch's last quarter sits on average, in cents from the note's key,
    /// above zero toward the next note.
    pub drift_cents: f32,
}

/// `reading` with every pitch held before the note it reaches made a note of its own, each
/// stated in its `held`, every note's residual and the take's measured again, and its
/// `suggestions` stated. `frames` is the performance `reading` was read from.
#[must_use]
pub fn hold_apart(reading: Reading, frames: &PerformanceFrames) -> Reading {
    let note_cost = reading.reader.map_or(NOTE_COST, |r| r.note_cost);
    let offset = reading.tuning.offset_cents();
    let mut notes: Vec<ReadNote> = Vec::with_capacity(reading.notes.len());
    // For each decoded note held pitches reach: its index, the made notes, and where in `notes`
    // the first of them is.
    let mut made: Vec<(usize, Vec<ReadNote>, usize)> = Vec::new();
    for (i, note) in reading.notes.iter().enumerate() {
        let previous = notes.last().cloned();
        match split(frames, offset, previous.as_ref(), note) {
            Some((held, rest)) => {
                made.push((i, held.clone(), notes.len()));
                notes.extend(held);
                notes.push(rest);
            }
            None => notes.push(note.clone()),
        }
    }
    let mut out = if made.is_empty() {
        reading
    } else {
        let total = |notes: &[ReadNote]| {
            residual_with_cost(frames, &reading.tuning, notes, note_cost)
                .1
                .loss
                .total()
        };
        let held = made
            .into_iter()
            .map(|(i, held, first)| {
                let decoded = &reading.notes[i];
                Held {
                    loss_change: total(&notes[first..=first + held.len()])
                        - total(std::slice::from_ref(decoded)),
                    decoded: bare(decoded),
                    notes: held.iter().map(bare).collect(),
                }
            })
            .collect();
        let (mut parts, take) = residual_with_cost(frames, &reading.tuning, &notes, note_cost);
        // Everything from a note's `held_from` on is as it was, so its residual's excursions
        // there are kept as stated: a rejoined dip's is the rejoin's, which the residual alone
        // does not find.
        for (note, part) in notes.iter().zip(&mut parts) {
            let Some(was) = reading
                .notes
                .iter()
                .find(|n| n.end == note.end && n.held_from == note.held_from)
                .and_then(|n| n.residual.as_ref())
            else {
                continue;
            };
            part.excursions.retain(|e| e.start < note.held_from);
            part.excursions.extend(
                was.excursions
                    .iter()
                    .filter(|e| e.start >= note.held_from)
                    .copied(),
            );
        }
        let reader = reading.reader.unwrap_or_else(crate::Reader::rules);
        let mut out = Reading::of(reading.tuning, reader, notes, parts, take);
        out.breathy = reading.breathy;
        out.rejoins = reading.rejoins;
        out.held = held;
        out
    };
    out.suggestions = suggest(frames, offset, &out.notes);
    out
}

/// A frame's pitch as a fractional key number on the take's tuning, `offset` cents from A440:
/// `NaN` when unvoiced.
fn key_of(frames: &PerformanceFrames, offset: f64, k: usize) -> f64 {
    (f64::from(frames.cents[k]) - offset - crate::A440_CENTS) / 100.0 + 69.0
}

/// A run of frames, `from..to` by index, held on `key`, its frames' median at `median`.
struct Stretch {
    from: usize,
    to: usize,
    key: u8,
    median: f64,
}

/// The notes made from the pitches held before `note`, and `note` as it is left: `None` when
/// none is held. `previous` is the note before it; `offset` is the tuning's, in cents from A440.
fn split(
    frames: &PerformanceFrames,
    offset: f64,
    previous: Option<&ReadNote>,
    note: &ReadNote,
) -> Option<(Vec<ReadNote>, ReadNote)> {
    let hop = u64::from(frames.hop);
    let shortest = (SHORTEST_NOTE_SECS * f64::from(frames.rate) / hop as f64).ceil() as usize;
    let arrived = f64::from(ARRIVED_SEMITONES);
    let after = previous.map_or(0, |p| p.end);
    let inside = indices(frames, note.start, note.held_from);
    // Where the note is held, as a fractional key.
    let held_at = f64::from(note.key) + f64::from(note.centre) / 100.0;
    // The lead-in: the frames before the note, back to the note before, with a pitch and voiced
    // as a note must be, that run straight into it, and at most a held pitch's length.
    let mut lead = inside.start;
    let longest = (HELD_PITCH_SECS * f64::from(frames.rate) / hop as f64) as usize;
    let bound = inside.start.saturating_sub(longest);
    let since_previous = indices(frames, after, note.start).start;
    let earliest = since_previous.max(bound);
    while lead > earliest
        && !frames.cents[lead - 1].is_nan()
        && frames.voicing[lead - 1] >= LEAST_CONFIDENCE
    {
        lead -= 1;
    }
    let pitch = |k: usize| key_of(frames, offset, k);
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut k = lead;
    while k < inside.end {
        if frames.cents[k].is_nan() {
            k += 1;
            continue;
        }
        let (mut low, mut high) = (pitch(k), pitch(k));
        let mut to = k + 1;
        while to < inside.end && !frames.cents[to].is_nan() {
            let p = pitch(to);
            if high.max(p) - low.min(p) > arrived {
                break;
            }
            (low, high) = (low.min(p), high.max(p));
            to += 1;
        }
        if to - k < shortest {
            k += 1;
            continue;
        }
        // Cut short by its length, not by the note before, with the voice held on before it.
        let cut_short = lead == bound
            && bound > since_previous
            && !frames.cents[lead - 1].is_nan()
            && frames.voicing[lead - 1] >= LEAST_CONFIDENCE;
        if k == lead && lead < inside.start && cut_short {
            // Held already where the lead-in can reach back to: a note its reader chose not to
            // read, not a pitch held before this one.
            k = to;
            continue;
        }
        let held: Vec<f32> = (k..to).map(|j| pitch(j) as f32).collect();
        let centre = f64::from(median(&held));
        let key = centre.round().clamp(0.0, 127.0) as u8;
        let below = (held_at - centre) * 100.0;
        if key == note.key || (0.0..EXCURSION_CENTS).contains(&below) {
            // The voice is on the note: what follows is the note's own.
            break;
        }
        if below < 0.0 {
            // Held above the note: an overshoot, a turn or a lean, which stays as its reader
            // filed it.
            k = to;
            continue;
        }
        match stretches.last_mut() {
            Some(last) if last.key == key => {
                let held: Vec<f32> = (last.from..to).map(|j| pitch(j) as f32).collect();
                last.median = f64::from(median(&held));
                last.to = to;
            }
            _ => stretches.push(Stretch {
                from: k,
                to,
                key,
                median: centre,
            }),
        }
        k = to;
    }
    if stretches.is_empty() {
        return None;
    }
    let mut held = Vec::with_capacity(stretches.len());
    let mut start = if stretches[0].from < inside.start {
        frames
            .frame(stretches[0].from)
            .saturating_sub(hop / 2)
            .max(after)
    } else {
        note.start
    };
    for (i, s) in stretches.iter().enumerate() {
        let (next, limit) = stretches
            .get(i + 1)
            .map_or((held_at, inside.end), |n| (n.median, n.from));
        // The first frame nearer where the voice goes next than where it was held.
        let leaves = (s.to..limit)
            .find(|&j| {
                let p = pitch(j);
                !p.is_nan() && (p - next).abs() < (p - s.median).abs()
            })
            .unwrap_or(limit);
        let end = if leaves < inside.end {
            frames.frame(leaves).saturating_sub(hop / 2)
        } else {
            note.held_from
        };
        let end = end.max(frames.frame(s.to - 1) + hop / 2);
        let held_from = frames.frame(s.from).saturating_sub(hop / 2).max(start);
        let span = indices(frames, start, end);
        let count = span.len().max(1) as f32;
        let confidence = span.map(|j| frames.voicing[j]).sum::<f32>() / count;
        held.push(
            ReadNote::held_at(s.median as f32, start, end, held_from)
                .with(confidence, note.strength),
        );
        start = end;
    }
    let mut rest = note.clone();
    rest.start = rest.start.max(start);
    rest.residual = None;
    Some((held, rest))
}

/// Each suggestion the module's rule makes about `notes`, in order.
fn suggest(frames: &PerformanceFrames, offset: f64, notes: &[ReadNote]) -> Vec<Suggestion> {
    let rate = f64::from(frames.rate);
    let secs = |samples: u64| samples as f64 / rate;
    let mut out = Vec::new();
    for pair in notes.windows(2) {
        let (note, next) = (&pair[0], &pair[1]);
        let long = |n: &ReadNote| n.end.saturating_sub(n.start);
        if next.key == note.key
            || secs(long(note)) > HELD_PITCH_SECS
            || long(next) <= long(note)
            || next.start < note.start
            || secs(next.start.saturating_sub(note.end)) > NEXT_NOTE_GAP_SECS
        {
            continue;
        }
        let step = i32::from(next.key) - i32::from(note.key);
        if step.abs() != 1 {
            continue;
        }
        let Some(drift) = drift(frames, offset, note, next) else {
            continue;
        };
        let toward = drift * f64::from(step.signum());
        if toward > APPROACH_DRIFT_CENTS {
            out.push(Suggestion {
                start: note.start,
                key: note.key,
                suggests: Suggests::LikelyApproach,
                rule: SuggestionRule::SemitoneWithDrift,
                next_key: next.key,
                step,
                drift_cents: toward as f32,
            });
        }
    }
    out
}

/// Where `note`'s held pitch sits over its last quarter, on average, in cents from its key: the
/// held pitch running from the note's start to where the voice reaches `next`, the first frame
/// from the note's `held_from` at least [`CHANGE_SEMITONES`] from where the note is held toward
/// `next`, where the rules end a held pitch and start the next, searched for within
/// [`HELD_PITCH_SECS`] of the note's start, as long as a held pitch lasts. A pitch held near
/// enough the next note that the voice never moves that far from it ends where the note ends,
/// as the findings measured it. `None` when no frame of the last quarter is voiced.
fn drift(frames: &PerformanceFrames, offset: f64, note: &ReadNote, next: &ReadNote) -> Option<f64> {
    let toward = if next.key > note.key { 1.0 } else { -1.0 };
    let held = key_cents(offset, note.key) + f64::from(note.centre);
    let change = f64::from(CHANGE_SEMITONES) * 100.0;
    let longest = note.start + (HELD_PITCH_SECS * f64::from(frames.rate)) as u64;
    let reaches = indices(frames, note.held_from, next.end.min(longest))
        .find(|&k| {
            let c = f64::from(frames.cents[k]);
            !c.is_nan() && (c - held) * toward >= change
        })
        .map_or(note.end, |k| frames.frame(k));
    let quarter = reaches.saturating_sub(note.start) / 4;
    let key = key_cents(offset, note.key);
    let last: Vec<f64> = indices(frames, reaches - quarter, reaches)
        .map(|k| f64::from(frames.cents[k]))
        .filter(|c| !c.is_nan())
        .map(|c| c - key)
        .collect();
    (!last.is_empty()).then(|| last.iter().sum::<f64>() / last.len() as f64)
}

#[cfg(test)]
mod tests;
