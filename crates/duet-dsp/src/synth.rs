//! Test signals: clicks, tones, vibrato, scoops, note sequences (attacked or legato), fades,
//! silence and band-limited noise bursts, at a given rate.
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

/// A tone around `freq` Hz whose pitch swings `cents` either side of it, `speed` times a second
/// (vibrato), with `harmonics` partials and peak at most `amplitude` as in [`tone`].
pub fn vibrato(
    rate: u32,
    freq: f64,
    secs: f64,
    cents: f64,
    speed: f64,
    harmonics: u32,
    amplitude: f32,
) -> Vec<f32> {
    swept(rate, secs, harmonics, amplitude, |t| {
        freq * (cents / 1200.0 * (TAU * speed * t).sin()).exp2()
    })
}

/// Notes one after another, each `(freq, secs)`, played legato: one tone whose pitch slides
/// from note to note over `glide` seconds, ending at each boundary, with its phase unbroken, so
/// nothing is attacked afresh. It rises over its first 5 ms and falls over its last 20 ms, as
/// [`note_sequence`]'s notes do.
pub fn legato(
    rate: u32,
    notes: &[(f64, f64)],
    glide: f64,
    harmonics: u32,
    amplitude: f32,
) -> Vec<f32> {
    let secs: f64 = notes.iter().map(|n| n.1).sum();
    let at = |t: f64| {
        let mut end = 0.0;
        for (i, &(freq, length)) in notes.iter().enumerate() {
            end += length;
            if t < end {
                let Some(&(next, _)) = notes.get(i + 1) else {
                    return freq;
                };
                let into = ((t - (end - glide)) / glide).clamp(0.0, 1.0);
                return freq * (next / freq).powf(into);
            }
        }
        notes.last().map_or(0.0, |n| n.0)
    };
    let mut out = swept(rate, secs, harmonics, amplitude, at);
    envelope(rate, &mut out);
    out
}

/// A tone at `freq` Hz that scoops up to it: it starts `cents` flat and rises in a straight line
/// over the first `fraction` of the note, then holds.
pub fn scoop(
    rate: u32,
    freq: f64,
    secs: f64,
    cents: f64,
    fraction: f64,
    harmonics: u32,
    amplitude: f32,
) -> Vec<f32> {
    swept(rate, secs, harmonics, amplitude, |t| {
        let below = cents * (1.0 - t / (fraction * secs)).max(0.0);
        freq * (-below / 1200.0).exp2()
    })
}

/// A tone around `freq` Hz whose pitch at `t` seconds is `cents(t)` from it, with its phase
/// unbroken, so a change of pitch is never attacked afresh, and `harmonics` partials and peak at
/// most `amplitude` as in [`tone`]. It rises over its first 5 ms and falls over its last 20 ms.
pub fn contour(
    rate: u32,
    freq: f64,
    secs: f64,
    cents: impl Fn(f64) -> f64,
    harmonics: u32,
    amplitude: f32,
) -> Vec<f32> {
    let mut out = swept(rate, secs, harmonics, amplitude, |t| {
        freq * (cents(t) / 1200.0).exp2()
    });
    envelope(rate, &mut out);
    out
}

/// Raises `signal` from silence in a straight line over its first `secs`: a soft entry, with no
/// attack for an onset to find.
pub fn fade_in(rate: u32, signal: &mut [f32], secs: f64) {
    let ramp = frames(rate, secs).max(1);
    for (i, s) in signal.iter_mut().take(ramp).enumerate() {
        *s *= i as f32 / ramp as f32;
    }
}

/// A tone whose frequency at `t` seconds is `freq(t)`, with its phase integrated so a change of
/// pitch is not a break, and `harmonics` partials and peak at most `amplitude` as in [`tone`].
fn swept(
    rate: u32,
    secs: f64,
    harmonics: u32,
    amplitude: f32,
    freq: impl Fn(f64) -> f64,
) -> Vec<f32> {
    let norm: f64 = (1..=harmonics).map(|n| 1.0 / f64::from(n)).sum();
    let mut phase = 0.0;
    (0..frames(rate, secs))
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let sum: f64 = (1..=harmonics)
                .map(|n| (phase * f64::from(n)).sin() / f64::from(n))
                .sum();
            phase += TAU * freq(t) / f64::from(rate);
            (sum / norm) as f32 * amplitude
        })
        .collect()
}

/// Shapes a note: a 5 ms rise and a 20 ms fall to silence.
fn envelope(rate: u32, note: &mut [f32]) {
    let attack = frames(rate, 0.005) as f32;
    let release = frames(rate, 0.02) as f32;
    let len = note.len();
    for (i, s) in note.iter_mut().enumerate() {
        *s *= (i as f32 / attack).min((len - i) as f32 / release).min(1.0);
    }
}

/// Notes one after another, each `(freq, secs)` a [`tone`] that rises over its first 5 ms and
/// falls to silence over its last 20 ms, so each note is attacked afresh, as a plucked or sung
/// note is, even when it repeats the pitch before it.
pub fn note_sequence(rate: u32, notes: &[(f64, f64)], harmonics: u32, amplitude: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for &(freq, secs) in notes {
        let mut note = tone(rate, freq, secs, harmonics, amplitude);
        envelope(rate, &mut note);
        out.extend(note);
    }
    out
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
