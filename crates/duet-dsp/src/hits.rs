use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::error::Result;
use crate::event::{Event, HitLane};
use crate::onset::Onsets;
use crate::stage::{Mono, Stage};

/// How much of the sound before an onset the classifier looks at. An onset is placed a little
/// after a sound's true start, so this takes in the attack.
const BEFORE_SECS: f64 = 0.01;
/// How much of the sound from an onset on the classifier looks at. The onset stage settles an
/// onset only once at least this much audio has arrived after it, so the window is always whole.
const AFTER_SECS: f64 = 0.03;
/// Below this, energy counts as the low band, which tells a kick. Task 9 should tune it.
const LOW_BAND_MAX_HZ: f64 = 200.0;
/// Below this, energy is rumble and DC offset and counts toward no band.
const RUMBLE_MAX_HZ: f64 = 30.0;
/// The least share of the window's energy in the low band for a hit to be a kick. Task 9 should
/// tune it: a real beatboxed kick has a body in the low band and a click above it.
const KICK_LOW_SHARE: f64 = 0.5;
/// The least spectral centroid, in Hz, of the energy above the low band, for a hit that is not
/// a kick to be a hat; anything
/// lower is a snare. Task 9 should tune it: a snare's noise reaches into the highs, a closed
/// hat's centre sits above it.
const HAT_CENTROID_HZ: f64 = 5_000.0;

/// Below this, energy counts toward a voiced kick: a sung "un" has its energy in the voice's
/// range, from about 100 Hz to here, and little in the low band once a phone's microphone has
/// rolled its fundamental off.
const VOICED_MAX_HZ: f64 = 1_000.0;
/// The least share of the window's energy below `VOICED_MAX_HZ` for a voiced hit to be a kick. A
/// snare's or hat's noise sits mostly above it. The voiced-kick constants are fitted to a
/// synthetic "un", not yet to a real take: tune them with the owner's.
const VOICED_KICK_SHARE: f64 = 0.8;
/// The lowest pitch, in Hz, a voiced kick is sung at.
const VOICED_LEAST_HZ: f64 = 60.0;
/// The highest pitch, in Hz, a voiced kick is sung at: a low hum, below most sung melody.
const VOICED_MOST_HZ: f64 = 300.0;
/// The least normalised autocorrelation of the window, at a lag in the voiced kick's pitch range,
/// for it to count as voiced. Noise limited to below `VOICED_MAX_HZ` has lost its likeness at
/// those lags; a held tone keeps it.
const VOICED_LEAST_CORRELATION: f64 = 0.6;

/// How much of the window just before a hit is taken away from the window around it. Above 1,
/// so the scatter between two looks at a decaying tail does not leave it standing. Task 9
/// should tune it.
const EARLIER_WEIGHT: f64 = 2.0;

/// What decides the lane of a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneMode {
    /// Every hit goes to the lane the user asked for ([`HitLane::Asked`]). Resolving which
    /// lane that is happens when the take is assembled.
    Asked,
    /// Every hit is told apart by sound, as a beatboxed kick, snare or closed hat.
    Beatbox,
}

