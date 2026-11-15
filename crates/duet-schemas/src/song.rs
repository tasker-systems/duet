//! `song`: a [`Song`], as duet-core serializes it.

use duet_core::Song;

use crate::{Kind, Version};

/// `song` 1.0's body is exactly what `Song` serializes to: what duet's `.json` files held before
/// documents.
impl Kind for Song {
    const NAME: &'static str = "song";
    const VERSION: Version = Version::new(1, 0);
}
