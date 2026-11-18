//! `part`: a [`Part`], as duet-core serializes it; and `drum-grid`: a [`DrumGrid`], as
//! duet-drums serializes it.

use duet_core::Part;
use duet_drums::DrumGrid;

use crate::{Kind, Version};

/// `part` 1.0's body is exactly what `Part` serializes to: what duet's `.json` files held before
/// documents.
///
/// Every pitch keeps its spelling: a letter, up to two sharps or two flats, and the letter's
/// octave, so `Cb5` (MIDI 71), `E#4`, `F##4` and `Bbb3` are written and read as spelled. The
/// pitch pattern refuses what no spelling keeps (`C###4`, `C#b4`). This came into 1.0 in place,
/// with no minor: before a first release a format changes in place, since no file exists to
/// keep (the owner's ruling 12 in decision 01a1eb48, 2026-11-17).
impl Kind for Part {
    const NAME: &'static str = "part";
    const VERSION: Version = Version::new(1, 0);
}

/// `drum-grid` 1.0's body is what `DrumGrid` serializes to: the conductor, the grid, channel,
/// levels and lanes, then `bar_steps` and `vel`, one lanes × steps tensor for the whole grid.
///
/// A body is checked as it is read ([`DrumGrid::check`]): a `vel` whose shape disagrees with
/// the lanes or the bars, or bar steps the meter does not give, are refused by name.
impl Kind for DrumGrid {
    const NAME: &'static str = "drum-grid";
    const VERSION: Version = Version::new(1, 0);

    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        let grid: DrumGrid = serde_json::from_value(body)?;
        grid.check().map_err(serde::de::Error::custom)?;
        Ok(grid)
    }
}
