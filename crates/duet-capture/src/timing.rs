//! Tempo, meter and downbeat, each used as given or estimated from a take's settled onsets,
//! and the beat-by-beat tempo the take actually kept.
//!
//! Tempo is quarter notes per minute everywhere, so in 6/8 a dotted quarter of 90 is 135.
//!
//! The estimate runs once, over every onset, at assembly ("For files it converts at
//! `finish`"); it is not a streaming stage.
//!
//! How a tempo, meter and downbeat are estimated:
//! 1. **The pulse.** The onsets' autocorrelation, built directly from onset pairs, each weighted
//!    by the square root of both onsets' relative strength and spread by [`LAG_SPREAD_SECS`].
//!    The pulse is its shortest lag peak within [`PULSE_PEAK_SHARE`] of the highest, so a
//!    pattern whose bars recur most strongly is still counted in its quickest regular step.
//! 2. **The grid.** The pulse is refined by a least-squares line through the onsets that fall
//!    within [`GRID_TOLERANCE`] of it, which is what makes a whole-bpm rounding trustworthy and
//!    gives the measured beat period notes are placed by. Its phase is the one, among grids
//!    anchored on each of the [`ANCHOR_CANDIDATES`] strongest onsets, that puts the most onset
//!    strength on the grid, so a loud stray or syncopated onset cannot pull it off the beat.
//! 3. **The meter.** Each meter is an accent template on that grid: the downbeat 1, a middle
//!    beat 0.5, other beats 0.25, subdivisions 0. A hypothesis is a meter, a number of grid
//!    steps to its felt beat (1, 2 or 4 in a simple meter; 3 or 6 in a compound one, whose
//!    beat divides in three, or also 1 when the meter is given), and a rotation. Its fit is
//!    the correlation of the on-grid onsets' strengths, relative to the strongest of them,
//!    with the template, scaled down when those strengths barely differ ([`ACCENT_SPREAD`]). Among hypotheses that fit at least [`WEAK_CONFIDENCE`], the one with
//!    the best fit times a tempo prior wins; with none, 4/4 applies at the beat level the prior
//!    favours, stated with its own, weak, fit.
//! 4. **The downbeat** is the first grid step the winning rotation puts on beat 1, or, when the
//!    meter evidence is weak, the first on-grid onset at least [`STRONG_ONSET`] of the loudest
//!    on-grid onset.
//!
//! Confidence is in 0..=1, and below [`WEAK_CONFIDENCE`] an estimate is weak. A tempo's is the
//! strength-weighted share of onsets on its grid. A meter's is its accent fit, and a downbeat's
//! is the fit of the meter that placed it.

use duet_core::TimeSig;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::event::Event;

/// The confidence below which an estimate is weak, and 4/4 is applied for want of a meter.
pub const WEAK_CONFIDENCE: f32 = 0.5;

/// The fewest onsets a tempo is estimated from.
const MIN_ONSETS: usize = 4;
/// The quickest pulse looked for: 600 a minute.
const MIN_PULSE_SECS: f64 = 0.1;
/// The slowest pulse looked for: 30 a minute.
const MAX_PULSE_SECS: f64 = 2.0;
/// The autocorrelation's lag resolution.
const LAG_STEP_SECS: f64 = 0.001;
/// How far each onset pair's lag is spread, as a Gaussian's standard deviation: the timing
/// slack two onsets may have and still count as the same interval.
const LAG_SPREAD_SECS: f64 = 0.01;
/// A lag peak at least this share of the highest can be the pulse. Task 9 tunes it.
const PULSE_PEAK_SHARE: f64 = 0.5;
/// How many of the strongest onsets are each tried as the grid's anchor. A loud onset off the
/// beat makes a poor anchor, so the anchor kept is the one whose grid holds the most onset
/// strength; this many loud off-beat onsets would be needed to crowd out every on-beat one.
const ANCHOR_CANDIDATES: usize = 16;
/// How far from a grid step, as a share of the step, an onset may sit and be on it.
/// Task 9 tunes it.
const GRID_TOLERANCE: f64 = 0.2;
/// The felt tempo the prior expects when none is given. Task 9 tunes it.
const PRIOR_BPM: f64 = 110.0;
/// How widely the prior spreads around [`PRIOR_BPM`], in octaves. Task 9 tunes it.
const PRIOR_OCTAVES: f64 = 1.0;
/// How widely the prior spreads around a given tempo, in octaves: it picks the beat level
/// nearest the given tempo.
const GIVEN_PRIOR_OCTAVES: f64 = 0.1;
/// The spread of relative onset strengths (their standard deviation) at which accents count in
/// full. Below it a meter's fit is scaled down, so the small differences every steady click has
/// are not taken for accents. Task 9 tunes it.
const ACCENT_SPREAD: f64 = 0.2;
/// The grid steps a simple meter's felt beat may span: a beat divides in two.
const SIMPLE_STEPS: [u32; 3] = [1, 2, 4];
/// The grid steps a compound meter's felt beat may span when its meter is estimated: a dotted
/// beat divides in three, and that three-way division is the only thing that tells it from a
/// simple beat.
const COMPOUND_STEPS: [u32; 2] = [3, 6];
/// The same when the meter is given, which tells it already: the grid may be the felt beat.
const GIVEN_COMPOUND_STEPS: [u32; 3] = [1, 3, 6];
/// The meters an estimate chooses between.
const CANDIDATES: [(u8, u8); 3] = [(4, 4), (3, 4), (6, 8)];
/// An on-grid onset at least this share of the loudest on-grid onset is strong enough to be a
/// downbeat when the meter does not say where one is. Task 9 tunes it.
const STRONG_ONSET: f64 = 0.5;

/// How far from a predicted beat, as a share of the beat, an onset may be and be that beat.
const SNAP_TOLERANCE: f64 = 0.2;
/// How much of the gap between a tracked beat's interval and the expected one moves the
/// expected one: enough to follow a slowly changing tempo. Task 9 tunes it.
const PERIOD_ADAPT: f64 = 0.25;
/// The beats around an interval that its local tempo is fitted over.
const FIT_BEATS: usize = 8;

/// What the user gave on the command line. Anything left out is estimated.
///
/// Deliberately exhaustive, so the command line can build one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Given {
    /// Quarter notes per minute.
    pub tempo_bpm: Option<f64>,
    pub meter: Option<TimeSig>,
    /// Where bar 1 starts, in seconds from the start of the take.
    pub downbeat_secs: Option<f64>,
}

/// A value the user gave, used exactly, or one estimated from the take, with how sure the
/// estimate is (0..=1; below [`WEAK_CONFIDENCE`] it is weak).
///
/// Deliberately exhaustive: a value is given or it is estimated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin<T> {
    Given(T),
    Estimated { value: T, confidence: f32 },
}

