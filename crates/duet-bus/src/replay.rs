//! Folding the journal through the ask's lifecycle: leniently on replay, strictly for a new
//! entry.

use std::collections::HashMap;

use duet_schemas::journal::{Event, Id, Line};

use crate::ask::{AskState, Refusal, step_of, transition};
use crate::{Place, ProjectJournal};

/// One ask, as the journal has it so far.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AskRecord {
    /// Where its `ask.asked` is.
    pub asked: Place,
    pub state: AskState,
    /// Where the entry that answered or withdrew it is: the one that stands.
    pub settled: Option<Place>,
}

/// A transition replay skipped, because its lifecycle refused it. It can come only from a union
/// merge of two clones or a hand edit, such as two answers to one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refused {
    /// The skipped entry.
    pub place: Place,
    pub id: Id,
    pub event: &'static str,
    /// The ask it would have stepped.
    pub ask: Id,
    pub refusal: Refusal,
    /// Where the entry that stands in its place is, if one does: the first in replay order.
    pub stands: Option<Place>,
}

/// The journal folded through the ask's lifecycle, in replay order: the segments in index
/// order, each in file order.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Replay {
    asks: HashMap<Id, AskRecord>,
    /// Every transition replay skipped, in replay order.
    pub refused: Vec<Refused>,
}

impl Replay {
    /// Nothing folded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds every entry of `journal`, in replay order.
    #[must_use]
    pub fn of(journal: &ProjectJournal) -> Self {
        let mut replay = Self::new();
        for (place, line) in journal.entries() {
            replay.fold(place, line);
        }
        replay
    }

    /// Folds the line at `place`, leniently: a transition its lifecycle refuses is skipped and
    /// recorded in [`Replay::refused`], and the ask keeps the state the first in order gave it.
    /// A line this build did not read changes nothing.
    pub fn fold(&mut self, place: Place, line: &Line) {
        let Some(event) = line.read() else {
            return;
        };
        let Some((ask, step)) = step_of(&line.id, event) else {
            return;
        };
        let record = self.asks.get(ask);
        match transition(record.map(|r| r.state), step) {
            Ok(state) => match self.asks.get_mut(ask) {
                Some(record) => {
                    record.state = state;
                    record.settled = Some(place);
                }
                None => {
                    self.asks.insert(
                        ask.clone(),
                        AskRecord {
                            asked: place,
                            state,
                            settled: None,
                        },
                    );
                }
            },
            Err(refusal) => {
                let stands = record.and_then(|r| match refusal {
                    Refusal::AlreadyAsked => Some(r.asked),
                    _ => r.settled,
                });
                self.refused.push(Refused {
                    place,
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

    /// The asks still open, oldest first.
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
