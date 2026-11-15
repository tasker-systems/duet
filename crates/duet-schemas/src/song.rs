//! `song`: a [`Song`], as duet-core serializes it; and `drum-grid`: a [`DrumGrid`], as
//! duet-drums serializes it.

use duet_core::Song;
use duet_drums::DrumGrid;

use crate::{Kind, Version};

/// `song` 1.0's body is exactly what `Song` serializes to: what duet's `.json` files held before
/// documents.
impl Kind for Song {
    const NAME: &'static str = "song";
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