impl<T> Origin<T> {
    /// The value, given or estimated.
    pub fn value(&self) -> &T {
        match self {
            Origin::Given(value) | Origin::Estimated { value, .. } => value,
        }
    }
}

/// A take's tempo, meter and downbeat, each marked given or estimated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Timing {
    /// Quarter notes per minute. An estimate is a whole number.
    pub tempo_bpm: Origin<f64>,
    /// Half and double the tempo, the likeliest other readings of it. They are stated as
    /// alternatives only when the tempo is estimated: a given tempo is not reported as an
    /// estimate, so neither are its half and double.
    pub tempo_alternatives: (f64, f64),
    /// Frames per quarter note, the period notes are placed by. For an estimated tempo it is the
    /// period measured on the grid, not the rounded `tempo_bpm` (92.4 a minute is stated as 92
    /// but placed at 92.4); for a given tempo it is exactly `60 * rate / tempo_bpm`.
    pub beat_period_frames: f64,
    pub meter: Origin<TimeSig>,
    /// Where bar 1 starts, as a frame on the input timeline.
    pub downbeat_frame: Origin<u64>,
}

/// The local tempo from one beat to the next.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BeatTempo {
    /// The beat the interval starts on, counted from 0 at the downbeat.
    pub beat: u32,
    /// Where that beat is, in seconds on the input timeline.
    pub secs: f64,
    /// Quarter notes per minute.
    pub bpm: f64,
}

/// An onset, with its strength relative to the take's strongest.
#[derive(Debug, Clone, Copy)]
struct Onset {
    frame: f64,
    strength: f64,
}

/// Evenly spaced steps that the onsets fall on: step `k` is at `origin + k * period` frames.
#[derive(Debug, Clone, Copy)]
struct Grid {
    origin: f64,
    period: f64,
}

impl Grid {
    /// The step nearest `frame`, and how far off it `frame` is, as a share of a step.
    fn step(&self, frame: f64) -> (i64, f64) {
        let at = (frame - self.origin) / self.period;
        (at.round() as i64, (at - at.round()).abs())
    }

    fn frame(&self, step: i64) -> f64 {
        self.origin + step as f64 * self.period
    }
}

/// One reading of the accents: a meter, whose first beat 1 is `rotation` grid steps after the
/// take's first on-grid onset; how well it fits them; its tempo; and how plausible that is.
#[derive(Debug, Clone, Copy)]
struct Hypothesis {
    meter: TimeSig,
    rotation: usize,
    fit: f64,
    quarter_bpm: f64,
    prior: f64,
}

/// The take's tempo, meter and downbeat: each one given is used exactly, and each one left out
/// is estimated from `onsets` (the [`Event::Onset`]s among them; other events are ignored),
/// whose frames are at `rate`.
///
/// Fails with [`Error::TooFewOnsets`] when a tempo must be estimated from fewer than four
/// onsets, and with [`Error::Invalid`] when a given value is out of range or the onsets have no
/// steady pulse.
pub fn estimate(onsets: &[Event], rate: u32, given: &Given) -> Result<Timing> {
    if let Some(bpm) = given.tempo_bpm
        && !(bpm.is_finite() && bpm > 0.0)
    {
        return Err(Error::Invalid(format!(
            "a tempo must be a positive number of beats a minute, not {bpm}"
        )));
    }
    if let Some(secs) = given.downbeat_secs
        && !(secs.is_finite() && secs >= 0.0)
    {
        return Err(Error::Invalid(format!(
            "a downbeat must be a time from the start of the take, not {secs} s"
        )));
    }
    let onsets = relative(onsets);
    let rate = f64::from(rate);
    let grid = if onsets.len() >= MIN_ONSETS {
        pulse(&onsets, rate).map(|period| fit_grid(&onsets, period))
    } else {
        None
    };
    let Some(grid) = grid else {
        return match given.tempo_bpm {
            Some(bpm) => Ok(without_pulse(&onsets, rate, bpm, given)),
            None if onsets.len() < MIN_ONSETS => Err(Error::TooFewOnsets {
                found: onsets.len(),
            }),
            None => Err(Error::Invalid(
                "no two onsets recur at a steady pulse, so no tempo can be estimated; \
                 give one with --tempo"
                    .into(),
            )),
        };
    };

    let (chosen, downbeat) = read_meter(&onsets, &grid, rate, given);
    let confidence = chosen.fit as f32;
    let beat_period_frames = 60.0 * rate / given.tempo_bpm.unwrap_or(chosen.quarter_bpm);
    let tempo_bpm = match given.tempo_bpm {
        Some(bpm) => Origin::Given(bpm),
        None => Origin::Estimated {
            value: chosen.quarter_bpm.round(),
            confidence: on_grid_share(&onsets, &grid) as f32,
        },
    };
    Ok(Timing {
        tempo_alternatives: alternatives(*tempo_bpm.value()),
        tempo_bpm,
        beat_period_frames,
        meter: match given.meter {
            Some(meter) => Origin::Given(meter),
            None => Origin::Estimated {
                value: chosen.meter,
                confidence,
            },
        },
        downbeat_frame: downbeat_origin(given, rate, downbeat, confidence),
    })
}

/// The reading of the accents that the estimate applies, and where it puts bar 1.
///
/// Only a given meter is read, if there is one. Among readings that fit at least
/// [`WEAK_CONFIDENCE`], the best fit times tempo prior wins, and bar 1 is its first downbeat.
/// With none, the given meter or 4/4 is read at the beat level the prior favours, and bar 1 is
/// the first strong onset.
fn read_meter(onsets: &[Onset], grid: &Grid, rate: f64, given: &Given) -> (Hypothesis, f64) {
    let (first, accents) = accents(onsets, grid);
    let meters: Vec<TimeSig> = match given.meter {
        Some(meter) => vec![meter],
        None => CANDIDATES
            .iter()
            .map(|&(num, den)| TimeSig { num, den })
            .collect(),
    };
    let hypotheses: Vec<Hypothesis> = meters
        .iter()
        .flat_map(|&meter| {
            steps_per_beat(meter, given.meter.is_some())
                .iter()
                .map(move |&steps| (meter, steps))
        })
        .map(|(meter, steps)| hypothesis(meter, steps, &accents, grid, rate, given.tempo_bpm))
        .collect();
    let strong = hypotheses
        .iter()
        .filter(|h| h.fit >= f64::from(WEAK_CONFIDENCE))
        .max_by(|a, b| (a.fit * a.prior).total_cmp(&(b.fit * b.prior)));
    if let Some(h) = strong {
        // Accent `rotation` is the take's first step on beat 1.
        let step = first + h.rotation as i64;
        let downbeat = onset_at(onsets, grid, step).unwrap_or_else(|| grid.frame(step));
        return (*h, downbeat);
    }
    let fallback = given.meter.unwrap_or_default();
    let h = hypotheses
        .iter()
        .filter(|h| h.meter == fallback)
        .max_by(|a, b| a.prior.total_cmp(&b.prior).then(a.fit.total_cmp(&b.fit)))
        .expect("every meter has a hypothesis per step count");
    (*h, first_strong(onsets, Some(grid)))
}