/// Drum hits from a take: onset detection, each onset given a lane.
///
/// It runs [`Onsets`] over the input and emits every [`Event::Onset`], as it settles, and an
/// [`Event::Hit`] for each, at the same frame and with the same strength, so a later stage can
/// take tempo from the onsets of a rhythm take as it does from a melody's.
///
/// In [`LaneMode::Asked`] a hit's lane is [`HitLane::Asked`]. In [`LaneMode::Beatbox`] it is
/// chosen from the energy the 40 ms around the onset (`BEFORE_SECS` and `AFTER_SECS`) add to
/// the 40 ms before them (`EARLIER_WEIGHT`), per bin of a Hann-windowed spectrum:
///
/// - **Kick** when the share of energy below `LOW_BAND_MAX_HZ` is at least `KICK_LOW_SHARE`;
/// - otherwise **Kick** when it is a voiced kick, a sung "un": the share of energy below
///   `VOICED_MAX_HZ` is at least `VOICED_KICK_SHARE`, and the window is voiced, its normalised
///   autocorrelation reaching `VOICED_LEAST_CORRELATION` at a lag for a pitch between
///   `VOICED_LEAST_HZ` and `VOICED_MOST_HZ`;
/// - otherwise **Hat** when the spectral centroid is at least `HAT_CENTROID_HZ`;
/// - otherwise **Snare**.
///
/// A window with no energy at all is a Snare. Energy below `RUMBLE_MAX_HZ` is ignored. A hit's
/// lane depends only on the audio in its window and the one before it, so the events do not
/// depend on how the stream is cut into chunks.
pub struct Hits {
    mode: LaneMode,
    onsets: Onsets,
    rate: u32,
    before: u64,
    after: u64,
    fft: Arc<dyn Fft<f32>>,
    taper: Vec<f32>,
    /// The stream's first frame and the frames received since.
    origin: Option<u64>,
    received: u64,
    /// Samples from stream frame `buffer_start` on.
    buffer: VecDeque<f32>,
    buffer_start: u64,
    /// Onsets whose window has not fully arrived: `(frame, strength)`.
    waiting: VecDeque<(u64, f32)>,
}

impl std::fmt::Debug for Hits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hits")
            .field("mode", &self.mode)
            .field("onsets", &self.onsets)
            .finish_non_exhaustive()
    }
}

