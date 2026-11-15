use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::error::{Error, Result};
use crate::stage::{Mono, Stage};

/// The lowest pitch tracked: below B1 (61.7 Hz), under a bass voice's low E2 and a guitar's
/// low E2 or drop-D.
const FMIN_HZ: f64 = 60.0;
/// The highest pitch tracked: above E6 (1318.5 Hz), a 24-fret guitar's top note, and a
/// soprano's high C6.
const FMAX_HZ: f64 = 1400.0;
/// The analysis frame, as time: about 46 ms, rounded up to a power of two in samples (1024 at
/// 22 050 Hz). Its first half is the difference function's integration window, and the second
/// half leaves room for the longest period (`rate / FMIN_HZ`, 368 samples at 22 050 Hz).
const FRAME_SECS: f64 = 0.046;
/// Hop between frames, as a fraction of the frame: 256 samples, 11.6 ms, at 22 050 Hz.
const HOP_DIVISOR: usize = 4;
/// Resolution of the pitch states: a tenth of a semitone, the reference pYIN's.
const BINS_PER_SEMITONE: usize = 10;
/// Thresholds the candidate scoring sweeps over `0..1`, the reference pYIN's.
const THRESHOLDS: usize = 100;
/// The Boltzmann prior over troughs: larger favours shorter periods. The reference pYIN's.
const BOLTZMANN: f32 = 2.0;
/// Probability given to the deepest trough under a threshold no trough is below. The reference
/// pYIN's.
const NO_TROUGH_PROB: f32 = 0.01;
/// The fastest pitch change the smoothing allows, in octaves a second. The reference pYIN's.
const MAX_OCTAVES_PER_SEC: f64 = 35.92;
/// Probability of switching between voiced and unvoiced from one frame to the next. The
/// reference pYIN's.
const SWITCH_PROB: f32 = 0.01;
/// Energies and correlations smaller than this are zero, so silence has no period. The reference
/// pYIN's.
const NEGLIGIBLE: f32 = 1e-6;
/// How far behind the newest frame the smoothing settles a frame. Longer is steadier and later.
const LAG_SECS: f64 = 0.2;
/// How far, in pitch bins, a frame's own candidate may be from its smoothed state and still
/// refine it: half a semitone.
const REFINE_BINS: usize = BINS_PER_SEMITONE / 2;

/// One settled frame of the pitch track.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PitchFrame {
    /// Where the frame sits, on the input [`Mono`] timeline: the middle of its integration window.
    pub frame: u64,
    /// The fundamental, when the smoothed track says the frame is voiced: the centre of the
    /// smoothed state's 10-cent bin. Notes are segmented from this.
    pub hz: Option<f32>,
    /// The same fundamental, refined: the frequency of the frame's own most probable candidate
    /// within half a semitone of the smoothed state, whose period is placed between samples by a
    /// parabola through its trough. The bin's centre when no candidate is that near. `None`
    /// exactly when [`hz`](Self::hz) is. The performance is written from this.
    pub fine_hz: Option<f32>,
    /// The frame's own voicing probability, before smoothing: the candidates' total.
    pub voiced: f32,
    /// The root mean square of the frame's integration window: how loud the frame is.
    pub level: f32,
}

/// Tracks the fundamental, frame by frame, pYIN-style.
///
/// Each frame gets YIN's cumulative mean normalised difference, and its troughs are scored as
/// pitch candidates the way the reference pYIN scores them: over a Beta(2, 18) prior on the
/// threshold, with a Boltzmann prior favouring shorter periods. The candidates' total is the
/// frame's voicing probability. A hidden Markov model over pitch states, each voiced or
/// unvoiced, smooths the track, decoded by fixed-lag Viterbi: a frame is settled once
/// 0.2 s (`LAG_SECS`) of frames after it have been seen, from the best path to the newest frame.
/// Nothing is decoded over the whole take, and only the frames inside the lag are held.
///
/// The range is 60 Hz to 1400 Hz (`FMIN_HZ`, `FMAX_HZ`). Frames sit at fixed offsets from the
/// first input frame, so the track does not depend on how the stream is cut into chunks. At the
/// end the take is padded with zeros so its last frames can be analysed.
pub struct PitchTrack {
    rate: u32,
    frame_len: usize,
    hop: usize,
    yin: Yin,
    hmm: FixedLag,
    /// The stream's first frame, which windows are numbered from.
    origin: Option<u64>,
    /// Frames received so far.
    received: u64,
    /// Samples not yet dropped, starting at stream offset `buffer_start`.
    buffer: Vec<f32>,
    buffer_start: usize,
    /// The next window to analyse.
    next_window: usize,
    /// Frames analysed but not yet settled: where each sits, its voicing probability, its level
    /// and its candidates.
    unsettled: VecDeque<Unsettled>,
}

