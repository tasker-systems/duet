//! The note HMM: a take's notes chosen by a hidden Markov model over its performance, on its own
//! tuning, with a stated cost per note.
//!
//! The structure is the one pYIN's note model has (Mauch et al. 2015), read from its source on
//! 2026-11-15 as a description only: it is GPL-2.0-or-later, and nothing of it is copied or
//! linked. It is adapted to the owner's three rulings and to the take's tuning.
//!
//! # States
//!
//! For each MIDI key on the take's tuning, one state per semitone (each note's `centre` carries
//! the fine pitch, so the decoder need not):
//! - **attack**: pitch not yet committed, whatever it is: a flat cost a frame
//!   ([`ATTACK_COST`]) more than a frame held exactly on its key. This is the slide or scoop
//!   into a note, and it belongs to the note it arrives at, since it can only lead to that
//!   key's stable state. (pYIN's attack is Normal about its pitch with σ 5 semitones, which
//!   costs about 2 nats a frame more than holding: enough that a glide is read as a short note
//!   on each key it passes.)
//! - **stable**: held pitch, Normal about the key with σ the vibrato tolerance
//!   ([`STABLE_SIGMA_SEMITONES`]). Stable is the only state that gives a note its key: a note is
//!   held pitch. It is a chain of [`SHORTEST_NOTE_SECS`] worth of frames before it may be left,
//!   so no note is held for less than the shortest note.
//!
//! and one **silent** state: no note sounding.
//!
//! # Transitions
//!
//! - silent → silent, or the attack, or the stable state directly, of **any** key: so a dip
//!   through silence back to the same pitch is two notes, and a note that starts on its pitch
//!   has no slide;
//! - attack → attack, or that key's stable state, each for nothing: an attack's length is left
//!   to its pitch, since its every frame costs [`ATTACK_COST`] and nothing else does. (pYIN's
//!   attack stays with probability 0.9, which favours an attack of ten frames whatever the
//!   pitch does.) **An attack never goes to silent,** and the take never ends in one: a slide
//!   that never holds is no note, so its frames are unread, and unread they are priced as
//!   silent, frame by frame, by their voicing. Were attack → silent allowed, such a run would
//!   cost [`ATTACK_COST`] a frame and never pay `c`, so a short note, or one held far from its
//!   key, would be cheaper left as a slide to nowhere than read, however strongly it was sung.
//! - stable → stable, or **leaving it** for silent, or the attack or stable state of **any** key,
//!   the same key included. The last is **legato**, which pYIN's model lacks: there, stable
//!   leaves only through silent, so a step with no breath is decoded through a silent state
//!   the voicing argues against. The same key is a dip that never goes unvoiced: it leaves the
//!   pitch and comes back.
//!
//! **The cost per note**, `c` ([`NOTE_COST`]), is the negative log probability of leaving
//! stable: every note pays it once, when it ends. Raising it merges notes; lowering it splits
//! them. Where the note goes on to is not itself priced (pYIN weights the next note by its
//! interval), so `c` is exactly what a note costs, wherever it leads; rows of the transition
//! matrix need not then sum to one, and the decoded path is the one of least total cost.
//!
//! **Onsets** raise the probability of leaving stable: at the frame an onset falls in, it is
//! [`ONSET_LEAVE`] rather than `e^-c`, so an onset splits a held pitch in two, as an attack
//! does, and does so whatever `c` is.
//!
//! # Observations
//!
//! A frame's voicing probability `v` weighs note against silence, as pYIN's does: a voiced
//! frame scores `ln v` plus its pitch's log density in a note state, `ln (1 − v)` in silence,
//! each floored at [`VOICING_FLOOR`]. A frame the pitch track decoded unvoiced has no pitch: it
//! scores `ln` [`DROPOUT`] in a note state, so a single dropped frame stays inside a note and
//! two or more end it.
//!
//! These scores are the negated prices of the residual's functions
//! ([`sounding_cost`], [`silent_cost`], [`held_cost`]), which the loss sums too: the path the
//! decoder chooses is the reading of least loss, but for the differences
//! [`Loss`](crate::Loss) names.
//!
//! # Decoding
//!
//! Fixed-lag Viterbi, as the pitch track decodes: a frame's state is settled once
//! [`LAG_SECS`] of frames after it have been seen, so the stream rule holds and nothing is
//! decoded over the whole take at once.