impl Hits {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32, mode: LaneMode) -> Hits {
        let frames = |secs: f64| (secs * f64::from(rate)).round() as u64;
        let (before, after) = (frames(BEFORE_SECS), frames(AFTER_SECS));
        let len = (before + after) as usize;
        let taper = (0..len)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * (i as f32 + 0.5) / len as f32).cos())
            .collect();
        Hits {
            mode,
            onsets: Onsets::new(rate),
            rate,
            before,
            after,
            fft: FftPlanner::new().plan_fft_forward(len.next_power_of_two()),
            taper,
            origin: None,
            received: 0,
            buffer: VecDeque::new(),
            buffer_start: 0,
            waiting: VecDeque::new(),
        }
    }

    /// The sample at stream frame `frame`: silence outside what has been received.
    fn sample(&self, frame: i64) -> f32 {
        usize::try_from(frame - self.buffer_start as i64)
            .ok()
            .and_then(|i| self.buffer.get(i))
            .copied()
            .unwrap_or(0.0)
    }

    /// The power spectrum of the window of `taper.len()` samples starting at stream frame `start`.
    fn power(&self, start: i64) -> Vec<f64> {
        let size = self.fft.len();
        let mut spectrum: Vec<Complex<f32>> = (0..size)
            .map(|i| match self.taper.get(i) {
                Some(w) => Complex::new(self.sample(start + i as i64) * w, 0.0),
                None => Complex::new(0.0, 0.0),
            })
            .collect();
        self.fft.process(&mut spectrum);
        spectrum[..=size / 2]
            .iter()
            .map(|c| f64::from(c.norm_sqr()))
            .collect()
    }

    /// The lane of a hit at `frame`, from the energy it adds: the spectrum of the window around
    /// it less that of the window just before, never below zero. The low band is taken as a
    /// whole, the bins above it one by one. A hit close behind a louder one is then not read as
    /// the tail of the one before.
    fn classify(&self, frame: u64) -> HitLane {
        let start = frame as i64 - self.before as i64;
        let now = self.power(start);
        let earlier = self.power(start - self.taper.len() as i64);
        let bin_hz = f64::from(self.rate) / self.fft.len() as f64;
        // The low band is judged as a whole, since a tail beats from one look to the next in
        // each of its bins; the bins above it, which no kick reaches, one by one.
        let (mut low_now, mut low_earlier) = (0.0, 0.0);
        let (mut high, mut weighted, mut voice) = (0.0, 0.0, 0.0);
        for (k, (now, earlier)) in now.iter().zip(&earlier).enumerate() {
            let hz = k as f64 * bin_hz;
            if hz < RUMBLE_MAX_HZ {
                continue;
            }
            if hz < LOW_BAND_MAX_HZ {
                low_now += now;
                low_earlier += earlier;
            } else {
                let power = (now - earlier * EARLIER_WEIGHT).max(0.0);
                high += power;
                weighted += power * hz;
                if hz < VOICED_MAX_HZ {
                    voice += power;
                }
            }
        }
        let low = (low_now - low_earlier * EARLIER_WEIGHT).max(0.0);
        let total = low + high;
        if total <= 0.0 {
            return HitLane::Snare;
        }
        if low / total >= KICK_LOW_SHARE
            || ((low + voice) / total >= VOICED_KICK_SHARE && self.voiced(start))
        {
            HitLane::Kick
        } else if weighted / high.max(f64::MIN_POSITIVE) >= HAT_CENTROID_HZ {
            HitLane::Hat
        } else {
            HitLane::Snare
        }
    }

    /// Whether the window of `taper.len()` samples from stream frame `start` holds a held pitch
    /// in the voiced kick's range: its normalised autocorrelation, at some lag for a pitch from
    /// `VOICED_MOST_HZ` down to `VOICED_LEAST_HZ`, is at least `VOICED_LEAST_CORRELATION`.
    fn voiced(&self, start: i64) -> bool {
        let window: Vec<f64> = (0..self.taper.len() as i64)
            .map(|i| f64::from(self.sample(start + i)))
            .collect();
        let rate = f64::from(self.rate);
        let shortest = (rate / VOICED_MOST_HZ).floor() as usize;
        let longest = ((rate / VOICED_LEAST_HZ).ceil() as usize).min(window.len() / 2);
        (shortest.max(1)..=longest).any(|lag| {
            let (head, tail) = (&window[..window.len() - lag], &window[lag..]);
            let cross: f64 = head.iter().zip(tail).map(|(a, b)| a * b).sum();
            let energy =
                head.iter().map(|a| a * a).sum::<f64>() * tail.iter().map(|b| b * b).sum::<f64>();
            energy > 0.0 && cross / energy.sqrt() >= VOICED_LEAST_CORRELATION
        })
    }

    fn emit_hit(&self, frame: u64, strength: f32, out: &mut Vec<Event>) {
        let lane = match self.mode {
            LaneMode::Asked => HitLane::Asked,
            LaneMode::Beatbox => self.classify(frame),
        };
        out.push(Event::Hit {
            frame,
            lane,
            strength,
        });
    }

    /// Emits the hit of every waiting onset whose window has fully arrived, or all when `last`.
    fn resolve(&mut self, last: bool, out: &mut Vec<Event>) {
        let end = self.origin.unwrap_or(0) + self.received;
        while let Some((frame, strength)) = self
            .waiting
            .pop_front_if(|&mut (f, _)| last || f + self.after <= end)
        {
            self.emit_hit(frame, strength, out);
        }
        // Keep what the next waiting onset needs, back to the start of the window before its own,
        // and what an onset not yet settled will: one lies no further back than the onset
        // stage's latency.
        let unsettled = end.saturating_sub(self.onsets.latency_frames());
        let keep_from = self
            .waiting
            .front()
            .map_or(unsettled, |w| w.0.min(unsettled));
        let drop = keep_from
            .saturating_sub(self.before + self.taper.len() as u64)
            .saturating_sub(self.buffer_start)
            .min(self.buffer.len() as u64);
        self.buffer.drain(..drop as usize);
        self.buffer_start += drop;
    }

    /// Passes each onset on to `out` and queues its hit.
    fn take_onsets(&mut self, events: Vec<Event>, out: &mut Vec<Event>) {
        for event in events {
            if let Event::Onset { frame, strength } = event {
                self.waiting.push_back((frame, strength));
            }
            out.push(event);
        }
    }
}

