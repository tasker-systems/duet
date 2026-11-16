//! duet capture: audio in, analysis events out, and a `Song` assembled from them.
//!
//! This crate holds the ends of the chain: reading a file ([`FileSource`], through Symphonia)
//! and assembling the settled events into a `Song` ([`assemble`], [`quantize_apart`]). The
//! analysis between them, from [`Prepare`] to [`estimate`] and [`estimate_key`], is pure DSP and
//! lives in `duet-dsp`; its types are re-exported here, so a caller needs only this crate.
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
//! [`FileSource`], [`Source`], [`assemble`] and what it returns, and the score are this crate's;
//! everything else above is `duet-dsp`'s.
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
//! `duet-dsp`'s documentation says where a model fits in the chain later.
//!
//! # Writing a take
//!
//! [`TakeWriter`] is the other direction: samples from a device written to a FLAC file a frame
//! at a time, losslessly at a [`Depth`], so `duet record` leaves a take [`FileSource`] reads
//! like any other.
//!
//! # Errors
//!
//! [`Error`] is this crate's: reading, decoding and assembling. The stages and [`estimate`]
//! return `duet-dsp`'s error, [`duet_dsp::Error`], which converts into [`Error::Analysis`] with
//! its message unchanged.
//!
//! # Known boundary
//!
//! [`FileSource`] refuses HE-AAC by name when the file's header declares it. HE-AAC signalled
//! only inside the audio data decodes as AAC-LC at half bandwidth, without a refusal; catching
//! it would mean comparing declared and decoded sample rates.

#![forbid(unsafe_code)]

mod assemble;
mod error;
mod score;
mod source;
#[cfg(test)]
mod synth;
mod take;

pub use assemble::{Analysis, Captured, Kind, LeadIn, QuantizeOffset, assemble, quantize_offsets};
pub use duet_dsp::{
    A440_CENTS, ANALYSIS_RATE, BETWEEN_KEYS_CENTS, BeatTempo, Chunk, Event, FrameBlock, Given,
    HitLane, Hits, KeyEstimate, LaneMode, Layer1, Loss, MAX_GAP_SECS, Melody, Mono, NOTE_COST,
    NoteModel, Onsets, Origin, Performance, PerformanceFrames, PerformanceHead, PitchFrame,
    PitchTrack, PitchUnit, Prepare, ReadNote, Reader, ReaderName, Reading, Stage,
    TempoAlternatives, Thresholds, Timing, Tuning, WEAK_CONFIDENCE, a4_cents_of_hz, absolute_cents,
    beat_series, estimate, estimate_key, estimate_tuning, felt_quarters, is_compound, read,
};
#[cfg(feature = "basic-pitch")]
pub use duet_dsp::{BasicPitch, BasicPitchRead, MODEL_SHA256};
pub use duet_dsp::{Dip, REJOIN_CENTS, Rejoin, read_as_decoded, rejoin};
pub use error::{Error, Result};
pub use score::{Merge, Quantized, merge_collisions, quantize_apart};
pub use source::{FileSource, Source};
pub use take::{Depth, TakeWriter, Written};