/// The timing when a tempo is given but the onsets are too few, or too scattered, to show a
/// pulse: 4/4 unless a meter is given, and bar 1 at the first strong onset, both without
/// confidence.
fn without_pulse(onsets: &[Onset], rate: f64, bpm: f64, given: &Given) -> Timing {
    let downbeat = if onsets.is_empty() {
        0.0
    } else {
        first_strong(onsets, None)
    };
    Timing {
        tempo_bpm: Origin::Given(bpm),
        tempo_alternatives: alternatives(bpm),
        beat_period_frames: 60.0 * rate / bpm,
        meter: match given.meter {
            Some(meter) => Origin::Given(meter),
            None => Origin::Estimated {
                value: TimeSig::default(),
                confidence: 0.0,
            },
        },
        downbeat_frame: downbeat_origin(given, rate, downbeat, 0.0),
    }
}

fn alternatives(bpm: f64) -> (f64, f64) {
    (bpm / 2.0, bpm * 2.0)
}

fn downbeat_origin(given: &Given, rate: f64, estimate: f64, confidence: f32) -> Origin<u64> {
    match given.downbeat_secs {
        Some(secs) => Origin::Given((secs * rate).round() as u64),
        None => Origin::Estimated {
            value: estimate.max(0.0).round() as u64,
            confidence,
        },
    }
}

/// The onsets among `events`, in order, each with its strength relative to the strongest.
fn relative(events: &[Event]) -> Vec<Onset> {
    let mut onsets: Vec<Onset> = events
        .iter()
        .filter_map(|e| match e {
            Event::Onset { frame, strength } => Some(Onset {
                frame: *frame as f64,
                strength: f64::from(*strength),
            }),
            _ => None,
        })
        .collect();
    onsets.sort_by(|a, b| a.frame.total_cmp(&b.frame));
    let loudest = onsets
        .iter()
        .map(|o| o.strength)
        .fold(f64::MIN_POSITIVE, f64::max);
    for onset in &mut onsets {
        onset.strength /= loudest;
    }
    onsets
}

/// The pulse, in frames: the shortest lag at which the onsets' autocorrelation peaks within
/// [`PULSE_PEAK_SHARE`] of its highest, or `None` when no two onsets are a pulse apart.
fn pulse(onsets: &[Onset], rate: f64) -> Option<f64> {
    let bins = ((MAX_PULSE_SECS - MIN_PULSE_SECS) / LAG_STEP_SECS).round() as usize + 1;
    let reach = (3.0 * LAG_SPREAD_SECS / LAG_STEP_SECS).ceil() as isize;
    let mut correlation = vec![0.0; bins];
    for (i, a) in onsets.iter().enumerate() {
        for b in &onsets[i + 1..] {
            let lag = (b.frame - a.frame) / rate;
            if lag > MAX_PULSE_SECS + 3.0 * LAG_SPREAD_SECS {
                break;
            }
            let weight = (a.strength * b.strength).sqrt();
            let centre = ((lag - MIN_PULSE_SECS) / LAG_STEP_SECS).round() as isize;
            for bin in centre - reach..=centre + reach {
                let Some(slot) = usize::try_from(bin)
                    .ok()
                    .and_then(|i| correlation.get_mut(i))
                else {
                    continue;
                };
                let off = (MIN_PULSE_SECS + bin as f64 * LAG_STEP_SECS - lag) / LAG_SPREAD_SECS;
                *slot += weight * (-0.5 * off * off).exp();
            }
        }
    }
    let highest = correlation.iter().copied().fold(0.0, f64::max);
    if highest <= 0.0 {
        return None;
    }
    let peak = (1..bins - 1).find(|&i| {
        let c = correlation[i];
        c >= PULSE_PEAK_SHARE * highest && c >= correlation[i - 1] && c > correlation[i + 1]
    })?;
    Some((MIN_PULSE_SECS + peak as f64 * LAG_STEP_SECS) * rate)
}

/// The grid through the onsets at about `period` frames a step: of the grids anchored on each
/// of the [`ANCHOR_CANDIDATES`] strongest onsets, the one with the most onset strength on it.
fn fit_grid(onsets: &[Onset], period: f64) -> Grid {
    let mut by_strength: Vec<usize> = (0..onsets.len()).collect();
    by_strength.sort_by(|&a, &b| onsets[b].strength.total_cmp(&onsets[a].strength));
    by_strength
        .iter()
        .take(ANCHOR_CANDIDATES)
        .map(|&anchor| grid_from(onsets, period, anchor))
        .max_by(|a, b| on_grid_share(onsets, a).total_cmp(&on_grid_share(onsets, b)))
        .expect("a grid is fitted from at least one onset")
}

/// The grid through the onsets at about `period` frames a step, in phase with `anchor`.
///
/// Steps are first counted from the anchor, out to each side, from each on-grid onset to the
/// next, which a small error in `period` cannot throw off; a least-squares line through them
/// sets the grid. Then every onset is put on the nearest step of that grid, and the line is
/// fitted again.
fn grid_from(onsets: &[Onset], period: f64, anchor: usize) -> Grid {
    let mut counted = vec![(onsets[anchor].frame, 0)];
    count_steps(onsets[anchor + 1..].iter(), period, 1, &mut counted);
    count_steps(onsets[..anchor].iter().rev(), period, -1, &mut counted);
    let rough = least_squares(&counted).unwrap_or(Grid {
        origin: onsets[anchor].frame,
        period,
    });
    let placed: Vec<(f64, i64)> = onsets
        .iter()
        .filter_map(|o| {
            let (step, off) = rough.step(o.frame);
            (off <= GRID_TOLERANCE).then_some((o.frame, step))
        })
        .collect();
    least_squares(&placed).unwrap_or(rough)
}