/// A frame analysed but not yet settled.
struct Unsettled {
    frame: u64,
    voiced: f32,
    level: f32,
    /// Its candidates, as `(pitch bin, Hz, probability)`.
    candidates: Vec<(usize, f32, f32)>,
}

impl std::fmt::Debug for PitchTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PitchTrack")
            .field("rate", &self.rate)
            .field("frame_len", &self.frame_len)
            .field("hop", &self.hop)
            .finish_non_exhaustive()
    }
}

impl PitchTrack {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32) -> PitchTrack {
        let frame_len = ((f64::from(rate) * FRAME_SECS).round() as usize).next_power_of_two();
        let hop = frame_len / HOP_DIVISOR;
        let bins =
            (12.0 * BINS_PER_SEMITONE as f64 * (FMAX_HZ / FMIN_HZ).log2()).floor() as usize + 1;
        let semitones_per_hop =
            (MAX_OCTAVES_PER_SEC * 12.0 * hop as f64 / f64::from(rate)).round() as usize;
        let lag = ((LAG_SECS * f64::from(rate)) / hop as f64).ceil() as usize;
        PitchTrack {
            rate,
            frame_len,
            hop,
            yin: Yin::new(rate, frame_len),
            hmm: FixedLag::new(bins, semitones_per_hop * BINS_PER_SEMITONE / 2, lag),
            origin: None,
            received: 0,
            buffer: Vec::new(),
            buffer_start: 0,
            next_window: 0,
            unsettled: VecDeque::new(),
        }
    }

    /// Samples between one frame and the next.
    pub(crate) fn hop(&self) -> u64 {
        self.hop as u64
    }

    /// The rate the track expects.
    pub(crate) fn rate(&self) -> u32 {
        self.rate
    }

    /// Where the first frame sits, once the first input has been pushed.
    pub(crate) fn first_frame(&self) -> Option<u64> {
        self.origin.map(|_| self.frame_of(0))
    }

    /// The pitch bin a frequency falls in, clamped to the range.
    fn bin_of(&self, hz: f64) -> usize {
        let bin = (12.0 * BINS_PER_SEMITONE as f64 * (hz / FMIN_HZ).log2()).round();
        (bin.max(0.0) as usize).min(self.hmm.bins - 1)
    }

    /// Where window `k` is placed: the middle of its integration window.
    fn frame_of(&self, k: usize) -> u64 {
        self.origin.unwrap_or(0) + (k * self.hop + self.yin.window / 2) as u64
    }

    /// Analyses every window the buffer now holds in full, and emits what settles.
    fn analyse(&mut self, out: &mut Vec<PitchFrame>) {
        loop {
            let start = self.next_window * self.hop - self.buffer_start;
            let Some(samples) = self.buffer.get(start..start + self.frame_len) else {
                break;
            };
            let (candidates, level) = self.yin.candidates(samples);
            let voiced = candidates.iter().fold(0.0, |total, c| total + c.1).min(1.0);
            let observation = self.observation(&candidates, voiced);
            let candidates = candidates
                .iter()
                .map(|&(period, probability)| {
                    let hz = f64::from(self.rate) / f64::from(period);
                    (self.bin_of(hz), hz as f32, probability)
                })
                .collect();
            self.unsettled.push_back(Unsettled {
                frame: self.frame_of(self.next_window),
                voiced,
                level,
                candidates,
            });
            self.next_window += 1;
            if let Some(state) = self.hmm.step(&observation) {
                self.settle(state, out);
            }
        }
        let keep_from = self.next_window * self.hop;
        let drop = keep_from
            .saturating_sub(self.buffer_start)
            .min(self.buffer.len());
        self.buffer.drain(..drop);
        self.buffer_start += drop;
    }

    /// The log likelihood of each state, voiced bins first, then unvoiced: a voiced bin has the
    /// probability of the candidates in it, and every unvoiced bin an equal share of the rest.
    fn observation(&self, candidates: &[(f32, f32)], voiced: f32) -> Vec<f32> {
        let bins = self.hmm.bins;
        let mut likelihood = vec![0.0; bins];
        for &(period, probability) in candidates {
            likelihood[self.bin_of(f64::from(self.rate) / f64::from(period))] += probability;
        }
        let unvoiced = ((1.0 - voiced) / bins as f32 + f32::MIN_POSITIVE).ln();
        let mut observation: Vec<f32> = likelihood
            .iter()
            .map(|p| (p + f32::MIN_POSITIVE).ln())
            .collect();
        observation.resize(2 * bins, unvoiced);
        observation
    }

    /// Emits the oldest unsettled frame, now settled in `state`.
    fn settle(&mut self, state: usize, out: &mut Vec<PitchFrame>) {
        let Unsettled {
            frame,
            voiced,
            level,
            candidates,
        } = self
            .unsettled
            .pop_front()
            .expect("the smoothing settles only frames it was given");
        let hz = (state < self.hmm.bins)
            .then(|| (FMIN_HZ * (state as f64 / (12 * BINS_PER_SEMITONE) as f64).exp2()) as f32);
        let fine_hz = hz.map(|centre| {
            candidates
                .iter()
                .filter(|(bin, _, _)| bin.abs_diff(state) <= REFINE_BINS)
                .max_by(|a, b| a.2.total_cmp(&b.2))
                .map_or(centre, |&(_, hz, _)| hz)
        });
        out.push(PitchFrame {
            frame,
            hz,
            fine_hz,
            voiced,
            level,
        });
    }
}