use crate::event::Event;
use crate::notes::{
    LEAST_CONFIDENCE, ONSET_GRACE_SECS, SHORTEST_NOTE_SECS, STRENGTH_WINDOW_SECS, level_strength,
    median,
};
use crate::reading::{PerformanceFrames, ReadNote, Reader, Reading, tuning_of};
use crate::residual::{NOTE_COST, held_cost, residual_with_cost, silent_cost, sounding_cost};

/// The held pitch's σ, in semitones: about the vibrato tolerance. A vibrato swings up to half a
/// semitone either side of where it is held; at this σ, a frame at the top of such a swing
/// still scores within a nat of one on the key, and a frame a whole semitone away scores two
/// nats worse than one on its own key, every frame it is held there.
pub const STABLE_SIGMA_SEMITONES: f64 = 0.5;
/// What a frame costs in an attack, in nats, beyond a frame held exactly on its key, whatever
/// its pitch: a slide is pitch on its way somewhere. At 0.5, a frame further than half a
/// semitone (one [`STABLE_SIGMA_SEMITONES`]) from a key is cheaper as slide than held on it.
/// A glide of 20 semitones a second passes a key in about 4 frames, which cost 2 nats as slide
/// and at least `c` as a note of their own, so it is no note at `c` or at `c/2`.
pub const ATTACK_COST: f64 = 0.5;
/// The probability of leaving stable at the frame an onset falls in.
pub const ONSET_LEAVE: f64 = 0.9;
/// The probability of a note state on a frame the pitch track decoded unvoiced.
pub const DROPOUT: f64 = 0.05;
/// The least probability a frame's voicing, or its complement, is taken as.
pub const VOICING_FLOOR: f64 = 0.01;
/// How long after a frame its state is settled.
pub const LAG_SECS: f64 = 1.0;
/// MIDI keys: every one has its states, so the model needs nothing of the take in advance.
const KEYS: usize = 128;

/// The note HMM, with its cost per note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteModel {
    /// The cost of a note, `c`, in nats: [`NOTE_COST`] by default.
    pub note_cost: f64,
}

impl Default for NoteModel {
    fn default() -> NoteModel {
        NoteModel {
            note_cost: NOTE_COST,
        }
    }
}

/// What a frame is, to the model: its pitch in fractional keys on the take's tuning (`NaN`
/// when unvoiced), its voicing, and whether an onset falls in it.
#[derive(Debug, Clone, Copy)]
struct Observed {
    pitch: f64,
    voicing: f64,
    onset: bool,
}

/// State indices: silent, then per key its attack and its stable chain.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// The stable chain's length, in frames: the shortest note.
    chain: usize,
}

impl Layout {
    const SILENT: usize = 0;

    fn per_key(self) -> usize {
        1 + self.chain
    }

    fn states(self) -> usize {
        1 + KEYS * self.per_key()
    }

    fn attack(self, key: usize) -> usize {
        1 + key * self.per_key()
    }

    fn stable(self, key: usize, step: usize) -> usize {
        self.attack(key) + 1 + step
    }

    fn held(self, key: usize) -> usize {
        self.stable(key, self.chain - 1)
    }

    /// What `state` is: `None` for silent, else its key and, for a stable state, its step
    /// along the chain.
    fn what(self, state: usize) -> Option<(usize, Option<usize>)> {
        if state == Self::SILENT {
            return None;
        }
        let i = state - 1;
        Some((i / self.per_key(), (i % self.per_key()).checked_sub(1)))
    }
}

/// Costs, as log probabilities (≤ 0), of one model.
struct Costs {
    stay_held: f64,
    leave_held: f64,
    stay_held_onset: f64,
    leave_held_onset: f64,
}

impl Costs {
    fn new(note_cost: f64) -> Costs {
        let leave = (-note_cost).exp();
        Costs {
            stay_held: (1.0 - leave).ln(),
            leave_held: -note_cost,
            stay_held_onset: (1.0 - ONSET_LEAVE).ln(),
            leave_held_onset: ONSET_LEAVE.ln(),
        }
    }
}

/// Fixed-lag Viterbi over the note model.
struct Decoder {
    layout: Layout,
    costs: Costs,
    lag: usize,
    /// The best log score of a path ending in each state; empty before the first frame.
    score: Vec<f64>,
    /// For each of the newest frames, each state's best predecessor; newest last.
    back: std::collections::VecDeque<Vec<u16>>,
    unsettled: usize,
}

impl Decoder {
    fn new(layout: Layout, note_cost: f64, lag: usize) -> Decoder {
        let mut score = vec![f64::NEG_INFINITY; layout.states()];
        score[Layout::SILENT] = 0.0;
        Decoder {
            layout,
            costs: Costs::new(note_cost),
            lag,
            score,
            back: std::collections::VecDeque::new(),
            unsettled: 0,
        }
    }

