//! The duet drum grid: a drum part as a lanes × steps velocity matrix.
//!
//! A [`DrumGrid`] has the conductor of a [`duet_core::Part`] (resolution, tempo, meter
//! and their changes), a step such as `1/16`, a list of named lanes each playing one
//! MIDI key, and, for every bar, a velocity per lane and step. It has two forms:
//!
//! - **Exact:** the matrix itself, through serde: the body of a drum-grid document, which
//!   duet-schemas reads and writes.
//! - **Text:** one row per lane and one cell per step, for people and LLMs to read and
//!   edit ([`DrumGrid::from_text`], [`DrumGrid::to_text`]). A cell is `.`, `g`, `x` or
//!   `X`, so the text holds three velocity levels; writing other velocities as text is
//!   refused until they are snapped to the levels, which is a step of its own
//!   ([`DrumGrid::snapped_to_levels`]).
//!
//! Lane names come from a [`LaneMap`]. The built-in `gm` map covers every General MIDI
//! Level 1 percussion key (35 to 81) on channel 10.
//!
//! A grid converts to a part ([`DrumGrid::to_part`]) and back ([`DrumGrid::from_part`])
//! exactly: grid → part → grid gives the same grid, as long as its lanes are in the map's
//! row order and each is played at least once. No bar count is needed: when a grid ends
//! in empty bars, or inside its last bar ([`DrumGrid::end`]), the part names that end.
//! Reading a part never moves or alters a note to make it fit; what does not fit is
//! refused by name.
//!
//! ```
//! use duet_drums::{DrumGrid, LaneMap};
//!
//! let text = "grid 1/8\n\n[bars 1]\nsnare | . . x . . . x . |\nkick  | x . . . x . . . |\n";
//! let grid = DrumGrid::from_text(text)?;
//! let part = grid.to_part()?;
//! let back = DrumGrid::from_part(&part, grid.grid, &LaneMap::gm(), None, None)?;
//! assert_eq!(back, grid);
//! assert_eq!(DrumGrid::from_text(&grid.to_text()?)?, grid);
//! # Ok::<(), duet_drums::Error>(())
//! ```

#![forbid(unsafe_code)]

mod error;
mod grid;
mod lanes;
mod part;
mod text;

pub use error::{Error, Result};
pub use grid::{DrumGrid, Lane, Levels};
pub use lanes::{LaneDef, LaneMap, LanePatch};
pub use part::DRUM_TRACK;
