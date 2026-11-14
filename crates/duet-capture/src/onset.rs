use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::error::{Error, Result};
use crate::event::Event;
use crate::stage::{Mono, Stage};

/// Analysis window, as time: about 23 ms, rounded up to a power of two in samples.
const FRAME_SECS: f64 = 0.023;
/// Hop between windows, as a fraction of the window.
const HOP_DIVISOR: usize = 4;
/// Gain before the log that compresses magnitudes, so quiet partials count beside loud ones.
const COMPRESSION: f32 = 1000.0;
/// History the adaptive threshold averages over, before the candidate peak.
const THRESHOLD_BEFORE_SECS: f64 = 0.1;
/// Lookahead: frames after a candidate that decide whether it is the peak and over the
/// threshold. This is what holds an onset back.
const LOOKAHEAD_SECS: f64 = 0.03;
/// The least spacing between two onsets.
const MIN_GAP_SECS: f64 = 0.05;
/// How far above the local mean the flux must rise, per bin on the compressed scale. Below
/// this nothing is an onset, so silence and steady tones, whose flux is rounding noise, have none.
const THRESHOLD_OFFSET: f32 = 0.03;
/// Where an onset sits in the window whose flux peaks: the click's flux peaks as it enters the
/// window's far end, so the onset is this fraction of the window after the window's start.
const ONSET_POSITION: f64 = 0.75;

/// Finds where sounds start, by spectral flux with an adaptive threshold.
///
/// Each window's flux is the rise in log-compressed magnitude since the window one hop
/// earlier, summed over bins and averaged. A frame is an onset when its flux is the peak of
/// the frames around it and exceeds their local mean by [`THRESHOLD_OFFSET`]. Everything before
/// the stream is taken to be silence, so a take that opens already sounding has an onset at its
/// start, placed exactly at the first input frame rather than at the usual position in the window.
///
/// `strength` is the peak's mean rise per bin, scaled to 0..=1.
///
/// An onset is held back by [`LOOKAHEAD_SECS`] of frames and one window, which
/// [`latency_frames`](Stage::latency_frames) reports. Windows sit at fixed offsets from the
/// first input frame, so the events do not depend on how the stream is cut into chunks.
pub struct Onsets {
    rate: u32,
    frame_len: usize,
    hop: usize,
    before: usize,
    after: usize,
    min_gap: usize,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// The stream's first frame, which windows are numbered from.
    origin: Option<u64>,
    /// Frames received so far.
    received: u64,
    /// Samples not yet dropped, starting at stream offset `buffer_start`.
    buffer: Vec<f32>,
    buffer_start: usize,
    previous: Vec<f32>,
    /// Flux of windows `flux_start..flux_start + flux.len()`.
    flux: Vec<f32>,
    flux_start: usize,
    /// The next window to decide on.
    next_decision: usize,
    last_onset: Option<usize>,
}

impl std::fmt::Debug for Onsets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Onsets")
            .field("rate", &self.rate)
            .field("frame_len", &self.frame_len)
            .field("hop", &self.hop)
            .finish_non_exhaustive()
    }
}