/// Counts grid steps along `onsets`, which run away from `counted[0]` in the direction
/// `sign`, from each on-grid onset to the next, and appends each on-grid onset with its step.
fn count_steps<'a>(
    onsets: impl Iterator<Item = &'a Onset>,
    period: f64,
    sign: i64,
    counted: &mut Vec<(f64, i64)>,
) {
    let mut last = counted[0];
    for onset in onsets {
        let steps = (onset.frame - last.0).abs() / period;
        if steps.round() >= 1.0 && (steps - steps.round()).abs() <= GRID_TOLERANCE {
            last = (onset.frame, last.1 + sign * steps.round() as i64);
            counted.push(last);
        }
    }
}

/// The line `frame = origin + step * period` nearest the `(frame, step)` points, or `None`
/// when they share one step.
fn least_squares(points: &[(f64, i64)]) -> Option<Grid> {
    let n = points.len() as f64;
    let mean_step = points.iter().map(|p| p.1 as f64).sum::<f64>() / n;
    let mean_frame = points.iter().map(|p| p.0).sum::<f64>() / n;
    let (mut across, mut spread) = (0.0, 0.0);
    for &(frame, step) in points {
        let ds = step as f64 - mean_step;
        across += ds * (frame - mean_frame);
        spread += ds * ds;
    }
    (spread > 0.0).then(|| {
        let period = across / spread;
        Grid {
            origin: mean_frame - period * mean_step,
            period,
        }
    })
}

/// The strength-weighted share of onsets on the grid.
fn on_grid_share(onsets: &[Onset], grid: &Grid) -> f64 {
    let total: f64 = onsets.iter().map(|o| o.strength).sum();
    let on: f64 = onsets
        .iter()
        .filter(|o| grid.step(o.frame).1 <= GRID_TOLERANCE)
        .map(|o| o.strength)
        .sum();
    on / total
}

/// The accent on each grid step from the take's first on-grid onset on: the strength of the
/// strongest onset on it, relative to the strongest on-grid onset, or 0. Returns the first step
/// too.
fn accents(onsets: &[Onset], grid: &Grid) -> (i64, Vec<f64>) {
    let on: Vec<(i64, f64)> = onsets
        .iter()
        .filter_map(|o| {
            let (step, off) = grid.step(o.frame);
            (off <= GRID_TOLERANCE).then_some((step, o.strength))
        })
        .collect();
    let first = on.iter().map(|o| o.0).min().unwrap_or(0);
    let last = on.iter().map(|o| o.0).max().unwrap_or(0);
    let loudest = on.iter().map(|o| o.1).fold(f64::MIN_POSITIVE, f64::max);
    let mut accents = vec![0.0; (last - first + 1) as usize];
    for (step, strength) in on {
        let slot = &mut accents[(step - first) as usize];
        *slot = f64::max(*slot, strength / loudest);
    }
    (first, accents)
}

/// The frame of the strongest onset on grid step `step`, if one is.
fn onset_at(onsets: &[Onset], grid: &Grid, step: i64) -> Option<f64> {
    onsets
        .iter()
        .filter(|o| {
            let (on, off) = grid.step(o.frame);
            on == step && off <= GRID_TOLERANCE
        })
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
        .map(|o| o.frame)
}

/// The frame of the first onset at least [`STRONG_ONSET`] of the loudest, counting only onsets
/// on `grid` when there is one.
fn first_strong(onsets: &[Onset], grid: Option<&Grid>) -> f64 {
    let candidates: Vec<&Onset> = onsets
        .iter()
        .filter(|o| grid.is_none_or(|g| g.step(o.frame).1 <= GRID_TOLERANCE))
        .collect();
    let loudest = candidates.iter().map(|o| o.strength).fold(0.0, f64::max);
    candidates
        .iter()
        .find(|o| o.strength >= STRONG_ONSET * loudest)
        .map_or(0.0, |o| o.frame)
}

/// How well `meter`, with its felt beat `steps` grid steps long, reads the accents, at its best
/// rotation; and how plausible its tempo is.
fn hypothesis(
    meter: TimeSig,
    steps: u32,
    accents: &[f64],
    grid: &Grid,
    rate: f64,
    given_bpm: Option<f64>,
) -> Hypothesis {
    let template = template(meter, steps);
    let bar = template.len();
    let (rotation, correlation) = (0..bar)
        .map(|r| {
            let at = |k: usize| template[(k + bar - r) % bar];
            (r, pearson(accents, at))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a bar has at least one step");
    let felt_bpm = 60.0 * rate / (f64::from(steps) * grid.period);
    let quarter_bpm = felt_bpm * felt_quarters(meter);
    let prior = match given_bpm {
        Some(bpm) => gaussian((quarter_bpm / bpm).log2() / GIVEN_PRIOR_OCTAVES),
        None => gaussian((felt_bpm / PRIOR_BPM).log2() / PRIOR_OCTAVES),
    };
    Hypothesis {
        meter,
        rotation,
        fit: correlation.max(0.0) * (std_dev(accents) / ACCENT_SPREAD).min(1.0),
        quarter_bpm,
        prior,
    }
}

fn gaussian(z: f64) -> f64 {
    (-0.5 * z * z).exp()
}

/// The grid steps `meter`'s felt beat may span, when the meter is `given` or estimated.
fn steps_per_beat(meter: TimeSig, given: bool) -> &'static [u32] {
    match (is_compound(meter), given) {
        (false, _) => &SIMPLE_STEPS,
        (true, false) => &COMPOUND_STEPS,
        (true, true) => &GIVEN_COMPOUND_STEPS,
    }
}

/// Whether the meter's felt beat is a dotted note: 6/8, 9/8, 12/8 and the like. Only eighths
/// and shorter group in threes here: 6/4 is treated as simple, six quarter beats.
fn is_compound(meter: TimeSig) -> bool {
    meter.den >= 8 && meter.num > 3 && meter.num.is_multiple_of(3)
}

/// The felt beat, in quarter notes: a dotted quarter (1.5) in 6/8, a quarter in 4/4.
fn felt_quarters(meter: TimeSig) -> f64 {
    let beat = if is_compound(meter) { 3.0 } else { 1.0 };
    beat * 4.0 / f64::from(meter.den)
}

/// The accent a bar of `meter` expects on each grid step, when its felt beat is `steps` steps:
/// 1 on the downbeat, 0.5 on the middle beat of an even number of beats, 0.25 on the other
/// beats and 0 between beats.
fn template(meter: TimeSig, steps: u32) -> Vec<f64> {
    let beats = if is_compound(meter) {
        u32::from(meter.num) / 3
    } else {
        u32::from(meter.num)
    };
    (0..beats * steps)
        .map(|k| match (k % steps, k / steps) {
            (0, 0) => 1.0,
            (0, beat) if beats.is_multiple_of(2) && beat == beats / 2 => 0.5,
            (0, _) => 0.25,
            _ => 0.0,
        })
        .collect()
}

/// The correlation of `values` with `at(k)` for each index `k`, or 0 when either is flat.
fn pearson(values: &[f64], at: impl Fn(usize) -> f64) -> f64 {
    let n = values.len() as f64;
    let mean_v = values.iter().sum::<f64>() / n;
    let mean_t = (0..values.len()).map(&at).sum::<f64>() / n;
    let (mut across, mut spread_v, mut spread_t) = (0.0, 0.0, 0.0);
    for (k, &v) in values.iter().enumerate() {
        let (dv, dt) = (v - mean_v, at(k) - mean_t);
        across += dv * dt;
        spread_v += dv * dv;
        spread_t += dt * dt;
    }
    if spread_v <= 0.0 || spread_t <= 0.0 {
        return 0.0;
    }
    across / (spread_v * spread_t).sqrt()
}

fn std_dev(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n).sqrt()
}

