//! The breathy ornament: a pass over a reading, after any reader has chosen its notes, that
//! reads a weakly voiced run at a note's end as part of that note, where its reader left it
//! unread (the 2026-11-16 arrivals and ornaments design, question 2's breath question, ruled by
//! the owner; task F).
//!
//! #48 prices a frame left unread by its voicing ([`unread_cost`](crate::unread_cost)), so a
//! frame voiced at 0.05 is free to leave out, and a reader ends a note where its voicing drops,
//! however clearly the pitch goes on. A breathy take (a laptop mic, a voice memo) drops its
//! voicing through ornaments and falls the singer hears as the note's. The owner's ruling: a
//! weakly voiced frame **belongs to the note it sits in** when all three hold:
//! 1. **pitch continuity:** its pitch continues the note's line: it has a pitch, and moves no
//!    more than [`BREATHY_STEP_CENTS`] from the frame before it, the note's last frame being the
//!    first such;
//! 2. **level:** its level is within [`BREATHY_LEVEL_DB`] of the note's held level, the median
//!    level of the note's held frames;
//! 3. **no onset** follows it before the next note.
//!
//! Otherwise it is a breath, and stays unread, priced as #48 prices it.
//!
//! **Pitch continuity weighs above level.** The owner's takes are breathy throughout, so level
//! is the weaker evidence. Continuity is held tight, to what a sung line does in a frame; the
//! level's range is wide, so that only a frame near the noise floor fails on level alone.
//!
//! A breath, as #48's witnesses model it (voicing about 0.02, its pitch no line, its level
//! near the floor), fails the first two, so it stays unread. The pass never leaves a frame out
//! that its reader read, so #48's other guarantee holds as well: a sung frame is never cheaper
//! to leave unread than to read.
//!
//! The pass carries a note on, frame by frame, over the unread run after it, up to the next
//! note's start; it stops at the first frame that fails, and at the first frame no nearer its
//! own key than the next note's, since a slide belongs to the note it arrives at (#28). It moves no
//! note's start, key or `held_from`, adds no note and drops none.
//!
//! The kept frames are held frames of the note: the residual measures them as any held frame,
//! so a breathy fall is the note's `fall`, and the loss prices them by their pitch and their
//! voicing. Reading a frame voiced at 0.05 costs its voicing, which leaving it out did not, so
//! the pass raises the loss: that is stated, with each note it carries on, and never hidden.
//!
//! **It runs before the same-key rejoin** ([`rejoin`](crate::rejoin)), which needs the two notes
//! it joins to meet. A note its reader ended at a breathy dip, and began again on the same key
//! after it, meets its second half only once the pass has carried it over the dip; the rejoin
//! then reads one note, as the owner hears Blood Sport's "-eak". Run after, the rejoin would
//! see a gap and leave two notes, and the pass would carry the first only to the second's start.
//!
//! **Each note carried on is stated** in the reading's `breathy`, with the note as its reader
//! decoded it, so the capture keeps what was read and a later choice can undo the pass (the
//! decision "intent is recorded beside the capture, never over it").

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::notes::median;
use crate::reading::{PerformanceFrames, ReadNote, Reading};
use crate::rejoin::{bare, indices, key_cents};
use crate::residual::{NOTE_COST, residual_with_cost};

/// How far, in cents, a weakly voiced frame's pitch may move from the frame before it and still
/// continue the note's line. A sung line moves at most about 22 cents a frame (86 frames a
/// second at 22 050 Hz): a vibrato's swing of ±50 cents at 6 Hz, or a fall of 20 semitones a
/// second, as fast as [`ATTACK_COST`](crate::ATTACK_COST)'s glide. A breath's pitch, where the
/// tracker gives one, jumps further: #48's witness jumps 40 to 75 cents a frame.
pub const BREATHY_STEP_CENTS: f64 = 30.0;

/// How far below the note's held level, in decibels, a weakly voiced frame may be and still be
/// the note's: −20 dB, a tenth of its root mean square. A breathy fall at half the note's level
/// (−6 dB, the synthetic witness) is well inside it, with room for a voice memo's fall decaying
/// further; a laptop mic's noise floor sits 30 dB and more below a sung note, so a frame at the
/// floor is outside it whatever its pitch. Wide, since pitch continuity weighs above level.
pub const BREATHY_LEVEL_DB: f64 = -20.0;

/// One note the pass carried on over the weakly voiced frames after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Breathy {
    /// The note as its reader decoded it, its residual left out: the frames from its `end` to
    /// the `end` below are the ones the pass kept.
    pub decoded: ReadNote,
    /// Where the note ends now, on the take's timeline.
    pub end: u64,
    /// The quietest kept frame's level against the note's held level, in decibels: at least
    /// [`BREATHY_LEVEL_DB`].
    pub lowest_db: f32,
    /// What carrying the note on changes the loss's total by, in nats: the kept frames priced
    /// as read, by their pitch and voicing, less their unread cost. Above zero where reading
    /// them raised the loss, as reading a weakly voiced frame does.
    pub loss_change: f64,
}

