//! `capture-analysis`: everything `duet capture` found in a take, as duet-capture's [`Analysis`]
//! serializes it.

use duet_capture::Analysis;

use crate::{Kind, Version};

/// `capture-analysis` 1.0's body is exactly what [`Analysis`] serializes to: the take's rate, its
/// every event, the timing with each value given or estimated, a melody's key with its evidence,
/// the tempo beat by beat, the grid the score was quantized to, and how far quantizing moved each
/// note. Absent values are left out, never written `null`.
///
/// **1.1** adds the optional `reading`: a melody's tuning and its notes as read on it. It is
/// additive, so a 1.0 document reads as one with no reading.
///
/// **1.2** adds the residual, also additive: each read note's optional `residual` (centre,
/// drift, vibrato, slide, fall and rest) and the reading's optional `residual` (the centres'
/// histogram and slope, the read and unread frames, and the loss as its three terms).
///
/// **1.3** adds the reading's optional `reader`, also additive: which reader chose its notes
/// (`rules` or `hmm`) and the cost per note `c` it chose them with, which the loss's `notes`
/// term counts. A 1.2 document's reading has none, and was read by the rules.
///
/// **1.4** prices the loss as the note HMM decodes, also additive in shape: the loss gains a
/// `voicing` term (what sounding the frames a note covers costs beyond leaving them out), taken
/// as 0 in a 1.3 document. Two terms are priced anew: `unread` costs each voiced frame by its
/// voicing, not 1, and `pitch` is `d²/2σ²` a held frame and a flat attack cost a slide frame,
/// not `r²/σ²` a frame. So a 1.3 loss and a 1.4 loss are not to be compared term by term.
impl Kind for Analysis {
    const NAME: &'static str = "capture-analysis";
    const VERSION: Version = Version::new(1, 4);
}
