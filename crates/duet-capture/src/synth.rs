//! Test signals: clicks, tones, silence and band-limited noise bursts, at a given rate.
//!
//! Every generator returns mono `f32` samples and is deterministic, so a test failure
//! reproduces. Times are in seconds.

use std::f64::consts::TAU;

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

/// A steady tone of `freq` Hz for `secs`, with `harmonics` partials (1 is a sine) at
/// amplitudes falling as 1/n, scaled so the peak is at most `amplitude`.
pub fn tone(rate: u32, freq: f64, secs: f64, harmonics: u32, amplitude: f32) -> Vec<f32> {
    let norm: f64 = (1..=harmonics).map(|n| 1.0 / f64::from(n)).sum();
    (0..frames(rate, secs))
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let sum: f64 = (1..=harmonics)
                .map(|n| (TAU * freq * f64::from(n) * t).sin() / f64::from(n))
                .sum();
            (sum / norm) as f32 * amplitude
        })
        .collect()
}

/// `secs` of silence with a burst of noise limited to `low..high` Hz at each `(start, length)`
/// in `bursts`, both in seconds. Each burst has a 2 ms linear attack and a 30 ms linear decay, so it ends
/// without a click of its own.
pub fn noise_bursts(
    rate: u32,
    secs: f64,
    bursts: &[(f64, f64)],
    low: f64,
    high: f64,
    amplitude: f32,
) -> Vec<f32> {
    const PARTIALS: u32 = 48;
    const ATTACK_SECS: f64 = 0.002;
    const DECAY_SECS: f64 = 0.03;
    let mut out = silence(rate, secs);
    // Fixed-seed xorshift: the same partials every run.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    for &(start, length) in bursts {
        let partials: Vec<(f64, f64)> = (0..PARTIALS)
            .map(|_| (low + (high - low) * next(), TAU * next()))
            .collect();
        let first = frames(rate, start);
        let count = frames(rate, length);
        let attack = frames(rate, ATTACK_SECS).max(1) as f64;
        let decay = frames(rate, DECAY_SECS).max(1) as f64;
        for i in 0..count {
            let t = (first + i) as f64 / f64::from(rate);
            let sum: f64 = partials.iter().map(|&(f, p)| (TAU * f * t + p).sin()).sum();
            let envelope = (i as f64 / attack)
                .min((count - 1 - i) as f64 / decay)
                .min(1.0);
            out[first + i] += (sum / f64::from(PARTIALS).sqrt() * envelope) as f32 * amplitude;
        }
    }
    out
}

/// Adds `other` into `into`, from its first sample.
pub fn add(into: &mut [f32], other: &[f32]) {
    for (a, b) in into.iter_mut().zip(other) {
        *a += b;
    }
}

fn frames(rate: u32, secs: f64) -> usize {
    (secs * f64::from(rate)).round() as usize
}