impl Stage for PitchTrack {
    type In = Mono;
    type Out = PitchFrame;

    fn push(&mut self, input: Mono, out: &mut Vec<PitchFrame>) -> Result<()> {
        if input.rate != self.rate {
            return Err(Error::Invalid(format!(
                "pitch tracking expects {} Hz, got a chunk at {} Hz",
                self.rate, input.rate
            )));
        }
        let origin = *self.origin.get_or_insert(input.start_frame);
        if input.start_frame != origin + self.received {
            return Err(Error::Invalid(format!(
                "the audio breaks off at {:.3} s and picks up at {:.3} s; pitch tracking needs it \
                 unbroken, as Prepare gives it",
                (origin + self.received) as f64 / f64::from(self.rate),
                input.start_frame as f64 / f64::from(self.rate)
            )));
        }
        self.received += input.samples.len() as u64;
        self.buffer.extend_from_slice(&input.samples);
        self.analyse(out);
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<PitchFrame>) -> Result<()> {
        if self.received > 0 {
            // Pad with zeros so every window that starts inside the take is analysed.
            let last_start = ((self.received - 1) as usize / self.hop) * self.hop;
            let end = last_start + self.frame_len;
            let held = self.buffer_start + self.buffer.len();
            self.buffer
                .resize(self.buffer.len() + end.saturating_sub(held), 0.0);
            self.analyse(out);
        }
        for state in self.hmm.flush() {
            self.settle(state, out);
        }
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        // A frame settles once the window `lag` hops after its own is whole.
        (self.hmm.lag * self.hop + self.frame_len - self.yin.window / 2) as u64
    }
}

/// YIN's difference function and pYIN's candidate scoring, for one frame at a time.
struct Yin {
    frame_len: usize,
    /// The difference function's integration window.
    window: usize,
    min_period: usize,
    max_period: usize,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    /// The Beta(2, 18) prior's mass on each threshold's step.
    threshold_prior: Vec<f32>,
}

impl Yin {
    fn new(rate: u32, frame_len: usize) -> Yin {
        let window = frame_len / 2;
        let mut planner = FftPlanner::new();
        // Beta(2, 18)'s CDF in closed form, since both shapes are integers.
        let cdf = |x: f64| 1.0 - (1.0 - x).powi(19) - 19.0 * x * (1.0 - x).powi(18);
        let threshold_prior = (0..THRESHOLDS)
            .map(|j| {
                let step = 1.0 / THRESHOLDS as f64;
                (cdf((j + 1) as f64 * step) - cdf(j as f64 * step)) as f32
            })
            .collect();
        Yin {
            frame_len,
            window,
            min_period: ((f64::from(rate) / FMAX_HZ).floor() as usize).max(1),
            max_period: ((f64::from(rate) / FMIN_HZ).ceil() as usize).min(frame_len - window - 1),
            fft: planner.plan_fft_forward(frame_len),
            ifft: planner.plan_fft_inverse(frame_len),
            threshold_prior,
        }
    }

