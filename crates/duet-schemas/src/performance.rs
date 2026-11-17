//! `performance`: a melody take's every settled pitch frame, as duet-dsp makes it, written as a
//! `.duet.jsonl` stream.

use duet_dsp::{FrameBlock, PerformanceHead};
use serde::de::Error as _;

use crate::{Kind, StreamKind, Version};

/// `performance` 1.0's first line holds a [`PerformanceHead`]: the analysis rate, the hop, the
/// sample frame of the first pitch frame, and the pitch unit, `"absolute-cents"`.
///
/// **1.1** adds the head's optional `take`, additive: the audio the frames were read from, as
/// `capture-analysis` 1.13 states it (the decoded samples' digest, rate, channels and frames).
/// Omitted when not stated, so a 1.0 stream reads as one stating no take.
impl Kind for PerformanceHead {
    const NAME: &'static str = "performance";
    const VERSION: Version = Version::new(1, 1);
}

/// Every later line is one [`FrameBlock`]: a run of consecutive frames as the columns `pitch`
/// (`null` when unvoiced), `voicing` and `level`, which have one length. The reader refuses a
/// block whose columns differ.
impl StreamKind for PerformanceHead {
    type Record = FrameBlock;

    fn read_minor_record(minor: u16, record: serde_json::Value) -> serde_json::Result<FrameBlock> {
        let _ = minor;
        let block: FrameBlock = serde_json::from_value(record)?;
        block
            .check()
            .map_err(|e| serde_json::Error::custom(e.to_string()))?;
        Ok(block)
    }
}
