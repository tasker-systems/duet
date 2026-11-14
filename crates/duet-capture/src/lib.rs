//! duet capture: audio in, analysis events out.
//!
//! Capture is a chain of stages over a stream: a [`Source`] yields [`Chunk`]s, and each
//! [`Stage`] takes input a piece at a time, holds it back only until its output is settled,
//! and reports how long that takes ([`Stage::latency_frames`]). The vocabulary the stages
//! speak is [`Event`]. Times inside the chain are sample frames at the analysis rate, not
//! musical positions: musical time does not exist until a tempo does.
//!
//! The stream rule, from the design's anti-goals: "A stage that buffers the entire take to
//! do its work, other than the final assembly into a `Song`, is a design failure."
//! Live input would otherwise be a rewrite.

#![forbid(unsafe_code)]

mod error;
mod event;
mod stage;

pub use error::{Error, Result};
pub use event::{Event, HitLane};
pub use stage::{Chunk, Mono, Source, Stage};
