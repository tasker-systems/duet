//! duet's audio analysis: samples in, events out. Pure DSP, with no I/O, codecs or formats.
//!
//! This crate is the analysis half of `duet capture`. Reading a file (Symphonia) and
//! assembling the events into a `Song` are `duet-capture`'s, which depends on this crate and
//! re-exports what it needs from it.
//!
//! # The chain
//!
//! ```text
//! Chunk --> Prepare --Mono--> Melody | Hits --Event--> estimate --Timing--> (assembly)
//!                               |
//!                               +-- Performance --Layer1--> (the performance, frames and onsets)
//! ```
//!
//! - A source of audio yields [`Chunk`]s of interleaved samples at any rate and channel count.
//! - [`Prepare`] downmixes to one channel and resamples to [`ANALYSIS_RATE`], so every stage
//!   after it can assume both. Its output is [`Mono`].
//! - [`Performance`] (pitch tracking by [`PitchTrack`], onsets by [`Onsets`]) keeps every
//!   settled pitch frame, as a [`PerformanceHead`] and [`FrameBlock`]s in absolute cents
//!   ([`absolute_cents`]), beside its onsets: the performance of a melody take, before any
//!   reading of it. This crate only makes them; duet-schemas writes them as a document.
//! - [`Melody`] (a [`Performance`], segmented into notes) and [`Hits`] (onsets, each given a
//!   lane by [`LaneMode`]) turn mono audio into [`Event`]s: [`Event::Note`], [`Event::Hit`]
//!   and [`Event::Onset`], with [`HitLane`] naming a hit's lane.
//! - [`estimate`] reads the settled onsets and fills in whatever [`Given`] leaves out, as a
//!   [`Timing`]: tempo, meter and downbeat, each an [`Origin`] (given, or estimated with a
//!   confidence). [`beat_series`] gives the tempo from each beat to the next.
//! - [`estimate_key`] gives a melody's [`KeyEstimate`].
//!
//! Times inside the chain are sample frames at the analysis rate, not musical positions:
//! musical time does not exist until a tempo does. [`Timing`] carries the beat period actually
//! measured, which assembly converts frames to beats with.
//!
//! # The stream rule
//!
//! The stream rule, from the design's anti-goals: "A stage that buffers the entire take to
//! do its work, other than the final assembly into a `Song`, is a design failure."
//! Live input would otherwise be a rewrite. A [`Stage`] takes input a piece at a time, holds it
//! back only until its output is settled, and reports how long that takes
//! ([`Stage::latency_frames`]). [`estimate`] and [`estimate_key`] are functions over the
//! settled events, not stages: like assembly, they keep the whole take only as events.
//!
//! # Where a model fits
//!
//! With the `basic-pitch` feature, Spotify's Basic Pitch is a reader (`BasicPitch`): it reads a
//! monophonic take's samples, chooses where its notes start and end, and the performance keys
//! them, as for the other readers. Its model is embedded and nothing is downloaded. Without the
//! feature there is no model and no model runtime.
//!
//! A model that replaces the melody stage, for a polyphonic take, would be one more
//! implementation of [`Stage`] with `In = Mono` and `Out = Event`: it owns whatever buffering
//! the model's window needs, reports it as its `latency_frames`, and emits [`Event::Note`]s as
//! they settle. It would replace [`Melody`] for polyphonic takes, and [`estimate`] and
//! assembly would not change. [`Event`] is `#[non_exhaustive]` so a model stage or a
//! streaming consumer can add to it. ONNX is the starting point; if a model ships only
//! PyTorch weights, burn and candle come before a Python sidecar.

#![forbid(unsafe_code)]

#[cfg(feature = "basic-pitch")]
mod basic_pitch;
mod error;
mod event;
mod hits;
mod hmm;
mod key;
mod notes;
mod onset;
mod performance;
mod pitch;
mod prepare;
mod reading;
mod rejoin;
mod residual;
mod stage;
#[cfg(test)]
mod synth;
mod timing;

#[cfg(feature = "basic-pitch")]
pub use basic_pitch::{BasicPitch, BasicPitchRead, MODEL_SHA256};
pub use error::{Error, Result};
pub use event::{Event, HitLane};
pub use hits::{Hits, LaneMode};
pub use hmm::{
    ATTACK_COST, DROPOUT, LAG_SECS, NoteModel, ONSET_LEAVE, STABLE_SIGMA_SEMITONES, VOICING_FLOOR,
};
pub use key::{KeyEstimate, estimate_key};
pub use notes::Melody;
pub use onset::Onsets;
pub use performance::{
    FrameBlock, Layer1, Performance, PerformanceHead, PitchUnit, absolute_cents,
};
pub use pitch::{PitchFrame, PitchTrack};
pub use prepare::{ANALYSIS_RATE, MAX_GAP_SECS, Prepare};
pub use reading::{
    A440_CENTS, BETWEEN_KEYS_CENTS, PerformanceFrames, ReadNote, Reader, ReaderName, Reading,
    Tuning, a4_cents_of_hz, estimate_tuning, read, read_as_decoded,
};
pub use rejoin::{Dip, REJOIN_CENTS, REJOIN_TOLERANCE_CENTS, Rejoin, rejoin};
pub use residual::{
    CentreHistogram, Drift, EXCURSION_CENTS, Excursion, LOSS_SIGMA_CENTS, Loss, Modulation,
    NOTE_COST, NoteResidual, SHORTEST_EXCURSION_FRAMES, TakeResidual, Transition, held_cost,
    residual, residual_with_cost, silent_cost, sounding_cost, unread_cost,
};
pub use stage::{Chunk, Mono, Stage};
pub use timing::{
    BeatTempo, Given, Origin, TempoAlternatives, Timing, WEAK_CONFIDENCE, beat_series, estimate,
    felt_quarters, is_compound,
};