impl Onsets {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32) -> Onsets {
        let frame_len = ((f64::from(rate) * FRAME_SECS).round() as usize).next_power_of_two();
        let hop = frame_len / HOP_DIVISOR;
        let in_hops = |secs: f64| ((secs * f64::from(rate)) / hop as f64).ceil() as usize;
        let window = (0..frame_len)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame_len as f32).cos())
            .collect();
        Onsets {
            rate,
            frame_len,
            hop,
            before: in_hops(THRESHOLD_BEFORE_SECS),
            after: in_hops(LOOKAHEAD_SECS),
            min_gap: in_hops(MIN_GAP_SECS),
            fft: FftPlanner::new().plan_fft_forward(frame_len),
            window,
            origin: None,
            received: 0,
            buffer: Vec::new(),
            buffer_start: 0,
            previous: vec![0.0; frame_len / 2 + 1],
            flux: Vec::new(),
            flux_start: 0,
            next_decision: 0,
            last_onset: None,
        }
    }

    /// Computes the flux of every window the buffer now holds in full.
    fn analyse(&mut self) {
        loop {
            let window_index = self.flux_start + self.flux.len();
            let start = window_index * self.hop - self.buffer_start;
            let Some(samples) = self.buffer.get(start..start + self.frame_len) else {
                break;
            };
            let mut spectrum: Vec<Complex<f32>> = samples
                .iter()
                .zip(&self.window)
                .map(|(s, w)| Complex::new(s * w, 0.0))
                .collect();
            self.fft.process(&mut spectrum);
            let scale = 2.0 / self.frame_len as f32;
            let magnitudes: Vec<f32> = spectrum[..=self.frame_len / 2]
                .iter()
                .map(|c| (1.0 + COMPRESSION * c.norm() * scale).ln())
                .collect();
            let rise: f32 = magnitudes
                .iter()
                .zip(&self.previous)
                .map(|(now, before)| (now - before).max(0.0))
                .sum();
            self.flux.push(rise / magnitudes.len() as f32);
            self.previous = magnitudes;
        }
        // Windows are only ever read from the next hop on.
        let keep_from = (self.flux_start + self.flux.len()) * self.hop;
        let drop = keep_from
            .saturating_sub(self.buffer_start)
            .min(self.buffer.len());
        self.buffer.drain(..drop);
        self.buffer_start += drop;
    }

    /// Decides every window whose lookahead has arrived, or all the rest when `last`.
    fn decide(&mut self, last: bool, out: &mut Vec<Event>) {
        let known = self.flux_start + self.flux.len();
        while self.next_decision < known && (last || self.next_decision + self.after < known) {
            let k = self.next_decision;
            self.next_decision += 1;
            let from = k.saturating_sub(self.before).max(self.flux_start);
            let to = (k + self.after).min(known - 1);
            let around = &self.flux[from - self.flux_start..=to - self.flux_start];
            let value = self.flux[k - self.flux_start];
            let mean = around.iter().sum::<f32>() / around.len() as f32;
            let peak_from = k.saturating_sub(self.after).max(self.flux_start);
            let is_peak = (peak_from..=to).all(|j| {
                let other = self.flux[j - self.flux_start];
                if j < k { other < value } else { other <= value }
            });
            let spaced = self.last_onset.is_none_or(|last| k - last >= self.min_gap);
            if is_peak && spaced && value > mean + THRESHOLD_OFFSET {
                self.last_onset = Some(k);
                out.push(Event::Onset {
                    frame: self.frame_of(k),
                    strength: (value / (1.0 + COMPRESSION * 0.5).ln()).clamp(0.0, 1.0),
                });
            }
        }
        // The threshold looks back `before` windows, and the peak check `after`.
        let keep_from = self.next_decision.saturating_sub(self.before);
        if keep_from > self.flux_start {
            self.flux.drain(..keep_from - self.flux_start);
            self.flux_start = keep_from;
        }
    }

    /// The input frame an onset found at window `k` is placed at.
    fn frame_of(&self, k: usize) -> u64 {
        if k == 0 {
            return self.origin.unwrap_or(0);
        }
        let offset = k * self.hop + (self.frame_len as f64 * ONSET_POSITION) as usize;
        self.origin.unwrap_or(0) + offset as u64
    }
}

impl Stage for Onsets {
    type In = Mono;
    type Out = Event;