    /// Each state's log likelihood of `o`, written into `out`.
    fn observe(&self, o: Observed, out: &mut [f64]) {
        if o.pitch.is_nan() {
            out.fill(DROPOUT.ln());
            out[Layout::SILENT] = 0.0;
            return;
        }
        let sounding = -sounding_cost(o.voicing);
        out[Layout::SILENT] = -silent_cost(o.voicing);
        let attack = sounding - ATTACK_COST;
        for key in 0..KEYS {
            let stable = sounding - held_cost(o.pitch - key as f64);
            out[self.layout.attack(key)] = attack;
            for step in 0..self.layout.chain {
                out[self.layout.stable(key, step)] = stable;
            }
        }
    }

    /// Takes the next frame, and returns the state of the frame `lag` frames back once it has
    /// settled.
    fn step(&mut self, o: Observed) -> Option<usize> {
        let l = self.layout;
        let c = &self.costs;
        let (stay_held, leave_held) = if o.onset {
            (c.stay_held_onset, c.leave_held_onset)
        } else {
            (c.stay_held, c.leave_held)
        };
        let old = &self.score;
        // The best way into a new note: from silence, for free, or leaving a held note.
        let mut entry = (old[Layout::SILENT], Layout::SILENT);
        for key in 0..KEYS {
            let s = old[l.held(key)] + leave_held;
            if s > entry.0 {
                entry = (s, l.held(key));
            }
        }
        let mut score = vec![f64::NEG_INFINITY; l.states()];
        let mut back = vec![0u16; l.states()];
        // Silence is entered from silence or from a held note, never from an attack.
        let silent = entry;
        for key in 0..KEYS {
            let attack = l.attack(key);
            let from_attack = old[attack];
            let stay = old[attack];
            (score[attack], back[attack]) = if stay > entry.0 {
                (stay, attack as u16)
            } else {
                (entry.0, entry.1 as u16)
            };
            let first = l.stable(key, 0);
            (score[first], back[first]) = if from_attack > entry.0 {
                (from_attack, attack as u16)
            } else {
                (entry.0, entry.1 as u16)
            };
            for step in 1..l.chain {
                let s = l.stable(key, step);
                (score[s], back[s]) = (old[s - 1], (s - 1) as u16);
            }
            let held = l.held(key);
            let stay = old[held] + stay_held;
            if stay > score[held] {
                (score[held], back[held]) = (stay, held as u16);
            }
        }
        (score[Layout::SILENT], back[Layout::SILENT]) = (silent.0, silent.1 as u16);
        let mut observed = vec![0.0; l.states()];
        self.observe(o, &mut observed);
        for (s, o) in score.iter_mut().zip(&observed) {
            *s += o;
        }
        let top = score.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        score.iter_mut().for_each(|s| *s -= top);
        self.score = score;
        self.back.push_back(back);
        if self.back.len() > self.lag {
            self.back.pop_front();
        }
        self.unsettled += 1;
        if self.unsettled <= self.lag {
            return None;
        }
        self.unsettled -= 1;
        let mut state = self.best(false);
        for back in self.back.iter().rev().take(self.lag) {
            state = usize::from(back[state]);
        }
        Some(state)
    }

    /// The states of every frame not yet settled, oldest first, on the best path to the end of
    /// the take, where a note still held pays its cost too.
    fn flush(&mut self) -> Vec<usize> {
        if self.unsettled == 0 {
            return Vec::new();
        }
        let mut state = self.best(true);
        let mut states = vec![state];
        for back in self.back.iter().rev().take(self.unsettled - 1) {
            state = usize::from(back[state]);
            states.push(state);
        }
        states.reverse();
        self.unsettled = 0;
        states
    }

    /// The state the best path so far ends in; at the take's `ending`, a held note pays its
    /// cost and an attack, which can no longer hold, is no end.
    fn best(&self, ending: bool) -> usize {
        let end = |s: usize| match self.layout.what(s) {
            Some((_, Some(_))) if ending => self.costs.leave_held,
            Some((_, None)) if ending => f64::NEG_INFINITY,
            _ => 0.0,
        };
        (0..self.score.len())
            .max_by(|&a, &b| (self.score[a] + end(a)).total_cmp(&(self.score[b] + end(b))))
            .unwrap_or(Layout::SILENT)
    }
}

