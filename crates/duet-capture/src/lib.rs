//! duet capture: audio in, analysis events out, and a `Song` assembled from them.
//!
//! # The pipeline
//!
//! ```text
//! FileSource --Chunk--> Prepare --Mono--> Melody | Hits --Event--> estimate --Timing--> assemble --> Captured
//! ```
//!
//! - [`Source`] yields [`Chunk`]s of interleaved samples. [`FileSource`] reads WAV, FLAC, MP3,
//!   Ogg Vorbis and AAC-LC in M4A through Symphonia, a packet at a time.
//! - [`Prepare`] downmixes to one channel and resamples to [`ANALYSIS_RATE`], so every stage
//!   after it can assume both. Its output is [`Mono`].
//! - [`Melody`] (pitch tracking by [`PitchTrack`], onsets by [`Onsets`], segmented into notes)
//!   and [`Hits`] (onsets, each given a lane by [`LaneMode`]) turn mono audio into [`Event`]s:
//!   [`Event::Note`], [`Event::Hit`] and [`Event::Onset`], with [`HitLane`] naming a hit's lane.
//! - [`estimate`] reads the settled onsets and fills in whatever [`Given`] leaves out, as a
//!   [`Timing`]: tempo, meter and downbeat, each an [`Origin`] (given, or estimated with a
//!   confidence). [`beat_series`] gives the tempo from each beat to the next.
//! - [`estimate_key`] gives a melody's [`KeyEstimate`].
//! - [`assemble`] places the events as a `Song` by the timing, and returns a [`Captured`]: the
//!   song, what was done before the downbeat ([`LeadIn`]), and the [`Analysis`] behind it.
//!   [`quantize_offsets`] fills in how far quantizing moved each note ([`QuantizeOffset`]).
//! - [`quantize_apart`] quantizes the song for its score, refining the grid until no two notes
//!   share a step, and [`merge_collisions`] merges what the finest grid still cannot part.
//!
//! Times inside the chain are sample frames at the analysis rate, not musical positions:
//! musical time does not exist until a tempo does. [`Timing`] carries the beat period actually
//! measured, and [`assemble`] converts frames to beats with it; the song states the whole bpm.
//!
//! # The stream rule
//!
//! The stream rule, from the design's anti-goals: "A stage that buffers the entire take to
//! do its work, other than the final assembly into a `Song`, is a design failure."
//! Live input would otherwise be a rewrite. A [`Stage`] takes input a piece at a time, holds it
//! back only until its output is settled, and reports how long that takes
//! ([`Stage::latency_frames`]). [`estimate`] and [`assemble`] are functions over the settled
//! events, not stages: they are the one place the whole take is kept, and only as events.
//!
//! # Where a model fits later
//!
//! This version has no model, no model runtime and no model download. A model such as
//! basic-pitch (Apache-2.0, with an ONNX export, run through `ort`) would be one more
//! implementation of [`Stage`] with `In = Mono` and `Out = Event`: it owns whatever buffering
//! the model's window needs, reports it as its `latency_frames`, and emits [`Event::Note`]s as
//! they settle. It would replace [`Melody`] for polyphonic takes, and [`estimate`] and
//! [`assemble`] would not change. [`Event`] is `#[non_exhaustive]` so a model stage or a
//! streaming consumer can add to it. ONNX is the starting point; if a model ships only
//! PyTorch weights, burn and candle come before a Python sidecar.
//!
//! # Known boundary
//!
//! [`FileSource`] refuses HE-AAC by name when the file's header declares it. HE-AAC signalled
//! only inside the audio data decodes as AAC-LC at half bandwidth, without a refusal; catching
//! it would mean comparing declared and decoded sample rates.

#![forbid(unsafe_code)]

mod assemble;
mod error;
mod event;
mod hits;
mod key;
mod notes;
mod onset;
mod pitch;
mod prepare;
mod score;
mod source;
mod stage;
#[cfg(test)]
mod synth;
mod timing;

pub use assemble::{Analysis, Captured, Kind, LeadIn, QuantizeOffset, assemble, quantize_offsets};
pub use error::{Error, Result};
pub use event::{Event, HitLane};
pub use hits::{Hits, LaneMode};
pub use key::{KeyEstimate, estimate_key};
pub use notes::Melody;
pub use onset::Onsets;
pub use pitch::{PitchFrame, PitchTrack};
pub use prepare::{ANALYSIS_RATE, MAX_GAP_SECS, Prepare};
pub use score::{Merge, Quantized, merge_collisions, quantize_apart};
pub use source::FileSource;
pub use stage::{Chunk, Mono, Source, Stage};
pub use timing::{
    BeatTempo, Given, Origin, Timing, WEAK_CONFIDENCE, beat_series, estimate, felt_quarters,
    is_compound,
};
