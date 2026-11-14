//! duet capture: audio in, analysis events out.
//!
//! Capture is a chain of stages over a stream: a [`Source`] yields [`Chunk`]s, and each
//! [`Stage`] takes input a piece at a time, holds it back only until its output is settled,
//! and reports how long that takes ([`Stage::latency_frames`]). The vocabulary the stages
//! speak is [`Event`]. Times inside the chain are sample frames at the analysis rate, not
//! musical positions: musical time does not exist until a tempo does.
//!
//! Known boundary: [`FileSource`] refuses HE-AAC by name when the file's header declares it.
//! HE-AAC signalled only inside the audio data decodes as AAC-LC at half bandwidth, without
//! a refusal; catching it would mean comparing declared and decoded sample rates.
//!
//! The stream rule, from the design's anti-goals: "A stage that buffers the entire take to
//! do its work, other than the final assembly into a `Song`, is a design failure."
//! Live input would otherwise be a rewrite.

#![forbid(unsafe_code)]

mod error;
mod event;
mod notes;
mod onset;
mod pitch;
mod prepare;
mod source;
mod stage;
#[cfg(test)]
mod synth;

pub use error::{Error, Result};
pub use event::{Event, HitLane};
pub use notes::Melody;
pub use onset::Onsets;
pub use pitch::{PitchFrame, PitchTrack};
pub use prepare::{ANALYSIS_RATE, Prepare};
pub use source::FileSource;
pub use stage::{Chunk, Mono, Source, Stage};
