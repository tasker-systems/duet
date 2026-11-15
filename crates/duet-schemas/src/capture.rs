//! `capture-analysis`: everything `duet capture` found in a take, as duet-capture's [`Analysis`]
//! serializes it.

use duet_capture::Analysis;

use crate::{Kind, Version};

/// `capture-analysis` 1.0's body is exactly what [`Analysis`] serializes to: the take's rate, its
/// every event, the timing with each value given or estimated, a melody's key with its evidence,
/// the tempo beat by beat, the grid the score was quantized to, and how far quantizing moved each
/// note. Absent values are left out, never written `null`.
impl Kind for Analysis {
    const NAME: &'static str = "capture-analysis";
    const VERSION: Version = Version::new(1, 0);
}