/// The local tempo from each beat to the next, from the downbeat on.
///
/// The beats are the meter's felt beats (dotted quarters in 6/8), and their tempo is stated in
/// quarter notes per minute.
///
/// Beats are tracked from `timing`'s downbeat: each is expected one beat period after the last,
/// and is the onset nearest that, if one is within `SNAP_TOLERANCE` of a beat; otherwise it
/// is where it was expected. The period starts at `timing`'s measured beat period and moves
/// `PERIOD_ADAPT` of the way to each snapped interval, so it follows a slowly changing tempo
/// rather than holding the global one. Tracking stops when the next beat would fall past the
/// last onset.
///
/// The onset stage places onsets on a grid of about 6 ms, which is a percent of a beat at 120,
/// so an interval's tempo is not read from its two ends alone: it is the slope, at the
/// interval's middle, of a least-squares quadratic through the `FIT_BEATS` snapped beats nearest
/// it (a Savitzky-Golay derivative), which keeps a slow drift and averages out that grain.
/// Beats in a rest are placed where they were expected but are not fitted.
pub fn beat_series(onsets: &[Event], rate: u32, timing: &Timing) -> Vec<BeatTempo> {
    let rate = f64::from(rate);
    let quarters = felt_quarters(*timing.meter.value());
    let frames: Vec<f64> = relative(onsets).iter().map(|o| o.frame).collect();
    let beats = track(
        &frames,
        *timing.downbeat_frame.value() as f64,
        timing.beat_period_frames * quarters,
    );
    (0..beats.len().saturating_sub(1))
        .map(|i| BeatTempo {
            beat: i as u32,
            secs: beats[i].frame / rate,
            bpm: 60.0 * rate * quarters / local_period(&beats, i),
        })
        .collect()
}

/// A tracked beat: its frame, and whether an onset is on it, or it was only expected there.
#[derive(Debug, Clone, Copy)]
struct Beat {
    frame: f64,
    snapped: bool,
}

/// The beats from `start`, at about `period` frames apart, snapped to `frames`.
fn track(frames: &[f64], start: f64, mut period: f64) -> Vec<Beat> {
    let Some(&end) = frames.last() else {
        return Vec::new();
    };
    let mut beats = vec![Beat {
        frame: start,
        snapped: frames.iter().any(|f| (f - start).abs() < 1.0),
    }];
    loop {
        let last = beats[beats.len() - 1].frame;
        let expected = last + period;
        let reach = SNAP_TOLERANCE * period;
        if expected > end + reach {
            break;
        }
        let snapped = frames
            .iter()
            .copied()
            .filter(|f| (f - expected).abs() <= reach)
            .min_by(|a, b| (a - expected).abs().total_cmp(&(b - expected).abs()));
        match snapped {
            Some(frame) => {
                period += PERIOD_ADAPT * (frame - last - period);
                beats.push(Beat {
                    frame,
                    snapped: true,
                });
            }
            None => beats.push(Beat {
                frame: expected,
                snapped: false,
            }),
        }
    }
    beats
}

/// The frames per beat at the middle of the interval from beat `i` to beat `i + 1`, fitted
/// over the [`FIT_BEATS`] snapped beats nearest it. Beats only expected, in a rest, are left
/// out: they are guesses at the tempo, not measurements of it.
fn local_period(beats: &[Beat], i: usize) -> f64 {
    let middle = i as f64 + 0.5;
    let mut measured: Vec<(f64, f64)> = beats
        .iter()
        .enumerate()
        .filter(|(_, b)| b.snapped)
        .map(|(k, b)| (k as f64 - middle, b.frame))
        .collect();
    measured.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
    measured.truncate(FIT_BEATS);
    match measured.len() {
        0 | 1 => beats[i + 1].frame - beats[i].frame,
        2 => (measured[1].1 - measured[0].1) / (measured[1].0 - measured[0].0),
        _ => quadratic_slope_at_zero(&measured),
    }
}