impl NoteModel {
    /// A model whose cost per note is `note_cost`.
    #[must_use]
    pub const fn with_cost(note_cost: f64) -> NoteModel {
        NoteModel { note_cost }
    }

    /// Reads `frames` and `onsets` (their [`Event::Onset`]s; other events are ignored) on
    /// `given`, A4 in absolute cents, or on the tuning estimated from the frames, as
    /// [`read`](crate::read) does, with the notes chosen by this model.
    #[must_use]
    pub fn read(
        &self,
        frames: &PerformanceFrames,
        onsets: &[Event],
        given: Option<f64>,
    ) -> Reading {
        let lag = frames_in(frames, LAG_SECS);
        self.read_with_lag(frames, onsets, given, lag)
    }

    fn read_with_lag(
        &self,
        frames: &PerformanceFrames,
        onsets: &[Event],
        given: Option<f64>,
        lag: usize,
    ) -> Reading {
        let tuning = tuning_of(frames, given);
        let offset = tuning.offset_cents();
        let observed = observe(frames, onsets, offset);
        let states = self.decode(&observed, frames_in_ceil(frames, SHORTEST_NOTE_SECS), lag);
        let notes = notes(frames, &observed, onsets, &states);
        let (parts, take) = residual_with_cost(frames, &tuning, &notes, self.note_cost);
        Reading::of(tuning, Reader::hmm(self.note_cost), notes, parts, take)
    }

    /// Each frame's state, decoded with a lag of `lag` frames, and whether it is a held one.
    fn decode(&self, observed: &[Observed], chain: usize, lag: usize) -> Vec<State> {
        let layout = Layout {
            chain: chain.max(1),
        };
        let mut decoder = Decoder::new(layout, self.note_cost, lag.max(1));
        let mut states = Vec::with_capacity(observed.len());
        for &o in observed {
            states.extend(decoder.step(o));
        }
        states.extend(decoder.flush());
        states
            .into_iter()
            .map(|s| match layout.what(s) {
                None => State::Silent,
                Some((key, None)) => State::Attack(key as u8),
                Some((key, Some(step))) => State::Stable(key as u8, step == 0),
            })
            .collect()
    }
}

/// A frame's decoded state, by what it says of the notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Silent,
    Attack(u8),
    /// A held frame of a key, and whether it is the first of a new note's.
    Stable(u8, bool),
}

fn frames_in(frames: &PerformanceFrames, secs: f64) -> usize {
    ((secs * f64::from(frames.rate) / f64::from(frames.hop)).round() as usize).max(1)
}

/// Frames spanning at least `secs`.
fn frames_in_ceil(frames: &PerformanceFrames, secs: f64) -> usize {
    ((secs * f64::from(frames.rate) / f64::from(frames.hop)).ceil() as usize).max(1)
}

/// Each frame as the model sees it: its pitch on the take's tuning (`offset` cents from A440),
/// and whether an onset falls in it, that is within half a hop of it.
fn observe(frames: &PerformanceFrames, onsets: &[Event], offset: f64) -> Vec<Observed> {
    let mut observed: Vec<Observed> = (0..frames.len())
        .map(|k| {
            let cents = f64::from(frames.cents[k]);
            Observed {
                pitch: (cents - offset) / 100.0,
                voicing: f64::from(frames.voicing[k]),
                onset: false,
            }
        })
        .collect();
    let hop = u64::from(frames.hop);
    for event in onsets {
        if let Event::Onset { frame, .. } = *event {
            let from = frames.first_frame.saturating_sub(hop / 2);
            let k = ((frame.max(from) - from) / hop) as usize;
            if let Some(o) = observed.get_mut(k) {
                o.onset = true;
            }
        }
    }
    observed
}

/// A note as the decoded states give it: its frames' indices.
struct Span {
    /// Its first frame, attack included.
    first: usize,
    /// Its first held frame.
    held: usize,
    /// One past its last frame.
    end: usize,
    /// Whether it follows the note before it with no silence between: legato.
    legato: bool,
}

