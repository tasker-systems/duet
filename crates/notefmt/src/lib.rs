//! Convert MIDI to a readable, LLM-editable note format and back, exactly.
//!
//! A [`Song`] holds tempo, meter, key and tracks of [`Note`]s placed at bar.beat.tick
//! positions. It reads and writes Standard MIDI Files ([`Song::from_midi`],
//! [`Song::to_midi`]), a one-note-per-line text format ([`Song::from_text`],
//! [`Song::to_text`]), and anything serde supports, such as JSON and TOML.
//!
//! ```
//! use notefmt::Song;
//!
//! let song = Song::from_text(
//!     "tempo 92\ntime 4/4\nkey D minor\n\n[track \"Piano\" ch1 prog0]\n1.1.0 D4 1 80\n",
//! )?;
//! let midi = song.to_midi()?;
//! assert_eq!(Song::from_midi(&midi)?, song);
//! # Ok::<(), notefmt::Error>(())
//! ```
//!
//! Round-trips are exact to the tick: lengths and tempos are written as the shortest
//! decimal that converts back to the same ticks and microseconds.

#![forbid(unsafe_code)]

mod error;
mod midi;
mod model;
mod pitch;
mod text;
pub mod time;

pub use error::{Error, Result};
pub use model::{Control, KeyChange, Note, Song, TempoChange, TimeChange, Track};
pub use pitch::Pitch;
pub use time::{Key, Position, TimeMap, TimeSig};