    /// YIN's cumulative mean normalised difference for periods `min_period..=max_period`, and
    /// the integration window's energy.
    fn normalised_difference(&self, frame: &[f32]) -> (Vec<f32>, f32) {
        let negligible = |x: f32| if x.abs() < NEGLIGIBLE { 0.0 } else { x };
        // The window's correlation with the frame at each lag, by FFT.
        let mut whole: Vec<Complex<f32>> = frame.iter().map(|&s| Complex::new(s, 0.0)).collect();
        let mut head: Vec<Complex<f32>> = frame[..self.window]
            .iter()
            .map(|&s| Complex::new(s, 0.0))
            .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
            .take(self.frame_len)
            .collect();
        self.fft.process(&mut whole);
        self.fft.process(&mut head);
        let mut correlation: Vec<Complex<f32>> =
            whole.iter().zip(&head).map(|(a, b)| a * b.conj()).collect();
        self.ifft.process(&mut correlation);
        let scale = 1.0 / self.frame_len as f32;

        let mut prefix = Vec::with_capacity(frame.len() + 1);
        prefix.push(0.0f64);
        for &s in frame {
            prefix.push(prefix[prefix.len() - 1] + f64::from(s) * f64::from(s));
        }
        let energy = |lag: usize| negligible((prefix[lag + self.window] - prefix[lag]) as f32);

        let first = energy(0);
        let mut running = 0.0;
        let mut out = Vec::with_capacity(self.max_period - self.min_period + 1);
        for (lag, product) in correlation
            .iter()
            .enumerate()
            .take(self.max_period + 1)
            .skip(1)
        {
            let difference = first + energy(lag) - 2.0 * negligible(product.re * scale);
            running += difference;
            if lag >= self.min_period {
                out.push(difference / (running / lag as f32 + f32::MIN_POSITIVE));
            }
        }
        (out, first)
    }

    /// The frame's pitch candidates as `(period in samples, probability)`, pYIN-style, and the
    /// root mean square of its integration window.
    fn candidates(&self, frame: &[f32]) -> (Vec<(f32, f32)>, f32) {
        let (curve, energy) = self.normalised_difference(frame);
        let level = (energy / self.window as f32).sqrt();
        let last = curve.len() - 1;
        let troughs: Vec<usize> = (0..=last)
            .filter(|&i| match i {
                0 => curve[0] < curve[1],
                i if i == last => curve[last] < curve[last - 1],
                i => curve[i] < curve[i - 1] && curve[i] <= curve[i + 1],
            })
            .collect();
        let Some(&deepest) = troughs
            .iter()
            .min_by(|&&a, &&b| curve[a].total_cmp(&curve[b]))
        else {
            return (Vec::new(), level);
        };
        let mut probability = vec![0.0f32; troughs.len()];
        for (j, &prior) in self.threshold_prior.iter().enumerate() {
            let threshold = (j + 1) as f32 / THRESHOLDS as f32;
            let below = troughs.iter().filter(|&&i| curve[i] < threshold).count();
            if below == 0 {
                // No trough under this threshold: its mass goes, scaled down, to the deepest.
                let at = troughs.iter().position(|&i| i == deepest).unwrap_or(0);
                probability[at] += NO_TROUGH_PROB * prior;
                continue;
            }
            let norm = (1.0 - (-BOLTZMANN).exp()) / (1.0 - (-BOLTZMANN * below as f32).exp());
            let mut rank = 0;
            for (t, &i) in troughs.iter().enumerate() {
                if curve[i] < threshold {
                    probability[t] += prior * norm * (-BOLTZMANN * rank as f32).exp();
                    rank += 1;
                }
            }
        }
        let candidates = troughs
            .iter()
            .zip(probability)
            .filter(|&(_, p)| p > 0.0)
            .map(|(&i, p)| ((i + self.min_period) as f32 + parabolic_shift(&curve, i), p))
            .collect();
        (candidates, level)
    }
}