/// The notes in `states`: each a run of attack frames, then held frames of one key. A note
/// ends where its held frames end: at silence, at an attack, or at held frames of another key
/// or of a new note of the same key. An attack that never reaches held frames is no note.
///
/// A fixed lag settles each frame on the best path so far, so two neighbouring frames can be
/// settled on paths that differ; every change of key or from held to attack is a boundary, so a
/// note is always one key's held pitch whatever the paths.
fn spans(states: &[State]) -> Vec<(Span, u8)> {
    let mut out: Vec<(Span, u8)> = Vec::new();
    // The attack in progress: where it began, and whether a note sounded right before it.
    let mut attack: Option<(usize, bool)> = None;
    let mut sounding: Option<(Span, u8)> = None;
    let mut previous = State::Silent;
    for (k, &state) in states.iter().enumerate() {
        match state {
            State::Silent => {
                out.extend(sounding.take());
                attack = None;
            }
            State::Attack(_) => {
                if attack.is_none() {
                    let legato = sounding.is_some();
                    if let Some(mut done) = sounding.take() {
                        done.0.end = k;
                        out.push(done);
                    }
                    attack = Some((k, legato));
                }
            }
            State::Stable(key, first) => {
                let continues = !first && matches!(previous, State::Stable(p, _) if p == key);
                if continues && let Some(s) = &mut sounding {
                    s.0.end = k + 1;
                    previous = state;
                    continue;
                }
                let legato = sounding.is_some();
                if let Some(mut done) = sounding.take() {
                    done.0.end = k;
                    out.push(done);
                }
                let (first, legato) = attack.take().unwrap_or((k, legato));
                sounding = Some((
                    Span {
                        first,
                        held: k,
                        end: k + 1,
                        legato,
                    },
                    key,
                ));
            }
        }
        previous = state;
    }
    out.extend(sounding);
    out
}

/// The decoded notes as read notes: their frames on the take's timeline, their key the nearest
/// to their held frames' median pitch (the decoded key, unless that median sits further than
/// half a key from it), their confidence their frames' mean voicing, and their strength the
/// latest onset's within [`STRENGTH_WINDOW_SECS`] before they start, or, for a note that
/// follows another legato with no onset, the note before's, or else their level's. Notes
/// shorter than the shortest note, or more weakly voiced than the rules allow one, are
/// dropped, as the rules drop them.
fn notes(
    frames: &PerformanceFrames,
    observed: &[Observed],
    onsets: &[Event],
    states: &[State],
) -> Vec<ReadNote> {
    let hop = u64::from(frames.hop);
    let at = |secs: f64| (secs * f64::from(frames.rate)).round() as u64;
    let (grace, window, shortest) = (
        at(ONSET_GRACE_SECS),
        at(STRENGTH_WINDOW_SECS),
        at(SHORTEST_NOTE_SECS),
    );
    let onsets: Vec<(u64, f32)> = onsets
        .iter()
        .filter_map(|e| match *e {
            Event::Onset { frame, strength } => Some((frame, strength)),
            _ => None,
        })
        .collect();
    let mut out: Vec<ReadNote> = Vec::new();
    let mut previous_end: Option<u64> = None;
    for (span, _) in spans(states) {
        let voiced = frames.frame(span.first);
        let held: Vec<f32> = (span.held..span.end)
            .filter_map(|k| {
                let p = observed[k].pitch;
                (!p.is_nan()).then_some(p as f32)
            })
            .collect();
        if held.is_empty() {
            continue;
        }
        let since_previous = previous_end.unwrap_or(0);
        let onset = onsets
            .iter()
            .rfind(|&&(f, _)| f + window >= voiced && f <= voiced + hop / 2 && f >= since_previous)
            .copied();
        let mut start = match onset {
            Some((f, _)) if f + grace >= voiced => f,
            _ => voiced.saturating_sub(hop / 2),
        };
        if span.legato {
            start = voiced.saturating_sub(hop / 2);
        }
        let end = frames.frame(span.end - 1) + hop / 2;
        let count = (span.end - span.first) as f32;
        let confidence = (span.first..span.end)
            .map(|k| frames.voicing[k])
            .sum::<f32>()
            / count;
        let level = (span.first..span.end).map(|k| frames.level[k]).sum::<f32>() / count;
        let inherited = span
            .legato
            .then(|| {
                out.last()
                    .filter(|n| n.end + hop >= start)
                    .map(|n| n.strength)
            })
            .flatten();
        let strength = onset
            .map(|o| o.1)
            .or(inherited)
            .unwrap_or_else(|| level_strength(level));
        if end < start + shortest || confidence < LEAST_CONFIDENCE {
            continue;
        }
        if span.legato
            && let Some(last) = out.last_mut()
            && last.end > start
        {
            last.end = start;
        }
        let held_from = frames.frame(span.held).saturating_sub(hop / 2).max(start);
        out.push(
            ReadNote::held_at(median(&held), start, end, held_from).with(confidence, strength),
        );
        previous_end = Some(end);
    }
    out
}

#[cfg(test)]
mod tests;