/// The slope at x = 0 of the least-squares quadratic through `points`.
fn quadratic_slope_at_zero(points: &[(f64, f64)]) -> f64 {
    // Normal equations for y = a + b x + c x², solved for b by Cramer's rule.
    let sum = |f: &dyn Fn(f64, f64) -> f64| points.iter().map(|&(x, y)| f(x, y)).sum::<f64>();
    let s0 = points.len() as f64;
    let s1 = sum(&|x, _| x);
    let s2 = sum(&|x, _| x * x);
    let s3 = sum(&|x, _| x * x * x);
    let s4 = sum(&|x, _| x * x * x * x);
    let t0 = sum(&|_, y| y);
    let t1 = sum(&|x, y| x * y);
    let t2 = sum(&|x, y| x * x * y);
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let whole = [[s0, s1, s2], [s1, s2, s3], [s2, s3, s4]];
    let for_b = [[s0, t0, s2], [s1, t1, s3], [s2, t2, s4]];
    det(for_b) / det(whole)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onset::Onsets;
    use crate::stage::{Mono, Stage};
    use crate::synth;

    const RATE: u32 = 22_050;
    /// Where the test patterns start. A click at frame 0 meets the onset stage's first window
    /// where its taper is zero, so it would not be heard.
    const LEAD: f64 = 0.25;

    fn meter(s: &str) -> TimeSig {
        s.parse().unwrap()
    }

    /// The onsets the onset stage finds in `signal`.
    fn onsets_of(signal: &[f32]) -> Vec<Event> {
        let mut stage = Onsets::new(RATE);
        let mut out = Vec::new();
        let mono = Mono {
            rate: RATE,
            start_frame: 0,
            samples: signal.to_vec(),
        };
        stage.push(mono, &mut out).unwrap();
        stage.finish(&mut out).unwrap();
        out
    }

    /// Clicks every `60 / bpm` seconds from `from` until `secs`, with `amplitudes` cycling
    /// over them, so the first click has `amplitudes[0]`.
    fn pattern(bpm: f64, from: f64, secs: f64, amplitudes: &[f32]) -> Vec<f32> {
        let period = 60.0 / bpm;
        let mut signal = synth::silence(RATE, secs + 0.5);
        for (i, &amplitude) in amplitudes.iter().enumerate() {
            let times: Vec<f64> = (0..)
                .map(|k| from + (k * amplitudes.len() + i) as f64 * period)
                .take_while(|&t| t < secs)
                .collect();
            synth::add(
                &mut signal,
                &synth::clicks(RATE, secs + 0.5, &times, amplitude),
            );
        }
        signal
    }

    /// Clicks whose tempo slides in a straight line, in time, from `start_bpm` at the first
    /// click, [`LEAD`] in, to `end_bpm` `secs` later.
    fn sliding(start_bpm: f64, end_bpm: f64, secs: f64) -> Vec<f32> {
        synth::clicks(
            RATE,
            secs + LEAD + 0.5,
            &sliding_times(start_bpm, end_bpm, secs),
            0.5,
        )
    }

    /// The click times of [`sliding`].
    fn sliding_times(start_bpm: f64, end_bpm: f64, secs: f64) -> Vec<f64> {
        // Beats elapsed by t: (start t + slope t² / 2) / 60, with slope in bpm per second.
        let half_slope = (end_bpm - start_bpm) / secs / 2.0;
        (0..)
            .map(|k| {
                let beats = f64::from(k);
                (-start_bpm + (start_bpm * start_bpm + 4.0 * half_slope * 60.0 * beats).sqrt())
                    / (2.0 * half_slope)
                    + LEAD
            })
            .take_while(|&t| t < secs + LEAD)
            .collect()
    }

    fn steady_120() -> Vec<f32> {
        pattern(120.0, LEAD, 20.0, &[0.5])
    }

    fn the_onset_nearest(onsets: &[Event], secs: f64) -> u64 {
        let target = secs * f64::from(RATE);
        onsets
            .iter()
            .filter_map(|e| match e {
                Event::Onset { frame, .. } => Some(*frame),
                _ => None,
            })
            .min_by(|a, b| {
                (*a as f64 - target)
                    .abs()
                    .total_cmp(&(*b as f64 - target).abs())
            })
            .unwrap()
    }

    fn estimated<T: Copy + std::fmt::Debug>(origin: &Origin<T>) -> (T, f32) {
        match origin {
            Origin::Estimated { value, confidence } => (*value, *confidence),
            Origin::Given(_) => panic!("expected an estimate, got {origin:?}"),
        }
    }

    #[test]
    fn a_given_tempo_and_meter_are_kept_exactly() {
        let onsets = onsets_of(&pattern(120.0, LEAD, 20.0, &[0.5, 0.15, 0.15, 0.15]));
        let given = Given {
            tempo_bpm: Some(97.0),
            meter: Some(meter("3/4")),
            downbeat_secs: None,
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        assert_eq!(timing.tempo_bpm, Origin::Given(97.0));
        assert_eq!(timing.meter, Origin::Given(meter("3/4")));
        assert_eq!(timing.tempo_alternatives, (48.5, 194.0));
    }

    #[test]
    fn a_fractional_given_tempo_is_kept_exactly() {
        let onsets = onsets_of(&steady_120());
        let given = Given {
            tempo_bpm: Some(97.25),
            ..Given::default()
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        assert_eq!(timing.tempo_bpm, Origin::Given(97.25));
    }

    #[test]
    fn a_120_click_is_estimated_at_120_with_half_and_double() {
        let timing = estimate(&onsets_of(&steady_120()), RATE, &Given::default()).unwrap();
        let (bpm, confidence) = estimated(&timing.tempo_bpm);
        assert_eq!(bpm, 120.0);
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
        assert_eq!(timing.tempo_alternatives, (60.0, 240.0));
    }

    #[test]
    fn an_estimate_is_a_whole_bpm() {
        let onsets = onsets_of(&pattern(92.4, LEAD, 20.0, &[0.5]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 92.0);
        assert_eq!(timing.tempo_alternatives, (46.0, 184.0));
    }

    #[test]
    fn six_eight_at_dotted_quarter_90_is_135_quarter_bpm() {
        // Eighths at 270 a minute: three to a dotted quarter of 90.
        let accents = [0.5, 0.15, 0.15, 0.3, 0.15, 0.15];
        let onsets = onsets_of(&pattern(270.0, LEAD, 20.0, &accents));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let (sig, confidence) = estimated(&timing.meter);
        assert_eq!(sig, meter("6/8"));
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
        assert_eq!(estimated(&timing.tempo_bpm).0, 135.0);
    }

    #[test]
    fn three_four_accents_are_found() {
        let onsets = onsets_of(&pattern(100.0, LEAD, 20.0, &[0.5, 0.15, 0.15]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let (sig, confidence) = estimated(&timing.meter);
        assert_eq!(sig, meter("3/4"));
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
        assert_eq!(estimated(&timing.tempo_bpm).0, 100.0);
    }

    #[test]
    fn four_four_accents_are_found() {
        let onsets = onsets_of(&pattern(120.0, LEAD, 20.0, &[0.5, 0.15, 0.3, 0.15]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let (sig, confidence) = estimated(&timing.meter);
        assert_eq!(sig, meter("4/4"));
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
        assert_eq!(estimated(&timing.tempo_bpm).0, 120.0);
    }

    #[test]
    fn weak_evidence_gives_four_four_at_low_confidence() {
        let timing = estimate(&onsets_of(&steady_120()), RATE, &Given::default()).unwrap();
        let (sig, confidence) = estimated(&timing.meter);
        assert_eq!(sig, meter("4/4"));
        assert!(confidence < WEAK_CONFIDENCE, "confidence {confidence}");
    }

    #[test]
    fn one_onset_is_too_few() {
        let onsets = onsets_of(&synth::clicks(RATE, 2.0, &[0.5], 0.5));
        assert_eq!(onsets.len(), 1, "{onsets:?}");
        let err = estimate(&onsets, RATE, &Given::default()).unwrap_err();
        assert!(
            matches!(err, Error::TooFewOnsets { found: 1 }),
            "got {err:?}"
        );
        assert!(err.to_string().contains("--tempo"), "{err}");
        assert!(err.to_string().starts_with("found 1 onset,"), "{err}");
    }

    #[test]
    fn a_given_tempo_needs_no_pulse() {
        let onsets = onsets_of(&synth::clicks(RATE, 2.0, &[0.5], 0.5));
        let given = Given {
            tempo_bpm: Some(100.0),
            ..Given::default()
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        assert_eq!(timing.tempo_bpm, Origin::Given(100.0));
        assert_eq!(estimated(&timing.meter), (meter("4/4"), 0.0));
        assert_eq!(
            estimated(&timing.downbeat_frame).0,
            the_onset_nearest(&onsets, 0.5)
        );
    }

    #[test]
    fn onsets_with_no_steady_pulse_ask_for_a_tempo() {
        // Five clicks, each further apart than the slowest pulse looked for.
        let onsets = onsets_of(&synth::clicks(RATE, 16.0, &[0.5, 3.5, 6.5, 9.5, 12.5], 0.5));
        assert_eq!(onsets.len(), 5, "{onsets:?}");
        let err = estimate(&onsets, RATE, &Given::default()).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
        assert!(err.to_string().contains("--tempo"), "{err}");
    }

    #[test]
    fn a_given_meter_sets_the_beat_the_tempo_is_counted_in() {
        // A click on the felt beat only, 90 a minute: in 4/4 that is 90, and in 6/8 it is
        // dotted quarters, so 135 quarter bpm.
        let onsets = onsets_of(&pattern(90.0, LEAD, 20.0, &[0.5]));
        let alone = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&alone.tempo_bpm).0, 90.0);
        let given = Given {
            meter: Some(meter("6/8")),
            ..Given::default()
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        assert_eq!(timing.meter, Origin::Given(meter("6/8")));
        assert_eq!(estimated(&timing.tempo_bpm).0, 135.0);
    }

    #[test]
    fn a_tempo_that_is_not_a_positive_number_is_refused() {
        let onsets = onsets_of(&steady_120());
        for bad in [0.0, -120.0, f64::NAN, f64::INFINITY] {
            let given = Given {
                tempo_bpm: Some(bad),
                ..Given::default()
            };
            let err = estimate(&onsets, RATE, &given).unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{bad}: got {err:?}");
        }
    }

    #[test]
    fn the_first_strong_onset_is_the_downbeat() {
        // Two quiet clicks lead in to an unaccented click from 1.25 s on.
        let mut signal = synth::clicks(RATE, 20.5, &[0.25, 0.75], 0.05);
        synth::add(&mut signal, &pattern(120.0, 1.25, 20.0, &[0.5]));
        let onsets = onsets_of(&signal);
        assert!(
            the_onset_nearest(&onsets, 0.25) < the_onset_nearest(&onsets, 0.75)
                && the_onset_nearest(&onsets, 0.75) < the_onset_nearest(&onsets, 1.25),
            "the quiet lead-in was not detected: {onsets:?}"
        );
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let (frame, _) = estimated(&timing.downbeat_frame);
        assert_eq!(frame, the_onset_nearest(&onsets, 1.25));
    }

    #[test]
    fn accents_place_the_downbeat() {
        // Beats 3 and 4 of a bar lead in to bars of 4/4 whose first beat is at 1.25 s.
        let accents = [0.3, 0.15, 0.5, 0.15];
        let onsets = onsets_of(&pattern(120.0, LEAD, 20.0, &accents));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.meter).0, meter("4/4"));
        let (frame, confidence) = estimated(&timing.downbeat_frame);
        assert_eq!(frame, the_onset_nearest(&onsets, 1.25));
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
    }

    #[test]
    fn a_stray_onset_before_the_take_moves_neither_grid_nor_downbeat() {
        // A noise a tenth of a second in, off the beat, before the bars of
        // `accents_place_the_downbeat`.
        let mut signal = synth::clicks(RATE, 20.5, &[0.1], 0.15);
        synth::add(
            &mut signal,
            &pattern(120.0, LEAD, 20.0, &[0.3, 0.15, 0.5, 0.15]),
        );
        let onsets = onsets_of(&signal);
        assert!(
            the_onset_nearest(&onsets, 0.1) < the_onset_nearest(&onsets, LEAD),
            "the stray onset was not detected: {onsets:?}"
        );
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 120.0);
        assert_eq!(estimated(&timing.meter).0, meter("4/4"));
        assert_eq!(
            estimated(&timing.downbeat_frame).0,
            the_onset_nearest(&onsets, 1.25)
        );
    }

    /// The frames of `onsets`.
    fn frames(onsets: &[Event]) -> Vec<u64> {
        onsets
            .iter()
            .filter_map(|e| match e {
                Event::Onset { frame, .. } => Some(*frame),
                _ => None,
            })
            .collect()
    }

    /// Whether `beat` lies on one of `onsets`, as a snapped beat does.
    fn is_snapped(beat: &BeatTempo, onsets: &[u64]) -> bool {
        onsets
            .iter()
            .any(|&f| (f as f64 / f64::from(RATE) - beat.secs).abs() < 1e-9)
    }

    #[test]
    fn a_stray_onset_louder_than_the_accents_moves_neither_grid_nor_downbeat() {
        // A burst of noise a tenth of a second in, off the beat and the loudest onset in the take,
        // before the bars of `accents_place_the_downbeat`.
        let mut signal = synth::noise_bursts(RATE, 20.5, &[(0.1, 0.04)], 200.0, 8_000.0, 0.5);
        synth::add(
            &mut signal,
            &pattern(120.0, LEAD, 20.0, &[0.3, 0.15, 0.5, 0.15]),
        );
        let onsets = onsets_of(&signal);
        let strengths: Vec<(u64, f32)> = onsets
            .iter()
            .filter_map(|e| match e {
                Event::Onset { frame, strength } => Some((*frame, *strength)),
                _ => None,
            })
            .collect();
        let loudest = strengths.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(
            (loudest.0 as f64 / f64::from(RATE) - 0.1).abs() < 0.03,
            "the stray is not the loudest onset: {strengths:?}"
        );
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 120.0);
        let half_second = 0.5 * f64::from(RATE);
        assert!(
            (timing.beat_period_frames / half_second - 1.0).abs() < 0.001,
            "{timing:?}"
        );
        let (sig, confidence) = estimated(&timing.meter);
        assert_eq!(sig, meter("4/4"));
        assert!(confidence >= WEAK_CONFIDENCE, "confidence {confidence}");
        assert_eq!(
            estimated(&timing.downbeat_frame).0,
            the_onset_nearest(&onsets, 1.25)
        );
    }

    #[test]
    fn loud_syncopations_leave_the_downbeat_on_the_beat() {
        // A quiet, steady beat at 120 from 0.25 s, with four loud onsets halfway between beats.
        let off_beats = [2.0, 5.0, 9.0, 13.0];
        let mut signal = pattern(120.0, LEAD, 20.0, &[0.15]);
        synth::add(&mut signal, &synth::clicks(RATE, 20.5, &off_beats, 0.5));
        let onsets = onsets_of(&signal);
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 120.0);
        assert_eq!(
            estimated(&timing.downbeat_frame).0,
            the_onset_nearest(&onsets, LEAD)
        );
        let series = beat_series(&onsets, RATE, &timing);
        assert!(series.len() >= 38, "{} beats", series.len());
        for beat in &series {
            let from_beat = (beat.secs - LEAD) / 0.5;
            assert!((from_beat - from_beat.round()).abs() < 0.1, "{beat:?}");
        }
    }

    #[test]
    fn notes_are_placed_by_the_measured_period_not_the_rounded_tempo() {
        let onsets = onsets_of(&pattern(92.4, LEAD, 20.0, &[0.5]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 92.0);
        let measured = 60.0 * f64::from(RATE) / 92.4;
        assert!(
            (timing.beat_period_frames / measured - 1.0).abs() < 0.001,
            "{} frames, wanted {measured}",
            timing.beat_period_frames
        );
    }

    #[test]
    fn a_given_tempo_sets_the_period_exactly() {
        let given = Given {
            tempo_bpm: Some(97.25),
            ..Given::default()
        };
        let timing = estimate(&onsets_of(&steady_120()), RATE, &given).unwrap();
        assert_eq!(timing.beat_period_frames, 60.0 * f64::from(RATE) / 97.25);
    }

    #[test]
    fn unaccented_eighths_at_120_read_as_120() {
        let onsets = onsets_of(&pattern(240.0, LEAD, 20.0, &[0.5]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 120.0);
    }

    #[test]
    fn a_six_eight_series_tracks_the_dotted_quarter_in_quarter_bpm() {
        let accents = [0.5, 0.15, 0.15, 0.3, 0.15, 0.15];
        let onsets = onsets_of(&pattern(270.0, LEAD, 20.0, &accents));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let series = beat_series(&onsets, RATE, &timing);
        // 20 s of dotted quarters at 90 a minute.
        assert!(series.len() >= 28, "{} beats", series.len());
        for beat in &series {
            assert!((beat.bpm - 135.0).abs() <= 1.0, "{beat:?}");
        }
        for pair in series.windows(2) {
            assert!(
                (pair[1].secs - pair[0].secs - 60.0 / 90.0).abs() < 0.01,
                "{pair:?}"
            );
        }
        let frames = frames(&onsets);
        let snapped = series.iter().filter(|b| is_snapped(b, &frames)).count();
        assert!(
            snapped * 10 >= series.len() * 9,
            "{snapped} of {} snapped",
            series.len()
        );
    }

    #[test]
    fn the_tracked_period_follows_a_drift_through_a_rest() {
        // 100 to 112 bpm over 20 s, with the clicks between 15.5 and 18 s left out, about four
        // beats: the beat carries on through the rest at the tempo it had reached (about 110),
        // where the take's average (about 106) would lose it.
        let times: Vec<f64> = sliding_times(100.0, 112.0, 20.0)
            .into_iter()
            .filter(|&t| !(15.5..18.0).contains(&t))
            .collect();
        let onsets = onsets_of(&synth::clicks(RATE, 20.0 + LEAD + 0.5, &times, 0.5));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let series = beat_series(&onsets, RATE, &timing);
        let frames = frames(&onsets);
        let after_rest: Vec<&BeatTempo> = series.iter().filter(|b| b.secs > 18.0).collect();
        assert!(after_rest.len() >= 3, "{series:?}");
        for beat in &after_rest {
            assert!(
                is_snapped(beat, &frames),
                "{beat:?} is not on an onset: {series:?}"
            );
        }
        let last = series.last().unwrap().bpm;
        assert!((last - 112.0).abs() <= 1.0, "{series:?}");
    }

    #[test]
    fn a_given_downbeat_is_kept() {
        let given = Given {
            downbeat_secs: Some(0.75),
            ..Given::default()
        };
        let timing = estimate(&onsets_of(&steady_120()), RATE, &given).unwrap();
        assert_eq!(timing.downbeat_frame, Origin::Given(16_538));
    }

    #[test]
    fn a_steady_click_has_a_flat_beat_series() {
        let onsets = onsets_of(&steady_120());
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let series = beat_series(&onsets, RATE, &timing);
        assert!(series.len() >= 38, "{} beats", series.len());
        for beat in &series {
            assert!((beat.bpm - 120.0).abs() <= 1.2, "{beat:?}");
        }
    }

    #[test]
    fn a_drifting_click_shows_its_drift() {
        let onsets = onsets_of(&sliding(100.0, 106.0, 20.0));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let series = beat_series(&onsets, RATE, &timing);
        assert!(series.len() >= 30, "{} beats", series.len());
        let bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
        for &bpm in &bpms[..3] {
            assert!((bpm - 100.0).abs() <= 1.0, "{bpms:?}");
        }
        for &bpm in &bpms[bpms.len() - 3..] {
            assert!((bpm - 106.0).abs() <= 1.0, "{bpms:?}");
        }
        assert!(bpms.windows(2).all(|w| w[1] > w[0]), "{bpms:?}");
    }

    #[test]
    fn beats_count_from_the_downbeat_in_order() {
        let onsets = onsets_of(&steady_120());
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        let series = beat_series(&onsets, RATE, &timing);
        let downbeat = *timing.downbeat_frame.value() as f64 / f64::from(RATE);
        assert_eq!(series[0].beat, 0);
        assert!((series[0].secs - downbeat).abs() < 1e-9, "{:?}", series[0]);
        for pair in series.windows(2) {
            assert_eq!(pair[1].beat, pair[0].beat + 1);
            assert!((pair[1].secs - pair[0].secs - 0.5).abs() < 0.01, "{pair:?}");
        }
    }

    #[test]
    fn timing_round_trips_through_json() {
        let timing = Timing {
            tempo_bpm: Origin::Estimated {
                value: 135.0,
                confidence: 0.75,
            },
            tempo_alternatives: (67.5, 270.0),
            beat_period_frames: 9_800.25,
            meter: Origin::Given(meter("6/8")),
            downbeat_frame: Origin::Estimated {
                value: 22_050,
                confidence: 0.25,
            },
        };
        let json = serde_json::to_string(&timing).unwrap();
        let back: Timing = serde_json::from_str(&json).unwrap();
        assert_eq!(back, timing);
        assert!(json.contains(r#""meter":{"given":"6/8"}"#), "{json}");
        let beat = BeatTempo {
            beat: 3,
            secs: 1.5,
            bpm: 120.5,
        };
        let back: BeatTempo = serde_json::from_str(&serde_json::to_string(&beat).unwrap()).unwrap();
        assert_eq!(back, beat);
    }
}