/// Where the parabola through a trough and its neighbours bottoms out, relative to the trough.
fn parabolic_shift(curve: &[f32], i: usize) -> f32 {
    if i == 0 || i + 1 >= curve.len() {
        return 0.0;
    }
    let a = (curve[i - 1] + curve[i + 1] - 2.0 * curve[i]) / 2.0;
    let b = (curve[i + 1] - curve[i - 1]) / 2.0;
    let shift = -b / (2.0 * a + f32::MIN_POSITIVE);
    if shift.abs() <= 1.0 { shift } else { 0.0 }
}

/// Viterbi decoding that settles each frame a fixed number of frames late.
///
/// The states are `bins` pitch bins voiced, then the same bins unvoiced. Pitch moves at most
/// `reach` bins a frame, with a triangular preference for staying put, and voicing switches with
/// [`SWITCH_PROB`]. Scores are log probabilities, renormalised every frame so a long take cannot
/// drift them out of range.
struct FixedLag {
    bins: usize,
    reach: usize,
    /// `log P(i -> i + d)` at `i * (2 * reach + 1) + (d + reach)`.
    log_move: Vec<f32>,
    log_stay: f32,
    log_switch: f32,
    lag: usize,
    /// The best score of a path ending in each state; empty before the first frame.
    score: Vec<f32>,
    /// For each of the newest frames, each state's best predecessor; newest last.
    back: VecDeque<Vec<u16>>,
    /// Frames seen but not yet settled.
    unsettled: usize,
}

impl FixedLag {
    fn new(bins: usize, reach: usize, lag: usize) -> FixedLag {
        let width = 2 * reach + 1;
        let mut log_move = vec![f32::NEG_INFINITY; bins * width];
        for i in 0..bins {
            let targets = i.saturating_sub(reach)..=(i + reach).min(bins - 1);
            let weight = |j: usize| (reach + 1 - i.abs_diff(j)) as f32;
            let total: f32 = targets.clone().map(weight).sum();
            for j in targets {
                log_move[i * width + j + reach - i] = (weight(j) / total).ln();
            }
        }
        FixedLag {
            bins,
            reach,
            log_move,
            log_stay: (1.0 - SWITCH_PROB).ln(),
            log_switch: SWITCH_PROB.ln(),
            lag,
            score: Vec::new(),
            back: VecDeque::new(),
            unsettled: 0,
        }
    }

    /// Takes the next frame's log likelihoods, and returns the state of the frame `lag` frames
    /// back once it has settled.
    fn step(&mut self, observation: &[f32]) -> Option<usize> {
        let n = self.bins;
        if self.score.is_empty() {
            // Every path starts unvoiced, at any pitch.
            let start = (1.0 / n as f32).ln();
            self.score = observation
                .iter()
                .enumerate()
                .map(|(s, o)| o + if s < n { f32::MIN_POSITIVE.ln() } else { start })
                .collect();
        } else {
            let width = 2 * self.reach + 1;
            let mut score = vec![0.0; 2 * n];
            let mut back = vec![0u16; 2 * n];
            for j in 0..n {
                let (mut voiced, mut from_voiced) = (f32::NEG_INFINITY, 0);
                let (mut unvoiced, mut from_unvoiced) = (f32::NEG_INFINITY, 0);
                for i in j.saturating_sub(self.reach)..=(j + self.reach).min(n - 1) {
                    let step = self.log_move[i * width + j + self.reach - i];
                    if self.score[i] + step > voiced {
                        (voiced, from_voiced) = (self.score[i] + step, i);
                    }
                    if self.score[n + i] + step > unvoiced {
                        (unvoiced, from_unvoiced) = (self.score[n + i] + step, n + i);
                    }
                }
                let best = |same: (f32, usize), other: (f32, usize)| {
                    let (same, other) = (
                        (same.0 + self.log_stay, same.1),
                        (other.0 + self.log_switch, other.1),
                    );
                    if same.0 >= other.0 { same } else { other }
                };
                let (v, from) = best((voiced, from_voiced), (unvoiced, from_unvoiced));
                (score[j], back[j]) = (observation[j] + v, from as u16);
                let (u, from) = best((unvoiced, from_unvoiced), (voiced, from_voiced));
                (score[n + j], back[n + j]) = (observation[n + j] + u, from as u16);
            }
            let top = score.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            score.iter_mut().for_each(|s| *s -= top);
            self.score = score;
            self.back.push_back(back);
            if self.back.len() > self.lag {
                self.back.pop_front();
            }
        }
        self.unsettled += 1;
        if self.unsettled <= self.lag {
            return None;
        }
        self.unsettled -= 1;
        let mut state = self.best();
        for back in self.back.iter().rev().take(self.lag) {
            state = usize::from(back[state]);
        }
        Some(state)
    }