/// `reading` with every note carried on over the weakly voiced frames after it that the
/// module's rules give it, each stated in its `breathy`, and every note's residual and the
/// take's measured again. `onsets` are the take's (other events are ignored), and `frames` the
/// performance `reading` was read from.
#[must_use]
pub fn breathe(reading: Reading, frames: &PerformanceFrames, onsets: &[Event]) -> Reading {
    let note_cost = reading.reader.map_or(NOTE_COST, |r| r.note_cost);
    let onsets: Vec<u64> = onsets
        .iter()
        .filter_map(|e| match *e {
            Event::Onset { frame, .. } => Some(frame),
            _ => None,
        })
        .collect();
    let offset = reading.tuning.offset_cents();
    let mut notes = reading.notes.clone();
    let mut carried: Vec<(usize, f32)> = Vec::new();
    for i in 0..notes.len() {
        let next = notes.get(i + 1);
        if let Some((end, lowest_db)) = carry(frames, &onsets, offset, &notes[i], next) {
            notes[i].end = end;
            carried.push((i, lowest_db));
        }
    }
    if carried.is_empty() {
        return reading;
    }
    let total = |notes: &[ReadNote]| {
        residual_with_cost(frames, &reading.tuning, notes, note_cost)
            .1
            .loss
            .total()
    };
    let breathy = carried
        .into_iter()
        .map(|(i, lowest_db)| {
            let decoded = &reading.notes[i];
            Breathy {
                loss_change: total(std::slice::from_ref(&notes[i]))
                    - total(std::slice::from_ref(decoded)),
                decoded: bare(decoded),
                end: notes[i].end,
                lowest_db,
            }
        })
        .collect();
    let (parts, take) = residual_with_cost(frames, &reading.tuning, &notes, note_cost);
    let reader = reading.reader.unwrap_or_else(crate::Reader::rules);
    let mut out = Reading::of(reading.tuning, reader, notes, parts, take);
    out.rejoins = reading.rejoins;
    out.breathy = breathy;
    out.held = reading.held;
    out.suggestions = reading.suggestions;
    out
}

/// Where `note` ends once carried on over the frames after it its reader left unread, and the
/// quietest kept frame's level against its held level, in decibels; `None` when it keeps none.
/// `next` is the note after it, which it never reaches; `offset` is the tuning's, in cents
/// from A440.
fn carry(
    frames: &PerformanceFrames,
    onsets: &[u64],
    offset: f64,
    note: &ReadNote,
    next: Option<&ReadNote>,
) -> Option<(u64, f32)> {
    let hop = u64::from(frames.hop);
    let bound = next.map_or(u64::MAX, |n| n.start);
    let after = indices(frames, note.end, bound);
    // The note's own last frame starts the line: it must have a pitch.
    let last = after.start.checked_sub(1)?;
    if after.is_empty() || frames.cents[last].is_nan() || frames.frame(last) < note.start {
        return None;
    }
    // No onset follows before the next note: an onset in the first frame after the note, or
    // any later before the next note's start, marks what follows as a breath before it.
    let first = frames.frame(after.start);
    if onsets.iter().any(|&f| f + hop / 2 >= first && f < bound) {
        return None;
    }
    let levels: Vec<f32> = indices(frames, note.held_from, note.end)
        .filter(|&k| !frames.cents[k].is_nan())
        .map(|k| frames.level[k])
        .collect();
    if levels.is_empty() {
        return None;
    }
    let held_level = f64::from(median(&levels));
    if held_level <= 0.0 {
        return None;
    }
    let own = key_cents(offset, note.key);
    // A next note on the same key is no slide's destination: the rejoin judges that dip.
    let theirs = next
        .filter(|n| n.key != note.key)
        .map(|n| key_cents(offset, n.key));
    let mut previous = f64::from(frames.cents[last]);
    let mut kept: Option<(usize, f64)> = None;
    for k in after {
        let cents = f64::from(frames.cents[k]);
        if cents.is_nan() || (cents - previous).abs() > BREATHY_STEP_CENTS {
            break;
        }
        let db = 20.0 * (f64::from(frames.level[k]) / held_level).log10();
        if db.is_nan() || db < BREATHY_LEVEL_DB {
            break;
        }
        if theirs.is_some_and(|t| (cents - t).abs() <= (cents - own).abs()) {
            break;
        }
        let lowest = kept.map_or(db, |(_, l)| l.min(db));
        kept = Some((k, lowest));
        previous = cents;
    }
    let (k, lowest) = kept?;
    Some(((frames.frame(k) + hop / 2).min(bound), lowest as f32))
}

#[cfg(test)]
mod tests;
