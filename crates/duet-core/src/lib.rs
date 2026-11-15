//! The core of duet: the [`Song`] every format converts through, and the notefmt text.
//!
//! A [`Song`] holds tempo, meter, key and tracks of [`Note`]s placed at bar.beat.tick
//! positions. It reads and writes notefmt, a one-note-per-line text format
//! ([`Song::from_text`], [`Song::to_text`]), and anything serde supports, such as JSON and
//! TOML. MIDI files are read and written by the `duet-midi` crate.
//!
//! ```
//! use duet_core::Song;
//!
//! let text = "tempo 92\ntime 4/4\nkey D minor\nend 2.1.0\n\n[track \"Piano\" ch1 prog0]\n1.1.0 D4 1 80\n";
//! let song = Song::from_text(text)?;
//! assert_eq!(Song::from_text(&song.to_text())?, song);
//! # Ok::<(), duet_core::Error>(())
//! ```
//!
//! Quantizing is a separate, explicit step: [`Song::quantized`] returns a new song on a
//! [`Grid`] and leaves the original as it was.
//!
//! Round-trips are exact to the tick: lengths and tempos are written as the shortest
//! decimal that converts back to the same ticks and microseconds.

#![forbid(unsafe_code)]

mod error;
mod model;
mod pitch;
mod quantize;
mod schema;
mod tensor;
mod text;
pub mod time;

pub use error::{Error, Result};
pub use model::{Control, EndRule, KeyChange, Note, Song, TempoChange, TimeChange, Track};
pub use pitch::Pitch;
pub use quantize::Grid;
pub use tensor::{Dtype, Element, Tensor};
pub use time::{Key, Mode, Position, TimeMap, TimeSig};