    /// The states of every frame not yet settled, oldest first, from the best path to the newest.
    fn flush(&mut self) -> Vec<usize> {
        if self.unsettled == 0 {
            return Vec::new();
        }
        let mut state = self.best();
        let mut states = vec![state];
        for back in self.back.iter().rev().take(self.unsettled - 1) {
            state = usize::from(back[state]);
            states.push(state);
        }
        states.reverse();
        self.unsettled = 0;
        states
    }

    fn best(&self) -> usize {
        self.score
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(s, _)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    const RATE: u32 = 22_050;

    fn track(signal: &[f32], chunk: usize) -> Vec<PitchFrame> {
        let mut stage = PitchTrack::new(RATE);
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

    fn cents(a: f32, b: f32) -> f32 {
        1200.0 * (a / b).log2()
    }

    fn scale() -> Vec<f32> {
        let notes: Vec<(f64, f64)> = [261.63, 293.66, 329.63, 349.23, 392.0]
            .iter()
            .map(|&f| (f, 0.25))
            .collect();
        let mut signal = synth::silence(RATE, 0.2);
        signal.extend(synth::note_sequence(RATE, &notes, 4, 0.5));
        signal.extend(synth::silence(RATE, 0.2));
        signal
    }

    #[test]
    fn a_steady_tone_is_tracked_at_its_frequency() {
        let mut signal = synth::silence(RATE, 0.3);
        signal.extend(synth::tone(RATE, 220.0, 1.0, 4, 0.5));
        signal.extend(synth::silence(RATE, 0.3));
        let frames = track(&signal, 1000);
        let inside: Vec<&PitchFrame> = frames
            .iter()
            .filter(|p| {
                let at = p.frame as f64 / f64::from(RATE);
                (0.4..1.2).contains(&at)
            })
            .collect();
        assert!(!inside.is_empty());
        for p in inside {
            let hz =
                p.hz.unwrap_or_else(|| panic!("unvoiced inside the tone: {p:?}"));
            assert!(cents(hz, 220.0).abs() < 10.0, "{p:?}");
        }
    }

    #[test]
    fn silence_is_unvoiced() {
        let frames = track(&synth::silence(RATE, 1.0), 1000);
        assert!(!frames.is_empty());
        assert!(frames.iter().all(|p| p.hz.is_none() && p.voiced == 0.0));
    }

    #[test]
    fn frames_cover_the_take_one_hop_apart() {
        let frames = track(&synth::silence(RATE, 1.0), 1000);
        let hop = frames[1].frame - frames[0].frame;
        assert!(frames.windows(2).all(|w| w[1].frame - w[0].frame == hop));
        assert!(frames.last().unwrap().frame + hop >= RATE as u64);
    }

    #[test]
    fn the_track_is_the_same_whatever_the_chunking() {
        let signal = scale();
        let reference = track(&signal, 1000);
        assert!(reference.iter().any(|p| p.hz.is_some()));
        for chunk in [1, 777, 4096] {
            assert_eq!(track(&signal, chunk), reference, "chunk of {chunk}");
        }
    }

    #[test]
    fn frames_settle_within_the_reported_latency() {
        let signal = scale();
        let mut stage = PitchTrack::new(RATE);
        let mut out = Vec::new();
        let mut fed = 0u64;
        let chunk = 64;
        for piece in signal.chunks(chunk) {
            let mono = Mono {
                rate: RATE,
                start_frame: fed,
                samples: piece.to_vec(),
            };
            fed += piece.len() as u64;
            let before = out.len();
            stage.push(mono, &mut out).unwrap();
            for p in &out[before..] {
                assert!(
                    fed <= p.frame + stage.latency_frames() + chunk as u64,
                    "settled at {fed} for a frame at {}",
                    p.frame
                );
            }
        }
        assert!(!out.is_empty());
    }

    #[test]
    fn a_chunk_that_skips_ahead_is_refused_in_seconds() {
        let mut stage = PitchTrack::new(RATE);
        let mono = |start_frame| Mono {
            rate: RATE,
            start_frame,
            samples: vec![0.0; 100],
        };
        stage.push(mono(0), &mut Vec::new()).unwrap();
        let err = stage.push(mono(500), &mut Vec::new()).unwrap_err();
        // Said in the recording's time: it breaks off after 100 frames and resumes at 500.
        let message = err.to_string();
        assert!(
            message.contains("breaks off at 0.005 s") && message.contains("picks up at 0.023 s"),
            "{message}"
        );
    }

    /// Our track against the reference pYIN's, frame by frame: within 50 cents wherever both say
    /// voiced, on at least 80 frames.
    ///
    /// pYIN's reading is a committed recording, `tests/fixtures/pyin-scale.txt`, whose header
    /// gives the pyin version and settings; pyin itself is not a dependency, since it pulls in
    /// zip 0.5, which `cargo audit` refuses. To re-record, add `pyin = "1.2.0"` under
    /// `[dev-dependencies]`, add this recorder to these tests, run it as the fixture's header
    /// says, and remove both again:
    ///
    /// ```text
    /// #[test]
    /// fn record_pyin_scale() {
    ///     use pyin::{Framing, PYINExecutor};
    ///     let stage = PitchTrack::new(RATE);
    ///     let mut reference = PYINExecutor::<f32>::new(
    ///         FMIN_HZ, FMAX_HZ, RATE, stage.frame_len, Some(stage.yin.window),
    ///         Some(stage.hop), Some(1.0 / BINS_PER_SEMITONE as f64),
    ///     );
    ///     let (_, f0, voiced, prob) = reference.pyin(&scale(), f32::NAN, Framing::Valid);
    ///     let mut text = String::new();
    ///     for (k, ((f, v), p)) in f0.iter().zip(&voiced).zip(&prob).enumerate() {
    ///         let f = if *v { format!("{f}") } else { "-".to_string() };
    ///         text.push_str(&format!("{k} {f} {p}\n"));
    ///     }
    ///     std::fs::write(std::env::var("PYIN_OUT").unwrap(), text).unwrap();
    /// }
    /// ```
    #[test]
    fn our_track_agrees_with_pyin_on_a_scale() {
        let signal = scale();
        let ours = track(&signal, 1000);
        let reading = pyin_reading();
        // One frame for each of ours whose window lies whole in the signal, which is what pyin's
        // Framing::Valid reads; ours goes on over the zero-padded end, which pyin does not. A
        // reading cut short would leave our frames unchecked without a word.
        let stage = PitchTrack::new(RATE);
        let whole = (signal.len() - stage.frame_len) / stage.hop + 1;
        assert!(ours.len() >= whole, "our track has {} frames", ours.len());
        assert_eq!(
            reading.len(),
            whole,
            "the pyin reading has {} frames; {whole} of our {} have whole windows",
            reading.len(),
            ours.len()
        );
        let mut compared = 0;
        for (k, theirs) in reading.into_iter().enumerate() {
            let ours = ours[k];
            if let (Some(hz), Some(theirs)) = (ours.hz, theirs) {
                compared += 1;
                assert!(
                    cents(hz, theirs).abs() <= 50.0,
                    "frame {k}: ours {hz} Hz, pyin {theirs} Hz"
                );
            }
        }
        // The notes span 1.25 s, about 107 frames: most must be compared, or the test is empty.
        assert!(compared >= 80, "only {compared} frames voiced in both");
    }

    /// The recorded pYIN f0 of each frame, `None` where it says unvoiced.
    fn pyin_reading() -> Vec<Option<f32>> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pyin-scale.txt");
        let text = std::fs::read_to_string(path).expect("the pyin recording is committed");
        text.lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .enumerate()
            .map(|(k, line)| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                assert_eq!(fields.len(), 3, "{line}");
                assert_eq!(
                    fields[0].parse::<usize>(),
                    Ok(k),
                    "frames out of order: {line}"
                );
                (fields[1] != "-").then(|| fields[1].parse().expect("f0 is a number"))
            })
            .collect()
    }
}
