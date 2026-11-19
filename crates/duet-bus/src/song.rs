//! A song's status, as the journal has it: the second lifecycle in the journal, beside the ask's
//! (bus design decision 23).
//!
//! The states and the moves the machine draws are duet-schemas' ([`Status`], the domain model's
//! ruling 9): `idea` → `sketch` → `developing` → `arranged`, and `arranged` → `developing` to
//! reopen. No state is terminal.
//!
//! A `song.status-set` states the move it made, `from` and `to`. [`transition`] is a single
//! `match` on what the journal has the song at and that move: the move must be one the machine
//! draws, and its `from` must be where the journal has the song. A song the journal has no move
//! for yet takes the first drawn move as it is, since the journal may begin after the song did.
//!
//! The document holds the current status, and the journal each change (ruling 14): replay never
//! reads `song.duet`. `duet journal check` compares the two ([`crate::Report`]).

use std::fmt;

use duet_schemas::journal::{Event, SongStatusSet};
use duet_schemas::song::Status;

/// Why a status move is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The move is not one the machine draws, a state to itself included.
    NotAMove { from: Status, to: Status },
    /// The move starts from a state other than the one the journal has the song at: two clones
    /// that each moved it, merged, or a move made after a change the journal never saw.
    NotAt { at: Status, from: Status },
}

impl Refusal {
    /// The refusal's name, as a refusal on the wire names it: `not-at`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotAMove { .. } => "not-a-move",
            Self::NotAt { .. } => "not-at",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAMove { from, to } => {
                write!(f, "{}: {from} → {to} is not a move", self.code())
            }
            Self::NotAt { at, from } => write!(
                f,
                "{}: it moves from {from}, and the journal has the song at {at}",
                self.code()
            ),
        }
    }
}

/// The song's status after the move `from` → `to`, from `state` (`None`: the journal has no
/// move for it yet).
///
/// # Errors
///
/// The [`Refusal`] naming why the move cannot be taken from `state`.
pub fn transition(state: Option<Status>, from: Status, to: Status) -> Result<Status, Refusal> {
    match state {
        Some(at) if at != from => Err(Refusal::NotAt { at, from }),
        _ if from.moves().contains(&to) => Ok(to),
        _ => Err(Refusal::NotAMove { from, to }),
    }
}

/// The status move an event makes, if it makes one.
#[must_use]
pub const fn step_of(event: &Event) -> Option<&SongStatusSet> {
    match event {
        Event::SongStatusSet(set) => Some(set),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_arm() {
        use Status::{Arranged, Developing, Idea, Sketch};
        for from in Status::ALL {
            for to in Status::ALL {
                let drawn = matches!(
                    (from, to),
                    (Idea, Sketch)
                        | (Sketch, Developing)
                        | (Developing, Arranged)
                        | (Arranged, Developing)
                );
                let want = if drawn {
                    Ok(to)
                } else {
                    Err(Refusal::NotAMove { from, to })
                };
                assert_eq!(transition(None, from, to), want, "none: {from} → {to}");
                assert_eq!(transition(Some(from), from, to), want, "{from} → {to}");
                for at in Status::ALL.into_iter().filter(|at| *at != from) {
                    assert_eq!(
                        transition(Some(at), from, to),
                        Err(Refusal::NotAt { at, from }),
                        "at {at}: {from} → {to}"
                    );
                }
            }
        }
    }
}
