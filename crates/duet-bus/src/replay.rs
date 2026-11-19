//! Folding the journal through its two lifecycles, the ask's and a song's status: leniently on
//! replay, strictly for a new entry.

use std::collections::HashMap;

use duet_schemas::journal::{Event, Id, Line, SongStatusSet};
use duet_schemas::song::Status;

use crate::ask::{AskState, Refusal, step_of, transition};
use crate::{Place, ProjectJournal, song};

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

/// One song's status, as the journal has it so far: where the last move that stands left it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SongRecord {
    pub status: Status,
    /// Where the `song.status-set` that stands is.
    pub set: Place,
    /// The song's name, as that move gave it.
    pub name: String,
}

/// A status move replay skipped, because the song's lifecycle refused it. Like a skipped ask
/// transition, it can come only from a union merge of two clones or a hand edit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RefusedMove {
    /// The skipped entry.
    pub place: Place,
    pub id: Id,
    /// The song it would have moved, and its name as the move gives it.
    pub song: Id,
    pub name: String,
    pub from: Status,
    pub to: Status,
    pub refusal: song::Refusal,
    /// Where the move that stands is, if one does: the last before it in replay order.
    pub stands: Option<Place>,
}

/// The journal folded through its lifecycles, in replay order: the segments in index order,
/// each in file order.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Replay {
    asks: HashMap<Id, AskRecord>,
    songs: HashMap<Id, SongRecord>,
    /// Every ask transition replay skipped, in replay order.
    pub refused: Vec<Refused>,
    /// Every status move replay skipped, in replay order.
    pub refused_moves: Vec<RefusedMove>,
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
        if let Some(set) = song::step_of(event) {
            self.fold_move(place, line, set);
            return;
        }
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

    /// Folds a status move, leniently: one the song's lifecycle refuses is skipped and recorded
    /// in [`Replay::refused_moves`], and the song keeps the status the move before it gave.
    fn fold_move(&mut self, place: Place, line: &Line, set: &SongStatusSet) {
        let record = self.songs.get(&set.song);
        match song::transition(record.map(|r| r.status), set.from, set.to) {
            Ok(status) => {
                self.songs.insert(
                    set.song.clone(),
                    SongRecord {
                        status,
                        set: place,
                        name: set.name.to_string(),
                    },
                );
            }
            Err(refusal) => self.refused_moves.push(RefusedMove {
                place,
                id: line.id.clone(),
                song: set.song.clone(),
                name: set.name.to_string(),
                from: set.from,
                to: set.to,
                refusal,
                stands: record.map(|r| r.set),
            }),
        }
    }

    /// Whether a status move, `event`, is allowed now: the strict check a new `song.status-set`
    /// passes before it is appended. Every other event is allowed.
    ///
    /// # Errors
    ///
    /// The [`song::Refusal`] the song's lifecycle gives, with the song it names.
    pub fn check_move(&self, event: &Event) -> Result<(), (song::Refusal, Id)> {
        let Some(set) = song::step_of(event) else {
            return Ok(());
        };
        song::transition(self.song(&set.song).map(|r| r.status), set.from, set.to)
            .map(|_| ())
            .map_err(|refusal| (refusal, set.song.clone()))
    }

    /// The song `id`'s status as the journal has it, if a move for it stands.
    #[must_use]
    pub fn song(&self, id: &Id) -> Option<&SongRecord> {
        self.songs.get(id)
    }

    /// Every song a move stands for, with where the journal has it.
    pub fn songs(&self) -> impl Iterator<Item = (&Id, &SongRecord)> {
        self.songs.iter()
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
