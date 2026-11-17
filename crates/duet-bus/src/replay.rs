//! Folding the journal through the ask's lifecycle: leniently on replay, strictly for a new
//! entry.

use std::collections::HashMap;

use duet_schemas::journal::{Event, Id, Line};

use crate::ask::{AskState, Refusal, step_of, transition};

/// One ask, as the journal has it so far.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AskRecord {
    /// The `seq` of its `ask.asked`.
    pub asked: u64,
    pub state: AskState,
    /// The `seq` of the entry that answered or withdrew it, the one that stands.
    pub settled: Option<u64>,
}

/// A transition replay skipped, because its lifecycle refused it. It can come only from a union
/// merge of two clones or a hand edit, such as two answers to one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refused {
    /// The skipped entry.
    pub seq: u64,
    pub id: Id,
    pub event: &'static str,
    /// The ask it would have stepped.
    pub ask: Id,
    pub refusal: Refusal,
    /// The `seq` of the entry that stands in its place, if one does: the first in file order.
    pub stands: Option<u64>,
}

/// The journal folded through the ask's lifecycle, in file order.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Replay {
    asks: HashMap<Id, AskRecord>,
    /// Every transition replay skipped, in file order.
    pub refused: Vec<Refused>,
}

impl Replay {
    /// Nothing folded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds every line of `lines`, `lines[0]` being `seq` 1.
    #[must_use]
    pub fn of(lines: &[Line]) -> Self {
        let mut replay = Self::new();
        for (seq, line) in (1..).zip(lines) {
            replay.fold(seq, line);
        }
        replay
    }

    /// Folds the line at `seq`, leniently: a transition its lifecycle refuses is skipped and
    /// recorded in [`Replay::refused`], and the ask keeps the state the first in file order gave
    /// it. A line this build did not read changes nothing.
    pub fn fold(&mut self, seq: u64, line: &Line) {
        let Some(event) = line.read() else {
            return;
        };
        let Some((ask, step)) = step_of(&line.id, event) else {
            return;
        };
        let record = self.asks.get(ask);
        match transition(record.map(|r| r.state), step) {
            Ok(state) => {
                let record = self.asks.entry(ask.clone()).or_insert(AskRecord {
                    asked: seq,
                    state,
                    settled: None,
                });
                if record.asked != seq {
                    record.state = state;
                    record.settled = Some(seq);
                }
            }
            Err(refusal) => {
                let stands = record.and_then(|r| match refusal {
                    Refusal::AlreadyAsked => Some(r.asked),
                    _ => r.settled,
                });
                self.refused.push(Refused {
                    seq,
                    id: line.id.clone(),
                    event: event.name(),
                    ask: ask.clone(),
                    refusal,
                    stands,
                });
            }
        }
    }

    /// Whether `event`, entered with `id`, is allowed now: the strict check a new entry passes
    /// before it is appended.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] its lifecycle gives, with the ask it names.
    pub fn check(&self, id: &Id, event: &Event) -> Result<(), (Refusal, Id)> {
        let Some((ask, step)) = step_of(id, event) else {
            return Ok(());
        };
        transition(self.asks.get(ask).map(|r| r.state), step)
            .map(|_| ())
            .map_err(|refusal| (refusal, ask.clone()))
    }

    /// The ask named `id`, if it was asked.
    #[must_use]
    pub fn ask(&self, id: &Id) -> Option<&AskRecord> {
        self.asks.get(id)
    }

    /// The asks still open, oldest first, by the `seq` they were asked at.
    #[must_use]
    pub fn open(&self) -> Vec<(&Id, &AskRecord)> {
        let mut open: Vec<_> = self
            .asks
            .iter()
            .filter(|(_, r)| r.state == AskState::Open)
            .collect();
        open.sort_by_key(|(_, r)| r.asked);
        open
    }
}
