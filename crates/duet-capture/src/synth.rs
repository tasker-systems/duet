//! Test signals for assembly's tests: silence and clicks, at a given rate.
//!
//! The full set of generators is `duet-dsp`'s, private to its tests; this keeps the two that
//! assembly's tests need, so neither crate exports test helpers. Times are in seconds.

/// `secs` of digital silence.
pub fn silence(rate: u32, secs: f64) -> Vec<f32> {
    vec![0.0; frames(rate, secs)]
}

/// `secs` of silence with a single-sample impulse of `amplitude` at each of `times`.
///
/// A click has a flat spectrum, so it is a pure onset with no pitch.
pub fn clicks(rate: u32, secs: f64, times: &[f64], amplitude: f32) -> Vec<f32> {
    let mut out = silence(rate, secs);
    for &t in times {
        out[frames(rate, t)] = amplitude;
    }
    out
}

/// The whole number of frames nearest `secs`.
fn frames(rate: u32, secs: f64) -> usize {
    (secs * f64::from(rate)).round() as usize
}
