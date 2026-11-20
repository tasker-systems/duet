//! Test audio for `duet capture`: clicks, attacked tones and noise bursts, written as 16-bit
//! mono WAV at the analysis rate, so nothing is resampled on the way in.
//!
//! Every generator is deterministic, so a failure reproduces. Times are in seconds. The files
//! go in a test's scratch directory and are never committed.

use std::f64::consts::TAU;
use std::path::Path;

/// The rate the files are written at: duet-capture's analysis rate.
pub const RATE: u32 = 22_050;

/// Writes `samples` as a 16-bit mono WAV at [`RATE`].
pub fn write_wav(path: &Path, samples: &[f32]) {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_len).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes()); // PCM
    bytes.extend(1u16.to_le_bytes()); // mono
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_len.to_le_bytes());
    for s in samples {
        bytes.extend(((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write wav");
}

/// `secs` of digital silence.
pub fn silence(secs: f64) -> Vec<f32> {
    vec![0.0; frames(secs)]
}

/// `secs` of silence with a single-sample click at each `(time, amplitude)`.
pub fn clicks(secs: f64, clicks: &[(f64, f32)]) -> Vec<f32> {
    let mut out = silence(secs);
    for &(t, amplitude) in clicks {
        out[frames(t)] += amplitude;
    }
    out
}

/// Notes one after another from `lead` seconds in, each `(midi, secs)` a tone of four
/// partials that rises over 5 ms and falls to silence over its last 20 ms, so each is attacked
/// afresh, with a second of silence after.
pub fn notes(lead: f64, notes: &[(u8, f64)]) -> Vec<f32> {
    notes_off(lead, notes, 0.0)
}

/// [`notes`], every one sung `cents` off its key on A440.
pub fn notes_off(lead: f64, notes: &[(u8, f64)], cents: f64) -> Vec<f32> {
    let each: Vec<(u8, f64, f64)> = notes.iter().map(|&(m, s)| (m, s, cents)).collect();
    notes_each_off(lead, &each)
}

/// [`notes`], each `(midi, secs, cents)` sung its own `cents` off A440.
pub fn notes_each_off(lead: f64, notes: &[(u8, f64, f64)]) -> Vec<f32> {
    let mut out = silence(lead);
    for &(midi, secs, cents) in notes {
        let freq = 440.0 * ((f64::from(midi) - 69.0 + cents / 100.0) / 12.0).exp2();
        let len = frames(secs);
        let (attack, release) = (frames(0.005) as f64, frames(0.02) as f64);
        for i in 0..len {
            let t = i as f64 / f64::from(RATE);
            let tone: f64 = (1..=4)
                .map(|n| (TAU * freq * f64::from(n) * t).sin() / f64::from(n))
                .sum::<f64>()
                / (1.0 + 1.0 / 2.0 + 1.0 / 3.0 + 1.0 / 4.0);
            let envelope = (i as f64 / attack).min((len - i) as f64 / release).min(1.0);
            out.push((tone * envelope * 0.5) as f32);
        }
    }
    out.extend(silence(1.0));
    out
}

/// [`notes`], each bent `bend(note, t)` cents off its key at `t` seconds into it, its phase
/// unbroken, so a bend is never attacked afresh.
pub fn notes_bent(lead: f64, notes: &[(u8, f64)], bend: impl Fn(usize, f64) -> f64) -> Vec<f32> {
    let mut out = silence(lead);
    for (note, &(midi, secs)) in notes.iter().enumerate() {
        let len = frames(secs);
        let (attack, release) = (frames(0.005) as f64, frames(0.02) as f64);
        let mut phase = 0.0;
        for i in 0..len {
            let t = i as f64 / f64::from(RATE);
            let freq = 440.0 * ((f64::from(midi) - 69.0 + bend(note, t) / 100.0) / 12.0).exp2();
            phase += TAU * freq / f64::from(RATE);
            let tone: f64 = (1..=4)
                .map(|n| (phase * f64::from(n)).sin() / f64::from(n))
                .sum::<f64>()
                / (1.0 + 1.0 / 2.0 + 1.0 / 3.0 + 1.0 / 4.0);
            let envelope = (i as f64 / attack).min((len - i) as f64 / release).min(1.0);
            out.push((tone * envelope * 0.5) as f32);
        }
    }
    out.extend(silence(1.0));
    out
}

/// `secs` of silence with a burst of noise limited to `low..high` Hz at each `(start, length)`,
/// with a 2 ms attack and a 30 ms decay.
pub fn bursts(secs: f64, bursts: &[(f64, f64)], low: f64, high: f64) -> Vec<f32> {
    const PARTIALS: u32 = 48;
    let mut out = silence(secs);
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
        let (first, count) = (frames(start), frames(length));
        let (attack, decay) = (frames(0.002) as f64, frames(0.03) as f64);
        for i in 0..count {
            let t = (first + i) as f64 / f64::from(RATE);
            let sum: f64 = partials.iter().map(|&(f, p)| (TAU * f * t + p).sin()).sum();
            let envelope = (i as f64 / attack)
                .min((count - 1 - i) as f64 / decay)
                .min(1.0);
            out[first + i] += (sum / f64::from(PARTIALS).sqrt() * envelope * 0.25) as f32;
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

fn frames(secs: f64) -> usize {
    (secs * f64::from(RATE)).round() as usize
}

// --- A FLAC writer, for a file a frame of which the decoder refuses ---

/// FLAC blocks of a clicks take: [`BLOCK`] samples each, `blocks` of them, with a click of
/// amplitude 0.6 every half second from 0.25 s, no click inside `reserved_block`.
pub const BLOCK: usize = 4096;

/// `blocks` blocks of the clicks take as 16-bit samples: 0.5 s a click at 120 bpm means a
/// capture's timing is given, never estimated into ambiguity.
fn click_samples(blocks: usize, reserved_block: usize) -> Vec<i16> {
    let mut out = vec![0i16; blocks * BLOCK];
    let secs = blocks * BLOCK / RATE as usize;
    for n in 0..=2 * secs {
        let at = frames(0.25 + 0.5 * n as f64);
        assert!(
            at / BLOCK != reserved_block,
            "a click fell inside the refused block"
        );
        out[at] = 19_660; // 0.6 of i16's range
    }
    out
}

/// FLAC's CRC-8 (polynomial 0x07), over a frame header.
fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |mut crc, &byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// FLAC's CRC-16 (polynomial 0x8005), over a whole frame.
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |mut crc, &byte| {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                crc << 1 ^ 0x8005
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// Writes `path` as a FLAC of the clicks take whose block number `reserved_block` is replaced
/// by a reserved subframe under a right CRC16: the decoder refuses it, and every later block
/// keeps its true place. Deterministic, a little over `2 × BLOCK × blocks` bytes.
pub fn write_flac_block_refused(path: &Path, blocks: usize, reserved_block: usize) {
    let samples = click_samples(blocks, reserved_block);
    let total = samples.len() as u64;
    let mut out = b"fLaC".to_vec();
    out.extend([0x80, 0x00, 0x00, 34]); // the last block, STREAMINFO, 34 bytes
    out.extend((BLOCK as u16).to_be_bytes());
    out.extend((BLOCK as u16).to_be_bytes());
    out.extend([0, 0, 0, 0, 0, 0]); // frame sizes unknown
    let packed = (u64::from(RATE) << 44) | (15 << 36) | total; // mono, 16 bits a sample
    out.extend(packed.to_be_bytes());
    out.extend([0; 16]); // no MD5
    for (number, block) in samples.chunks(BLOCK).enumerate() {
        let mut frame = vec![0xff, 0xf8, 0xc6, 0x08, number as u8]; // block 4096, 22.05 kHz
        frame.push(crc8(&frame));
        if number == reserved_block {
            frame.extend([0x08, 0xaa, 0x55, 0xaa, 0x55]); // a reserved subframe type
        } else {
            frame.push(0x02); // verbatim
            for &sample in block {
                frame.extend(sample.to_be_bytes());
            }
        }
        let crc = crc16(&frame);
        frame.extend(crc.to_be_bytes());
        out.extend(frame);
    }
    std::fs::write(path, out).expect("write flac");
}