    fn push(&mut self, input: Mono, out: &mut Vec<Event>) -> Result<()> {
        if input.rate != self.rate {
            return Err(Error::Invalid(format!(
                "onset detection expects {} Hz, got a chunk at {} Hz",
                self.rate, input.rate
            )));
        }
        let origin = *self.origin.get_or_insert(input.start_frame);
        if input.start_frame != origin + self.received {
            return Err(Error::Invalid(format!(
                "a chunk starts at frame {} where the stream is at frame {}",
                input.start_frame,
                origin + self.received
            )));
        }
        self.received += input.samples.len() as u64;
        self.buffer.extend_from_slice(&input.samples);
        self.analyse();
        self.decide(false, out);
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<Event>) -> Result<()> {
        if self.received > 0 {
            // Fade the take out over its last two hops and pad it with zeros, so its last partial
            // window can be analysed. A hard cut would end a sounding take with a corner whose
            // broadband splatter reads as an onset; a smooth fade does not.
            let last_start = ((self.received - 1) as usize / self.hop) * self.hop;
            let end = last_start + self.frame_len;
            let held = self.buffer_start + self.buffer.len();
            self.buffer
                .resize(self.buffer.len() + end.saturating_sub(held), 0.0);
            let fade = 2 * self.hop;
            let real_end = self.received as usize - self.buffer_start;
            for d in 0..fade.min(real_end) {
                let gain =
                    0.5 - 0.5 * (std::f32::consts::PI * (d + 1) as f32 / (fade + 1) as f32).cos();
                self.buffer[real_end - 1 - d] *= gain;
            }
            self.analyse();
        }
        self.decide(true, out);
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        (self.after * self.hop + self.frame_len) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    const RATE: u32 = 22_050;

    fn run(signal: &[f32], chunk: usize) -> Vec<Event> {
        run_from(signal, chunk, 0)
    }

    fn run_from(signal: &[f32], chunk: usize, origin: u64) -> Vec<Event> {
        let mut stage = Onsets::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(chunk).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: origin + (i * chunk) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        out
    }

    fn frames_of(events: &[Event]) -> Vec<u64> {
        events
            .iter()
            .map(|e| match e {
                Event::Onset { frame, .. } => *frame,
                other => panic!("expected an Onset, got {other:?}"),
            })
            .collect()
    }

    fn assert_near(found: &[u64], expected_secs: &[f64], tolerance_secs: f64) {
        assert_eq!(found.len(), expected_secs.len(), "found {found:?}");
        for (&frame, &secs) in found.iter().zip(expected_secs) {
            let at = frame as f64 / f64::from(RATE);
            assert!(
                (at - secs).abs() <= tolerance_secs,
                "onset at {at} s, wanted {secs} s"
            );
        }
    }

    #[test]
    fn clicks_at_known_times_are_found() {
        let times: Vec<f64> = (0..8).map(|i| 0.5 + 0.5 * f64::from(i)).collect();
        let signal = synth::clicks(RATE, 5.0, &times, 0.8);
        let events = run(&signal, 1024);
        assert_near(&frames_of(&events), &times, 0.010);
    }

    #[test]
    fn silence_has_no_onsets_and_a_steady_tone_has_one_at_its_start() {
        assert!(run(&synth::silence(RATE, 3.0), 1024).is_empty());
        let mut signal = synth::silence(RATE, 0.5);
        signal.extend(synth::tone(RATE, 440.0, 3.0, 3, 0.5));
        assert_near(&frames_of(&run(&signal, 1024)), &[0.5], 0.010);
    }

    #[test]
    fn a_take_that_opens_sounding_has_an_onset_at_its_start() {
        let tone = synth::tone(RATE, 440.0, 1.0, 3, 0.5);
        for origin in [0, 50_000] {
            let events = run_from(&tone, 1024, origin);
            assert_eq!(frames_of(&events), vec![origin], "{events:?}");
        }
    }

    #[test]
    fn a_click_in_the_last_few_milliseconds_is_found() {
        let signal = synth::clicks(RATE, 2.0, &[1.999], 0.8);
        assert_near(&frames_of(&run(&signal, 1024)), &[1.999], 0.010);
    }

    #[test]
    fn onsets_are_the_same_whatever_the_chunking() {
        let times = [0.4, 0.9, 1.5, 2.2];
        let mut signal = synth::clicks(RATE, 3.0, &times, 0.8);
        synth::add(&mut signal, &synth::tone(RATE, 330.0, 3.0, 2, 0.1));
        let reference = run(&signal, 512);
        // The tone sounds from the first frame, so the take opens with an onset.
        assert_eq!(reference.len(), times.len() + 1, "{reference:?}");
        for chunk in [1, 777, 4096] {
            assert_eq!(run(&signal, chunk), reference, "chunk of {chunk}");
        }
    }

    #[test]
    fn a_noise_burst_is_found_at_its_start() {
        let signal =
            synth::noise_bursts(RATE, 2.0, &[(0.5, 0.15), (1.2, 0.15)], 500.0, 4000.0, 0.5);
        let events = run(&signal, 1024);
        assert_near(&frames_of(&events), &[0.5, 1.2], 0.020);
    }

    #[test]
    fn a_quiet_click_over_a_tone_is_found() {
        let mut signal = synth::tone(RATE, 220.0, 2.0, 4, 0.3);
        synth::add(&mut signal, &synth::clicks(RATE, 2.0, &[1.0], 0.4));
        let frames = frames_of(&run(&signal, 1024));
        // The tone sounds from the first frame, so the take opens with an onset.
        assert_eq!(frames[0], 0);
        assert_near(&frames[1..], &[1.0], 0.010);
    }

    #[test]
    fn strength_is_larger_for_a_louder_click() {
        let signal = synth::clicks(RATE, 2.0, &[0.5], 0.2);
        let loud = synth::clicks(RATE, 2.0, &[0.5], 0.9);
        let strength = |events: Vec<Event>| match events[..] {
            [Event::Onset { strength, .. }] => strength,
            ref other => panic!("wanted one onset, got {other:?}"),
        };
        assert!(strength(run(&loud, 1024)) > strength(run(&signal, 1024)));
    }

    #[test]
    fn onsets_keep_the_stream_timeline_when_it_does_not_start_at_zero() {
        let signal = synth::clicks(RATE, 2.0, &[1.0], 0.8);
        let mut stage = Onsets::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(1000).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: 50_000 + (i * 1000) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        let frames = frames_of(&out);
        assert_eq!(frames.len(), 1);
        let offset = frames[0] as f64 - 50_000.0 - f64::from(RATE);
        assert!(offset.abs() <= 0.010 * f64::from(RATE), "{frames:?}");
    }

    #[test]
    fn onsets_are_settled_within_the_reported_latency() {
        let signal = synth::clicks(RATE, 4.0, &[1.0], 0.8);
        let mut stage = Onsets::new(RATE);
        let mut out = Vec::new();
        let mut fed = 0u64;
        for piece in signal.chunks(256) {
            let mono = Mono {
                rate: RATE,
                start_frame: fed,
                samples: piece.to_vec(),
            };
            fed += piece.len() as u64;
            stage.push(mono, &mut out).unwrap();
            if let Some(Event::Onset { frame, .. }) = out.first() {
                assert!(
                    fed <= frame + stage.latency_frames() + 256,
                    "settled at {fed} for an onset at {frame}"
                );
                return;
            }
        }
        panic!("no onset was settled before the end of the stream");
    }

    #[test]
    fn a_chunk_that_skips_ahead_is_refused() {
        let mut stage = Onsets::new(RATE);
        let mono = |start_frame| Mono {
            rate: RATE,
            start_frame,
            samples: vec![0.0; 100],
        };
        stage.push(mono(0), &mut Vec::new()).unwrap();
        assert!(stage.push(mono(500), &mut Vec::new()).is_err());
    }
}
