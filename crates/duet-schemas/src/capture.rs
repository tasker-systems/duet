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
///
/// **1.5** measures an estimated tuning's confidence above a flat spread, unchanged in shape:
/// 0 when the steady frames spread flat across the hundred cents, 1 when every one is within
/// 15 cents of the estimate. Before, it was the share within 15 cents, which a flat spread
/// already puts at 0.31; a 1.4 confidence of `s` is `(s − 0.31) / 0.69` in 1.5, at least 0.
///
/// **1.6** adds each note residual's optional `excursions`, also additive: the runs of frames
/// inside a note held somewhere other than where the note is held, each its `start`, its `end`
/// and its `cents` from the note's key. They are measured, never named: a grace, a turn or a
/// trill is worked out from them when something is written. Omitted when a note has none, so a
/// 1.5 document reads as one with no ornament stated.
///
/// **1.7** adds `basic-pitch` to the reading's `reader.name`, also additive: Spotify's Basic
/// Pitch, an outside model, chose where the notes start and end, and the take's performance
/// gave each its key, centre and residual on the take's tuning. Every 1.6 document reads as it
/// did.
impl Kind for Analysis {
    const NAME: &'static str = "capture-analysis";
    const VERSION: Version = Version::new(1, 7);
}
