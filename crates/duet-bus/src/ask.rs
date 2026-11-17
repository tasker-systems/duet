//! An ask's lifecycle: `open`, then `answered` or `withdrawn`, each terminal.
//!
//! It is the one lifecycle in the journal. [`transition`] is a single `match` on the current
//! state and the step; an arm that is not listed refuses, naming why. A terminal state never
//! changes. The journal is kept for ever, so every transition ever written must still replay: a
//! change to this lifecycle is a new event version, never an edit to an arm.

use std::fmt;

use duet_schemas::journal::{Event, Id};

/// Where an ask is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskState {
    Open,
    /// Terminal.
    Answered,
    /// Terminal.
    Withdrawn,
}

/// What an event does to an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Ask,
    Answer,
    Withdraw,
}

/// Why a step is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// An ask with this id was asked already.
    AlreadyAsked,
    /// No ask with this id was asked before it.
    NotAsked,
    /// The ask was answered already.
    AlreadyAnswered,
    /// The ask was withdrawn already.
    AlreadyWithdrawn,
}

impl Refusal {
    /// The refusal's name, as a refusal on the wire names it: `already-answered`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::AlreadyAsked => "already-asked",
            Self::NotAsked => "not-asked",
            Self::AlreadyAnswered => "already-answered",
            Self::AlreadyWithdrawn => "already-withdrawn",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The ask's next state after `step`, from `state` (`None`: not asked yet).
///
/// # Errors
///
/// The [`Refusal`] naming why `step` cannot be taken from `state`.
pub const fn transition(state: Option<AskState>, step: Step) -> Result<AskState, Refusal> {
    match (state, step) {
        (None, Step::Ask) => Ok(AskState::Open),
        (None, Step::Answer | Step::Withdraw) => Err(Refusal::NotAsked),
        (Some(_), Step::Ask) => Err(Refusal::AlreadyAsked),
        (Some(AskState::Open), Step::Answer) => Ok(AskState::Answered),
        (Some(AskState::Open), Step::Withdraw) => Ok(AskState::Withdrawn),
        (Some(AskState::Answered), _) => Err(Refusal::AlreadyAnswered),
        (Some(AskState::Withdrawn), _) => Err(Refusal::AlreadyWithdrawn),
    }
}

/// The ask an event steps, and how, if it steps one. An `ask.asked` entry's own `id` is its
/// ask's.
#[must_use]
pub fn step_of<'a>(id: &'a Id, event: &'a Event) -> Option<(&'a Id, Step)> {
    match event {
        Event::AskAsked(_) => Some((id, Step::Ask)),
        Event::AskAnswered(answered) => Some((&answered.ask, Step::Answer)),
        Event::AskWithdrawn(withdrawn) => Some((&withdrawn.ask, Step::Withdraw)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_arm() {
        use AskState::{Answered, Open, Withdrawn};
        use Step::{Answer, Ask, Withdraw};
        let table = [
            (None, Ask, Ok(Open)),
            (None, Answer, Err(Refusal::NotAsked)),
            (None, Withdraw, Err(Refusal::NotAsked)),
            (Some(Open), Ask, Err(Refusal::AlreadyAsked)),
            (Some(Open), Answer, Ok(Answered)),
            (Some(Open), Withdraw, Ok(Withdrawn)),
            (Some(Answered), Ask, Err(Refusal::AlreadyAsked)),
            (Some(Answered), Answer, Err(Refusal::AlreadyAnswered)),
            (Some(Answered), Withdraw, Err(Refusal::AlreadyAnswered)),
            (Some(Withdrawn), Ask, Err(Refusal::AlreadyAsked)),
            (Some(Withdrawn), Answer, Err(Refusal::AlreadyWithdrawn)),
            (Some(Withdrawn), Withdraw, Err(Refusal::AlreadyWithdrawn)),
        ];
        for (state, step, want) in table {
            assert_eq!(transition(state, step), want, "{state:?} {step:?}");
        }
    }
}