impl Stage for Hits {
    type In = Mono;
    type Out = Event;

    fn push(&mut self, input: Mono, out: &mut Vec<Event>) -> Result<()> {
        let mut onsets = Vec::new();
        self.onsets.push(input.clone(), &mut onsets)?;
        if self.origin.is_none() {
            self.origin = Some(input.start_frame);
            self.buffer_start = input.start_frame;
        }
        self.received += input.samples.len() as u64;
        self.buffer.extend(input.samples);
        self.take_onsets(onsets, out);
        self.resolve(false, out);
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<Event>) -> Result<()> {
        let mut onsets = Vec::new();
        self.onsets.finish(&mut onsets)?;
        self.take_onsets(onsets, out);
        self.resolve(true, out);
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        self.onsets.latency_frames().max(self.after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    const RATE: u32 = 22_050;

    fn run(signal: &[f32], chunk: usize, mode: LaneMode) -> Vec<Event> {
        let mut stage = Hits::new(RATE, mode);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(chunk).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: (i * chunk) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        out
    }

    fn hits(events: &[Event]) -> Vec<(u64, HitLane, f32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Hit {
                    frame,
                    lane,
                    strength,
                } => Some((*frame, *lane, *strength)),
                _ => None,
            })
            .collect()
    }

    fn onsets(events: &[Event]) -> Vec<(u64, f32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Onset { frame, strength } => Some((*frame, *strength)),
                _ => None,
            })
            .collect()
    }

    /// A kick at 0.5 s, a snare at 1 s and a closed hat at 1.5 s.
    fn pattern() -> Vec<f32> {
        let mut signal = synth::noise_bursts(RATE, 4.0, &[(0.5, 0.1)], 40.0, 150.0, 0.8);
        let snare = synth::noise_bursts(RATE, 4.0, &[(1.0, 0.1)], 1_000.0, 4_000.0, 0.8);
        synth::add(&mut signal, &snare);
        let hat = synth::noise_bursts(RATE, 4.0, &[(1.5, 0.05)], 6_000.0, 10_000.0, 0.8);
        synth::add(&mut signal, &hat);
        signal
    }

    #[test]
    fn low_mid_and_high_bursts_are_kick_snare_and_hat() {
        let found = hits(&run(&pattern(), 1024, LaneMode::Beatbox));
        let lanes: Vec<HitLane> = found.iter().map(|h| h.1).collect();
        assert_eq!(
            lanes,
            [HitLane::Kick, HitLane::Snare, HitLane::Hat],
            "{found:?}"
        );
        for (hit, secs) in found.iter().zip([0.5, 1.0, 1.5]) {
            let at = hit.0 as f64 / f64::from(RATE);
            assert!((at - secs).abs() < 0.02, "hit at {at} s, wanted {secs} s");
        }
    }

    #[test]
    fn asked_mode_puts_every_hit_in_the_asked_lane() {
        let found = hits(&run(&pattern(), 1024, LaneMode::Asked));
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().all(|h| h.1 == HitLane::Asked), "{found:?}");
    }

    #[test]
    fn one_hit_per_onset() {
        let starts: Vec<(f64, f64)> = (0..12).map(|i| (0.5 + 0.5 * f64::from(i), 0.1)).collect();
        let signal = synth::noise_bursts(RATE, 7.0, &starts, 1_000.0, 4_000.0, 0.8);
        let events = run(&signal, 1024, LaneMode::Beatbox);
        let found = hits(&events);
        assert_eq!(found.len(), 12, "{found:?}");
        let marks = onsets(&events);
        assert_eq!(marks.len(), 12, "{marks:?}");
        for (hit, mark) in found.iter().zip(&marks) {
            assert_eq!((hit.0, hit.2), *mark);
        }
    }

    #[test]
    fn events_do_not_depend_on_how_the_stream_is_cut() {
        let signal = pattern();
        let reference = run(&signal, signal.len(), LaneMode::Beatbox);
        assert_eq!(hits(&reference).len(), 3);
        for chunk in [1, 777, 4096] {
            let events = run(&signal, chunk, LaneMode::Beatbox);
            assert_eq!(hits(&events), hits(&reference), "hits, chunk {chunk}");
            assert_eq!(onsets(&events), onsets(&reference), "onsets, chunk {chunk}");
        }
    }

    #[test]
    fn a_hit_is_emitted_within_the_stated_latency() {
        let signal = pattern();
        let mut stage = Hits::new(RATE, LaneMode::Beatbox);
        let latency = stage.latency_frames();
        let mut seen = 0;
        for (i, piece) in signal.chunks(1).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: i as u64,
                samples: piece.to_vec(),
            };
            let mut out = Vec::new();
            stage.push(mono, &mut out).unwrap();
            for hit in hits(&out) {
                seen += 1;
                let late = (i as u64 + 1) - hit.0;
                assert!(
                    late <= latency,
                    "emitted {late} frames late, bound {latency}"
                );
            }
        }
        assert_eq!(seen, 3);
    }

    #[test]
    fn a_take_opening_on_a_hit_is_classified() {
        let signal = synth::noise_bursts(RATE, 1.0, &[(0.0, 0.1)], 40.0, 150.0, 0.8);
        let found = hits(&run(&signal, 512, LaneMode::Beatbox));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].1, HitLane::Kick);
    }

    #[test]
    fn a_quiet_hat_close_behind_a_loud_kick_is_a_hat() {
        for gap in [0.05, 0.055, 0.06] {
            let mut signal = Vec::new();
            for i in 0..3 {
                let start = 0.5 + f64::from(i) * 1.0;
                // The kick sounds 0.2 s, decaying with a 40 ms time constant: its tail, still
                // 12 dB down at 80 ms, runs under the hat.
                let mut kick = synth::noise_bursts(RATE, 4.0, &[(start, 0.2)], 40.0, 150.0, 0.8);
                let first = (start * f64::from(RATE)) as usize;
                for (i, s) in kick.iter_mut().enumerate().skip(first) {
                    *s *= (-((i - first) as f32) / (0.04 * RATE as f32)).exp();
                }
                let hat =
                    synth::noise_bursts(RATE, 4.0, &[(start + gap, 0.05)], 6_000.0, 10_000.0, 0.16);
                if signal.is_empty() {
                    signal = kick;
                } else {
                    synth::add(&mut signal, &kick);
                }
                synth::add(&mut signal, &hat);
            }
            let reference = run(&signal, signal.len(), LaneMode::Beatbox);
            for chunk in [1, 777, 1024, 4096] {
                let events = run(&signal, chunk, LaneMode::Beatbox);
                let found = hits(&events);
                let lanes: Vec<HitLane> = found.iter().map(|h| h.1).collect();
                assert_eq!(
                    lanes,
                    [HitLane::Kick, HitLane::Hat].repeat(3),
                    "gap {gap}, chunk {chunk}: {found:?}"
                );
                assert_eq!(found, hits(&reference), "hits, gap {gap}, chunk {chunk}");
                assert_eq!(
                    onsets(&events),
                    onsets(&reference),
                    "onsets, gap {gap}, chunk {chunk}"
                );
            }
        }
    }

    /// A sung "un" at `start`: a voiced tone on 110 Hz, 150 ms long, with a 10 ms rise and a
    /// 30 ms fall. Its partials fall 6 dB an octave, a nasal formant near 250 Hz lifts them, and
    /// a phone's microphone, rolling off 12 dB an octave below 150 Hz, weakens its fundamental,
    /// so its energy sits between about 100 Hz and 1 kHz, mostly above 200 Hz.
    fn un(signal: &mut [f32], start: f64, amplitude: f32) {
        const F0: f64 = 110.0;
        const FORMANT_HZ: f64 = 250.0;
        const MIC_CORNER_HZ: f64 = 150.0;
        let first = (start * f64::from(RATE)) as usize;
        let count = (0.15 * f64::from(RATE)) as usize;
        let (rise, fall) = (0.01 * f64::from(RATE), 0.03 * f64::from(RATE));
        let gains: Vec<f64> = (1..=9)
            .map(|n| {
                let hz = F0 * f64::from(n);
                let resonance = 1.0 / (1.0 + ((hz - FORMANT_HZ) / 120.0).powi(2));
                let mic = (hz / MIC_CORNER_HZ).powi(2) / (1.0 + (hz / MIC_CORNER_HZ).powi(2));
                (0.3 + resonance) / f64::from(n) * mic
            })
            .collect();
        let norm: f64 = gains.iter().sum();
        for i in 0..count {
            let t = i as f64 / f64::from(RATE);
            let sum: f64 = gains
                .iter()
                .enumerate()
                .map(|(k, g)| g * (std::f64::consts::TAU * F0 * (k + 1) as f64 * t).sin())
                .sum();
            let envelope = (i as f64 / rise)
                .min((count - 1 - i) as f64 / fall)
                .min(1.0);
            signal[first + i] += (sum / norm * envelope) as f32 * amplitude;
        }
    }

    #[test]
    fn a_voiced_unce_is_a_kick_then_a_hat() {
        // Four "unce"s a beat apart: each "un" 150 ms, its "ce" a noise burst just after it.
        let mut signal = synth::silence(RATE, 3.0);
        for i in 0..4 {
            let start = 0.25 + 0.5 * f64::from(i);
            un(&mut signal, start, 0.6);
            let ce =
                synth::noise_bursts(RATE, 3.0, &[(start + 0.15, 0.08)], 4_000.0, 10_000.0, 0.3);
            synth::add(&mut signal, &ce);
        }
        let reference = run(&signal, signal.len(), LaneMode::Beatbox);
        let found = hits(&reference);
        let lanes: Vec<HitLane> = found.iter().map(|h| h.1).collect();
        assert_eq!(lanes, [HitLane::Kick, HitLane::Hat].repeat(4), "{found:?}");
        for (pair, hit) in found.chunks(2).enumerate() {
            let start = 0.25 + 0.5 * pair as f64;
            for (hit, secs) in hit.iter().zip([start, start + 0.15]) {
                let at = hit.0 as f64 / f64::from(RATE);
                assert!((at - secs).abs() < 0.02, "hit at {at} s, wanted {secs} s");
            }
        }
        for chunk in [1, 777, 4096] {
            let events = run(&signal, chunk, LaneMode::Beatbox);
            assert_eq!(hits(&events), found, "hits, chunk {chunk}");
        }
    }

    #[test]
    fn unvoiced_noise_in_the_voice_range_is_not_a_kick() {
        // The band a sung "un" fills, but noise, with no held pitch: a snare, not a voiced kick.
        let signal =
            synth::noise_bursts(RATE, 3.0, &[(0.5, 0.1), (1.5, 0.15)], 200.0, 1_000.0, 0.8);
        let found = hits(&run(&signal, 1024, LaneMode::Beatbox));
        let lanes: Vec<HitLane> = found.iter().map(|h| h.1).collect();
        assert_eq!(lanes, [HitLane::Snare, HitLane::Snare], "{found:?}");
    }

    #[test]
    fn hit_frames_are_on_the_timeline_the_stream_started_at() {
        let signal = pattern();
        let origin = 50_000;
        let mut stage = Hits::new(RATE, LaneMode::Beatbox);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(1024).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: origin + (i * 1024) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        let found = hits(&out);
        let reference = hits(&run(&signal, 1024, LaneMode::Beatbox));
        assert_eq!(found.len(), 3, "{found:?}");
        for (late, plain) in found.iter().zip(&reference) {
            assert_eq!(
                (late.0, late.1, late.2),
                (plain.0 + origin, plain.1, plain.2)
            );
        }
    }
}
