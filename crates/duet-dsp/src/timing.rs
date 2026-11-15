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
//!    With a tempo given, the pulse does not set the step: the grid's step is the given beat,
//!    or its half, quarter or double, whichever is nearest the pulse ([`GIVEN_LEVELS`]), and
//!    only its phase is fitted.
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
//!    on-grid onset. A reading that would put the downbeat after the last onset is no reading
//!    of the take, and is passed over.
//!
//! Confidence is in 0..=1, and below [`WEAK_CONFIDENCE`] an estimate is weak. A tempo's is the
//! lesser of the strength-weighted share of onsets on its grid and the margin of its beat level:
//! how clearly the chosen reading beat the readings of its meter at half and double its tempo,
//! by the measure it was chosen by (fit times prior, or the prior alone when no reading fit), as
//! the base-2 log of the ratio, so twice the score is a margin of 1 and an equal score 0. A
//! grid every onset is on still says nothing about whether its beat is the quarter or the half. A meter's is its accent fit, and a downbeat's
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
/// The grid steps a given tempo's grid may take, as shares of its beat: the beat itself, its
/// half and quarter, and its double. The one nearest the onsets' pulse is taken, so a take in
/// eighths is read on an eighth grid, but the tempo, not the pulse, sets the step.
const GIVEN_LEVELS: [f64; 4] = [0.25, 0.5, 1.0, 2.0];
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

/// How sharply the beat tracker penalises a span between beat onsets that is not a whole number
/// of beat periods: the cost is this times the square of how far, in beats, the span ends from
/// the nearest whole number, against a gain of each beat onset's relative strength (at most 1).
/// Measured in beats, not as a ratio, a half-beat slip costs the same however many beats the
/// span holds: 7.5, more than any onset gains, so no sequence slips onto the off-beats to collect
/// them, even across a rest. A one-beat interval 15% off costs about 0.7, so a take played that
/// far from the period, or drifting that far, is still followed on its strong onsets.
const TIGHTNESS: f64 = 30.0;
/// How far, in beats, a span between beat onsets may end from a whole number of beats: a quarter
/// of a beat, halfway to the off-beat. A span ending further out lands nearer an off-beat than a
/// beat, so it is not a span from beat to beat at all. A cost alone cannot keep a sequence off
/// the off-beats: where they are louder than the beats, one slip pays for itself in a few bars.
/// A span across a rest, with no onset inside it, may end this far out per beat it spans, since
/// a tempo that moved during the rest shifts where the beat comes back; there a slip and a drift
/// look alike.
const MAX_BEND_BEATS: f64 = 0.25;
/// The most beats one interval of the tracked sequence may span by choice, the ones between left
/// as rests: two bars of 4/4. An onset after a longer rest follows on from the last one before it.
const MAX_SPAN_BEATS: f64 = 8.0;
/// How far from the given or estimated beat period the period a take kept may be, as a ratio
/// either way, and still be its tempo rather than another beat level: halfway, in log, to the
/// nearest other level, four-thirds (a beat of an eighth and a half, which a tresillo's taps all
/// fall on). It covers a take meant at 120 and played at 108.
const KEPT_RANGE: f64 = 1.1547;
/// How far from a step of the beat's division, as a share of the step, an onset counts toward
/// the period a take kept.
const SNAP_TOLERANCE: f64 = 0.2;
/// How many chance spreads a period's fit must lead the given or estimated period's by to
/// replace it. A rule of thumb, not a significance level: the search compares the hint with
/// every period in its range, and the best of many chance differences crosses two spreads far
/// more often than one does, so three; but fits at nearby periods are correlated, and the chance
/// spread treats two periods' fits as independent, so the rate at which chance alone replaces the
/// hint is not exactly known.
const CLEAR_LEAD: f64 = 3.0;
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
    /// Half and double an estimated tempo, the likeliest other readings of it. `None` for a
    /// given tempo, which is not reported as an estimate, so neither are its half and double,
    /// and the analysis leaves them out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_alternatives: Option<(f64, f64)>,
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
        pulse(&onsets, rate).map(|pulse| match given.tempo_bpm {
            Some(bpm) => given_grid(&onsets, 60.0 * rate / bpm, pulse),
            None => fit_grid(&onsets, pulse),
        })
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

    let (chosen, downbeat, margin) = read_meter(&onsets, &grid, rate, given);
    let confidence = chosen.fit as f32;
    let beat_period_frames = 60.0 * rate / given.tempo_bpm.unwrap_or(chosen.quarter_bpm);
    let tempo_bpm = match given.tempo_bpm {
        Some(bpm) => Origin::Given(bpm),
        None => Origin::Estimated {
            value: chosen.quarter_bpm.round(),
            confidence: on_grid_share(&onsets, &grid).min(margin) as f32,
        },
    };
    Ok(Timing {
        tempo_alternatives: match tempo_bpm {
            Origin::Given(_) => None,
            Origin::Estimated { value, .. } => Some(alternatives(value)),
        },
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

/// The reading of the accents that the estimate applies, where it puts bar 1, and how clearly
/// it beat its octave alternatives ([`octave_margin`]).
///
/// Only a given meter is read, if there is one. Among readings that fit at least
/// [`WEAK_CONFIDENCE`], the best fit times tempo prior wins, and bar 1 is its first downbeat.
/// With none, the given meter or 4/4 is read at the beat level the prior favours, and bar 1 is
/// the first strong onset.
fn read_meter(onsets: &[Onset], grid: &Grid, rate: f64, given: &Given) -> (Hypothesis, f64, f64) {
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
    let last = onsets.last().map_or(0.0, |o| o.frame);
    let strong = hypotheses
        .iter()
        .filter(|h| h.fit >= f64::from(WEAK_CONFIDENCE))
        .filter_map(|h| {
            // Accent `rotation` is the take's first step on beat 1.
            let step = first + h.rotation as i64;
            let downbeat = onset_at(onsets, grid, step).unwrap_or_else(|| grid.frame(step));
            // A bar 1 after every note is no reading of the take's accents.
            (downbeat <= last).then_some((h, downbeat))
        })
        .max_by(|a, b| (a.0.fit * a.0.prior).total_cmp(&(b.0.fit * b.0.prior)));
    if let Some((h, downbeat)) = strong {
        let margin = octave_margin(h, &hypotheses, |h| h.fit * h.prior);
        return (*h, downbeat, margin);
    }
    let fallback = given.meter.unwrap_or_default();
    let h = hypotheses
        .iter()
        .filter(|h| h.meter == fallback)
        .max_by(|a, b| a.prior.total_cmp(&b.prior).then(a.fit.total_cmp(&b.fit)))
        .expect("every meter has a hypothesis per step count");
    let margin = octave_margin(h, &hypotheses, |h| h.prior);
    (*h, first_strong(onsets, Some(grid)), margin)
}

/// How clearly `chosen` beat its octave alternatives, the hypotheses of its meter at half or
/// double its tempo, by `score`, the measure it was chosen by: the base-2 log of its score over
/// the best of theirs, in 0..=1. It is 1 when it scores twice theirs or more, or has none, and
/// 0 when one of them scores as well or better, as one that fit too weakly to be chosen can.
fn octave_margin(
    chosen: &Hypothesis,
    hypotheses: &[Hypothesis],
    score: impl Fn(&Hypothesis) -> f64,
) -> f64 {
    let rival = hypotheses
        .iter()
        .filter(|h| {
            h.meter == chosen.meter
                && ((h.quarter_bpm / chosen.quarter_bpm).log2().abs() - 1.0).abs() < 0.01
        })
        .map(&score)
        .fold(0.0, f64::max);
    if rival <= 0.0 {
        return 1.0;
    }
    (score(chosen) / rival).log2().clamp(0.0, 1.0)
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
        tempo_alternatives: None,
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

/// The grid at a given tempo, whose beat is `beat` frames: a step of that beat, or of its half,
/// quarter or double, whichever is nearest `pulse` ([`GIVEN_LEVELS`]). Only its phase is fitted:
/// of the grids anchored on each of the [`ANCHOR_CANDIDATES`] strongest onsets, the one with the
/// most onset strength on it, moved to the mean phase of the onsets on it.
fn given_grid(onsets: &[Onset], beat: f64, pulse: f64) -> Grid {
    let off = |period: f64| (period / pulse).log2().abs();
    let period = GIVEN_LEVELS
        .iter()
        .map(|level| beat * level)
        .fold(beat, |best, p| if off(p) < off(best) { p } else { best });
    let mut by_strength: Vec<usize> = (0..onsets.len()).collect();
    by_strength.sort_by(|&a, &b| onsets[b].strength.total_cmp(&onsets[a].strength));
    let anchored = |&anchor: &usize| {
        let rough = Grid {
            origin: onsets[anchor].frame,
            period,
        };
        let phases: Vec<f64> = onsets
            .iter()
            .filter_map(|o| {
                let (step, off) = rough.step(o.frame);
                (off <= GRID_TOLERANCE).then_some(o.frame - step as f64 * period)
            })
            .collect();
        Grid {
            origin: phases.iter().sum::<f64>() / phases.len() as f64,
            period,
        }
    };
    by_strength
        .iter()
        .take(ANCHOR_CANDIDATES)
        .map(anchored)
        .fold(None, |best: Option<Grid>, grid| match best {
            Some(b) if on_grid_share(onsets, &b) >= on_grid_share(onsets, &grid) => Some(b),
            _ => Some(grid),
        })
        .unwrap_or(Grid {
            origin: onsets.first().map_or(0.0, |o| o.frame),
            period,
        })
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
///
/// The estimate reads accents by this rule, so anything that states a felt beat uses it too.
pub fn is_compound(meter: TimeSig) -> bool {
    meter.den >= 8 && meter.num > 3 && meter.num.is_multiple_of(3)
}

/// The felt beat, in quarter notes: a dotted quarter (1.5) in 6/8, a quarter in 4/4.
pub fn felt_quarters(meter: TimeSig) -> f64 {
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
/// Beats are tracked from `timing`'s downbeat as one sequence, chosen whole (a dynamic-programming
/// beat tracker, after Ellis 2007), at the beat period the take kept: the period, near
/// `timing`'s, whose beats the onsets fall nearest. `timing`'s period says what the beat is meant
/// to be, given or estimated, not how fast it was played. The onsets the sequence puts beats on
/// are those that maximise their summed relative strength less a cost for each interval that
/// differs from that period. An interval may span several beats, the beats inside it placed
/// evenly, as rests. An interval ending nearer an off-beat than a beat is no interval from beat to
/// beat, and is not taken, so off-beats louder than the beats do not draw the beats onto them.
///
/// A tracker that instead snaps each beat to the onset nearest where it expects it, and adapts
/// to each, is carried onto the off-beats by a take's ordinary timing scatter and runs away; a
/// whole sequence pays for every interval such a slip would bend.
///
/// The onset stage places onsets on a grid of about 6 ms, which is a percent of a beat at 120,
/// so an interval's tempo is not read from its two ends alone: it is the slope, at the
/// interval's middle, of a least-squares quadratic through the `FIT_BEATS` snapped beats nearest
/// it (a Savitzky-Golay derivative), which keeps a slow drift and averages out that grain.
/// Beats in a rest are placed where they were expected but are not fitted.
pub fn beat_series(onsets: &[Event], rate: u32, timing: &Timing) -> Vec<BeatTempo> {
    let rate = f64::from(rate);
    let quarters = felt_quarters(*timing.meter.value());
    // The beat, and the sixteenth, whose grid holds every eighth too: see [`kept_period`].
    let divisions: &[f64] = if is_compound(*timing.meter.value()) {
        &[1.0, 6.0]
    } else {
        &[1.0, 4.0]
    };
    let beats = track(
        &relative(onsets),
        *timing.downbeat_frame.value() as f64,
        timing.beat_period_frames * quarters,
        divisions,
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

/// The beats from `start`, put on `onsets` (sorted by frame) as the best-scoring sequence at the
/// beat period the take kept, its beat divided into each of `divisions`: see [`beat_series`].
fn track(onsets: &[Onset], start: f64, hint: f64, divisions: &[f64]) -> Vec<Beat> {
    sequence(onsets, start, kept_period(onsets, start, hint, divisions)).1
}

/// The beat period the take kept after `start`: among periods within [`KEPT_RANGE`] of `hint`,
/// the one whose grid, counted from `start`, puts the most onset strength near it. The grid is
/// the beat and its sixteenths (`divisions`), and an onset counts at whichever level it sits
/// nearest, out to [`SNAP_TOLERANCE`] of that level's step, counting less the further it is: an
/// onset meant on the beat may stray further than one meant on a sixteenth. The hint is the
/// given or estimated tempo, which says what the beat is meant to be, not how fast it was played;
/// a sequence held to a period a tenth off is not the take's.
///
/// The sixteenths count because a line can put too few onsets on its beats for them alone to say
/// its tempo: a bass line syncopated in sixteenths, or a sung line mostly off the beat, fits some
/// other period as well by chance. The sixteenth grid holds every eighth, so a separate eighth
/// level would only widen the reach around eighths. The share of the take the grid reaches is
/// the same at every period, so a quicker one gains nothing by having more steps.
fn kept_period(onsets: &[Onset], start: f64, hint: f64, divisions: &[f64]) -> f64 {
    let counted = |period: f64| {
        onsets
            .iter()
            .filter(move |o| o.frame >= start - SNAP_TOLERANCE * period)
    };
    let score = |period: f64| -> f64 {
        counted(period)
            .map(|o| o.strength * nearness((o.frame - start) / period, divisions))
            .sum()
    };
    // Neighbouring periods tried differ, by the take's last onset, by half the finest level's
    // reach: the fit's peak is narrower the longer the take, and a coarser search steps over it.
    let last = onsets.last().map_or(start, |o| o.frame);
    let span_beats = ((last - start) / hint).max(1.0);
    let finest = divisions.iter().copied().fold(1.0, f64::max);
    let ratio = 1.0 + SNAP_TOLERANCE / finest / 2.0 / span_beats;
    let steps = (KEPT_RANGE.ln() / ratio.ln()).ceil() as i32;
    // Ties go to the period nearest the hint: the candidates run outward from it.
    let (best_score, best) = (0..=steps)
        .flat_map(|k| [k, -k])
        .map(|k| hint * ratio.powi(k))
        .fold((f64::MIN, hint), |kept, period| {
            let s = score(period);
            if s > kept.0 { (s, period) } else { kept }
        });
    // About how far apart two periods' scores fall by chance: each onset adds its strength times
    // a nearness that, at a phase unrelated to the grid, varies by `nearness_spread`, so the
    // difference of two independent scores varies by √2 times that, scaled by the root of the
    // summed squared strengths. Nearby periods' scores are not independent; see [`CLEAR_LEAD`]. Sparse onsets give a wide chance spread, and only a period clearly
    // ahead of the hint replaces it.
    let strengths: f64 = counted(hint).map(|o| o.strength * o.strength).sum();
    let chance = std::f64::consts::SQRT_2 * nearness_spread(divisions) * strengths.sqrt();
    if best_score - score(hint) > CLEAR_LEAD * chance {
        best
    } else {
        hint
    }
}

/// How near `beats` (a position in beats from the downbeat) is to the grid of the beat divided
/// into each of `divisions`: 1 on a step, falling to 0 at [`SNAP_TOLERANCE`] of a step away, at
/// whichever level it is nearest.
fn nearness(beats: f64, divisions: &[f64]) -> f64 {
    divisions
        .iter()
        .map(|division| {
            let steps = beats * division;
            let off = (steps - steps.round()).abs() / SNAP_TOLERANCE;
            if off < 1.0 { 1.0 - off * off } else { 0.0 }
        })
        .fold(0.0, f64::max)
}

/// The standard deviation of [`nearness`] over a beat, for a position unrelated to the grid.
fn nearness_spread(divisions: &[f64]) -> f64 {
    const SAMPLES: u32 = 4096;
    let values: Vec<f64> = (0..SAMPLES)
        .map(|i| nearness(f64::from(i) / f64::from(SAMPLES), divisions))
        .collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64).sqrt()
}

/// The best-scoring sequence of beats from `start` at `period`, and its score.
fn sequence(onsets: &[Onset], start: f64, period: f64) -> (f64, Vec<Beat>) {
    // The sequence's nodes: the downbeat, then every onset after it. The downbeat is an onset's
    // own node when one is on it.
    if onsets.is_empty() {
        return (f64::MIN, Vec::new());
    }
    let on_start = onsets.iter().find(|o| (o.frame - start).abs() < 1.0);
    let mut nodes = vec![Onset {
        frame: start,
        strength: on_start.map_or(0.0, |o| o.strength),
    }];
    nodes.extend(onsets.iter().filter(|o| o.frame >= start + 1.0).copied());
    // best[j]: the score of the best sequence from the downbeat ending on node j, the node
    // before it, and the beats its last interval spans. Only node 0 starts a sequence.
    let mut best: Vec<Option<(f64, usize, u32)>> = vec![None; nodes.len()];
    best[0] = Some((nodes[0].strength, 0, 0));
    for j in 1..nodes.len() {
        for i in (0..j).rev() {
            let gap = nodes[j].frame - nodes[i].frame;
            if gap > (MAX_SPAN_BEATS + 0.5) * period {
                break;
            }
            let Some((before, _, _)) = best[i] else {
                continue;
            };
            let beats = (gap / period).round().max(1.0);
            if beats > MAX_SPAN_BEATS {
                continue;
            }
            let bend = gap / period - beats;
            // Across a rest, with no onset inside the span, a tempo that moved meanwhile shifts
            // where the beat comes back, so the bound is per beat spanned; a span over onsets
            // that were there to be beats is held to the bound itself.
            let rest = j == i + 1;
            let bound = if rest {
                MAX_BEND_BEATS * beats
            } else {
                MAX_BEND_BEATS
            };
            if bend.abs() >= bound {
                continue;
            }
            let score = before + nodes[j].strength - TIGHTNESS * bend * bend;
            if best[j].is_none_or(|(s, _, _)| score > s) {
                best[j] = Some((score, i, beats as u32));
            }
        }
        // After a rest longer than any interval spans, the beat carries on from the last node
        // before it, the rest's beats placed evenly; its phase is less sure, but the beat goes on.
        // A node nearer than that which no beat reaches is off the beat, and is not a beat.
        if best[j].is_none()
            && let Some(i) = (0..j).rev().find(|&i| best[i].is_some())
            && nodes[j].frame - nodes[i].frame > (MAX_SPAN_BEATS + 0.5) * period
        {
            let gap = nodes[j].frame - nodes[i].frame;
            let beats = (gap / period).round().max(1.0);
            let bend = gap / period - beats;
            let before = best[i].map_or(0.0, |(s, _, _)| s);
            let score = before + nodes[j].strength - TIGHTNESS * bend * bend;
            best[j] = Some((score, i, beats as u32));
        }
    }
    // The sequence ends on the best-scoring node within a beat of the furthest any reaches, so
    // it neither stops short of the take's end nor runs past it.
    let furthest = (0..nodes.len())
        .rev()
        .find(|&j| best[j].is_some())
        .unwrap_or(0);
    let mut end = furthest;
    for j in (0..=furthest).rev() {
        if nodes[furthest].frame - nodes[j].frame > period {
            break;
        }
        if let (Some((s, _, _)), Some((e, _, _))) = (best[j], best[end])
            && s > e
        {
            end = j;
        }
    }
    let mut chain = vec![end];
    while let Some(&j) = chain.last() {
        if j == 0 {
            break;
        }
        chain.push(best[j].map_or(0, |(_, i, _)| i));
    }
    chain.reverse();
    let mut beats = vec![Beat {
        frame: start,
        snapped: on_start.is_some(),
    }];
    for pair in chain.windows(2) {
        let (from, to) = (nodes[pair[0]].frame, nodes[pair[1]].frame);
        let spans = best[pair[1]].map_or(1, |(_, _, b)| b);
        for k in 1..spans {
            beats.push(Beat {
                frame: from + (to - from) * f64::from(k) / f64::from(spans),
                snapped: false,
            });
        }
        beats.push(Beat {
            frame: to,
            snapped: true,
        });
    }
    (best[end].map_or(f64::MIN, |(s, _, _)| s), beats)
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
        assert_eq!(timing.tempo_alternatives, None);
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
        assert_eq!(timing.tempo_alternatives, Some((60.0, 240.0)));
    }

    #[test]
    fn an_estimate_is_a_whole_bpm() {
        let onsets = onsets_of(&pattern(92.4, LEAD, 20.0, &[0.5]));
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert_eq!(estimated(&timing.tempo_bpm).0, 92.0);
        assert_eq!(timing.tempo_alternatives, Some((46.0, 184.0)));
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

    /// A deterministic stand-in for a random source: a 64-bit LCG, mapped to -1..1.
    struct Jitter(u64);

    impl Jitter {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        }
    }

    /// A steady, dense, played take at `bpm`: every quarter, most eighths, some sixteenths and
    /// ghost onsets anywhere, each up to 50 ms off where it was meant: about the scatter a
    /// beatboxed take's onsets show around a steady grid. Onsets are given directly, so the test is of the tracker, not the onset stage.
    fn dense_played_take(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let rate = f64::from(RATE);
        let mut times: Vec<(f64, f32)> = Vec::new();
        let sixteenth = 15.0 / bpm;
        let mut step = 0u32;
        loop {
            let meant = LEAD + f64::from(step) * sixteenth;
            if meant > secs {
                break;
            }
            let chance = rng.next().abs();
            let played = match step % 4 {
                0 => Some(1.0),
                2 if chance < 0.8 => Some(0.6),
                1 | 3 if chance < 0.3 => Some(0.4),
                _ => None,
            };
            if let Some(strength) = played {
                times.push((meant + 0.050 * rng.next(), strength));
            }
            if rng.next().abs() < 0.1 {
                times.push((meant + sixteenth * rng.next().abs(), 0.3));
            }
            step += 1;
        }
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        times.dedup_by(|b, a| b.0 - a.0 < 0.05);
        times
            .into_iter()
            .map(|(t, strength)| Event::Onset {
                frame: (t * rate).round() as u64,
                strength,
            })
            .collect()
    }

    /// The beat series of a steady dense take played at 108, tracked from a tempo of `from_bpm`,
    /// with the downbeat on its first beat. The series is the tempo the take kept, so it should
    /// settle on 108 whatever tempo it starts from.
    fn series_of_a_take_at_108(seed: u64, from_bpm: f64) -> Vec<BeatTempo> {
        series_of_a_take_at_108_lasting(seed, from_bpm, 30.0)
    }

    /// [`series_of_a_take_at_108`] for a take `secs` long.
    fn series_of_a_take_at_108_lasting(seed: u64, from_bpm: f64, secs: f64) -> Vec<BeatTempo> {
        let onsets = dense_played_take(108.0, seed, secs);
        let given = Given {
            tempo_bpm: Some(from_bpm),
            meter: Some(meter("4/4")),
            downbeat_secs: Some(LEAD),
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        beat_series(&onsets, RATE, &timing)
    }

    #[test]
    fn a_steady_dense_take_keeps_its_tempo() {
        // Started at the tempo played, and at 120 and 123, about what a take meant at 120 but
        // played at 108 is given, or estimated, as.
        for from_bpm in [108.0, 120.0, 123.0] {
            for seed in 1..=8 {
                let series = series_of_a_take_at_108(seed, from_bpm);
                let context = format!("from {from_bpm}, seed {seed}");
                assert!(series.len() >= 50, "{context}: {} beats", series.len());
                // An 8-beat local fit through onsets this scattered reads a few bpm either way;
                // a slip or a climb reads tens.
                let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
                bpms.sort_by(f64::total_cmp);
                let median = bpms[bpms.len() / 2];
                assert!((median - 108.0).abs() <= 1.5, "{context}: median {median}");
                for (i, beat) in series.iter().enumerate() {
                    // In the first and last bar the local fit is one-sided, and reads wider.
                    let inner = i >= 4 && i + 4 < series.len();
                    assert!(
                        !inner || (beat.bpm - 108.0).abs() <= 8.0,
                        "{context}: {beat:?}"
                    );
                    let beats = (beat.secs - LEAD) * 108.0 / 60.0;
                    let off = (beats - beats.round()) * 60.0 / 108.0;
                    // Well short of the 0.28 s of a slip onto the off-beat.
                    assert!(
                        off.abs() <= 0.1,
                        "{context}: {beat:?} is {off} s off a beat"
                    );
                }
            }
        }
    }

    /// Even taps on every eighth at `bpm`, of much the same strength, each up to 30 ms off: a
    /// hand keeping time on a book. Every period near the beat finds a tap near each of its beats,
    /// so a tracker that scores periods by what their beats collect favours the quickest.
    fn even_taps(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let eighth = 30.0 / bpm;
        (0..)
            .map(|k| LEAD + f64::from(k) * eighth)
            .take_while(|&t| t <= secs)
            .map(|t| Event::Onset {
                frame: ((t + 0.030 * rng.next()) * f64::from(RATE))
                    .round()
                    .max(0.0) as u64,
                strength: (0.9 + 0.1 * rng.next()) as f32,
            })
            .collect()
    }

    #[test]
    fn even_taps_keep_the_tempo_they_were_tapped_at() {
        for from_bpm in [113.0, 120.0, 100.0] {
            for seed in 1..=8 {
                let onsets = even_taps(113.0, seed, 20.0);
                let given = Given {
                    tempo_bpm: Some(from_bpm),
                    meter: Some(meter("4/4")),
                    downbeat_secs: Some(LEAD),
                };
                let timing = estimate(&onsets, RATE, &given).unwrap();
                let series = beat_series(&onsets, RATE, &timing);
                let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
                bpms.sort_by(f64::total_cmp);
                let median = bpms[bpms.len() / 2];
                assert!(
                    (median - 113.0).abs() <= 1.5,
                    "from {from_bpm}, seed {seed}: median {median}"
                );
            }
        }
    }

    /// A tresillo tapped at `bpm` in 4/4: strong taps on eighths 0, 3 and 6 of each bar, each up
    /// to 20 ms off, with faint ghosts (a book's own rattle) scattered between.
    fn tresillo_taps(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let eighth = 30.0 / bpm;
        let mut taps = Vec::new();
        for k in 0.. {
            let meant = LEAD + f64::from(k) * eighth;
            if meant > secs {
                break;
            }
            if [0, 3, 6].contains(&(k % 8)) {
                taps.push((meant + 0.020 * rng.next(), 1.0));
            } else if rng.next().abs() < 0.6 {
                taps.push((meant + eighth * 0.5 * rng.next(), 0.05));
            }
        }
        taps.sort_by(|a: &(f64, f32), b| a.0.total_cmp(&b.0));
        taps.into_iter()
            .map(|(t, strength)| Event::Onset {
                frame: (t * f64::from(RATE)).round().max(0.0) as u64,
                strength,
            })
            .collect()
    }

    #[test]
    fn a_tresillo_keeps_its_quarter_note_tempo() {
        // Its taps fall every beat and a half, then every beat; a beat of an eighth and a half,
        // four-thirds the tempo, lands on every tap, but it is another beat level, not the tempo
        // given or estimated.
        for from_bpm in [115.0, 120.0, 104.0] {
            for seed in 1..=8 {
                let onsets = tresillo_taps(115.0, seed, 20.0);
                let given = Given {
                    tempo_bpm: Some(from_bpm),
                    meter: Some(meter("4/4")),
                    downbeat_secs: Some(LEAD),
                };
                let timing = estimate(&onsets, RATE, &given).unwrap();
                let series = beat_series(&onsets, RATE, &timing);
                let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
                bpms.sort_by(f64::total_cmp);
                let median = bpms[bpms.len() / 2];
                assert!(
                    (median - 115.0).abs() <= 2.0,
                    "from {from_bpm}, seed {seed}: median {median}"
                );
            }
        }
    }

    /// A sparse sung line at `bpm`: about one onset in four eighths, two in three of them off
    /// the beat, each up to 40 ms off and of much the same strength, one louder. Too few onsets
    /// land on beats for the beats alone to say the tempo; the eighths they land on do.
    fn sparse_syncopated_line(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let eighth = 30.0 / bpm;
        let mut onsets = Vec::new();
        for k in 0.. {
            let meant = LEAD + f64::from(k) * eighth;
            if meant > secs {
                break;
            }
            let chance = rng.next().abs();
            let sung = if k == 0 {
                true
            } else if k % 2 == 0 {
                chance < 0.15
            } else {
                chance < 0.3
            };
            if sung {
                let strength = if k == 16 { 1.0 } else { 0.3 + 0.1 * rng.next() };
                onsets.push(Event::Onset {
                    frame: ((meant + 0.040 * rng.next()) * f64::from(RATE))
                        .round()
                        .max(0.0) as u64,
                    strength: strength as f32,
                });
            }
        }
        onsets
    }

    #[test]
    fn a_sparse_syncopated_line_keeps_its_given_tempo() {
        for seed in 1..=8 {
            let onsets = sparse_syncopated_line(108.0, seed, 30.0);
            let given = Given {
                tempo_bpm: Some(108.0),
                meter: Some(meter("4/4")),
                downbeat_secs: Some(LEAD),
            };
            let timing = estimate(&onsets, RATE, &given).unwrap();
            let series = beat_series(&onsets, RATE, &timing);
            let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
            bpms.sort_by(f64::total_cmp);
            let median = bpms[bpms.len() / 2];
            assert!(
                (median - 108.0).abs() <= 2.0,
                "seed {seed}: median {median} from {} onsets",
                onsets.len()
            );
        }
    }

    /// A sequenced bass line at `bpm`, syncopated in sixteenths: in every bar, sixteenths 0, 2,
    /// 5 and 11, each up to 3 ms off. Half its onsets are on neither a beat nor an eighth, so a
    /// tempo read from beats and eighths alone can fit another period better than its own.
    fn sixteenth_syncopation(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let sixteenth = 15.0 / bpm;
        let pattern = [(0, 0.8), (2, 0.55), (5, 0.3), (11, 0.45)];
        let mut onsets = Vec::new();
        for bar in 0.. {
            let bar_start = LEAD + f64::from(bar) * 16.0 * sixteenth;
            if bar_start > secs {
                break;
            }
            for (step, strength) in pattern {
                let t = bar_start + f64::from(step) * sixteenth + 0.003 * rng.next();
                onsets.push(Event::Onset {
                    frame: (t * f64::from(RATE)).round().max(0.0) as u64,
                    strength,
                });
            }
        }
        onsets
    }

    #[test]
    fn a_line_syncopated_in_sixteenths_keeps_its_given_tempo() {
        for seed in 1..=4 {
            let onsets = sixteenth_syncopation(121.0, seed, 30.0);
            let given = Given {
                tempo_bpm: Some(121.0),
                meter: Some(meter("4/4")),
                downbeat_secs: Some(LEAD),
            };
            let timing = estimate(&onsets, RATE, &given).unwrap();
            let series = beat_series(&onsets, RATE, &timing);
            let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
            bpms.sort_by(f64::total_cmp);
            let median = bpms[bpms.len() / 2];
            assert!(
                (median - 121.0).abs() <= 1.0,
                "seed {seed}: median {median}"
            );
        }
    }

    /// A sparse sung line at `bpm`, timed as a voice is: about one onset in four eighths, most
    /// off the beat, each about 40 ms early with 50 ms of scatter, and one in five a further
    /// 100 ms either way (a scoop, a late breath).
    fn sung_line(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let eighth = 30.0 / bpm;
        let mut onsets = Vec::new();
        for k in 0.. {
            let meant = LEAD + f64::from(k) * eighth;
            if meant > secs {
                break;
            }
            let chance = rng.next().abs();
            let sung = k == 0
                || if k % 2 == 0 {
                    chance < 0.15
                } else {
                    chance < 0.3
                };
            if sung {
                let mut off = -0.040 + 0.050 * rng.next();
                if rng.next().abs() < 0.2 {
                    off += 0.100 * rng.next();
                }
                onsets.push(Event::Onset {
                    frame: ((meant + off).max(0.0) * f64::from(RATE)).round() as u64,
                    strength: (0.35 + 0.15 * rng.next()) as f32,
                });
            }
        }
        onsets
    }

    #[test]
    fn a_sung_line_keeps_a_given_tempo_its_onsets_do_not_contradict() {
        // A line this sparse and this loosely timed fits some period within the search a little
        // better than its own, by chance; the given tempo must not give way to it. (The local
        // tempo each beat reports through sparse beats has its own, separate, scatter.)
        let simple = [1.0, 4.0];
        for seed in 1..=16 {
            let onsets = sung_line(108.0, seed, 30.0);
            let given = Given {
                tempo_bpm: Some(108.0),
                meter: Some(meter("4/4")),
                downbeat_secs: None,
            };
            let timing = estimate(&onsets, RATE, &given).unwrap();
            let start = *timing.downbeat_frame.value() as f64;
            let kept = kept_period(
                &relative(&onsets),
                start,
                timing.beat_period_frames,
                &simple,
            );
            let bpm = 60.0 * f64::from(RATE) / kept;
            assert!(
                (bpm - 108.0).abs() <= 0.5,
                "seed {seed}: kept {bpm} from {} onsets",
                onsets.len()
            );
        }
    }

    /// Eighths at `bpm`, the off-beats louder than the beats (a hi-hat on the "and", a
    /// backbeat pattern), each up to 20 ms off.
    fn loud_off_beats(bpm: f64, seed: u64, secs: f64) -> Vec<Event> {
        let mut rng = Jitter(seed);
        let eighth = 30.0 / bpm;
        (0..)
            .map(|k| (k, LEAD + f64::from(k) * eighth))
            .take_while(|&(_, t)| t <= secs)
            .map(|(k, t)| Event::Onset {
                frame: ((t + 0.020 * rng.next()).max(0.0) * f64::from(RATE)).round() as u64,
                strength: if k % 2 == 0 { 0.4 } else { 1.0 },
            })
            .collect()
    }

    #[test]
    fn loud_off_beats_leave_the_beats_on_the_beat() {
        for seed in 1..=4 {
            let onsets = loud_off_beats(120.0, seed, 30.0);
            let given = Given {
                tempo_bpm: Some(120.0),
                meter: Some(meter("4/4")),
                downbeat_secs: Some(LEAD),
            };
            let timing = estimate(&onsets, RATE, &given).unwrap();
            let series = beat_series(&onsets, RATE, &timing);
            assert!(series.len() >= 55, "seed {seed}: {} beats", series.len());
            for beat in &series {
                let beats = (beat.secs - LEAD) / 0.5;
                let off = (beats - beats.round()) * 0.5;
                assert!(
                    off.abs() <= 0.1,
                    "seed {seed}: {beat:?} is {off} s off a beat"
                );
            }
        }
    }

    #[test]
    fn a_long_steady_take_keeps_its_tempo() {
        // Five minutes: the period that fits is narrower the longer the take, and a search that
        // steps over it keeps the tempo it was started from.
        for seed in 1..=2 {
            let series = series_of_a_take_at_108_lasting(seed, 120.0, 300.0);
            assert!(series.len() >= 530, "seed {seed}: {} beats", series.len());
            for beat in &series[4..series.len() - 4] {
                assert!((beat.bpm - 108.0).abs() <= 8.0, "seed {seed}: {beat:?}");
                let beats = (beat.secs - LEAD) * 108.0 / 60.0;
                let off = (beats - beats.round()) * 60.0 / 108.0;
                assert!(
                    off.abs() <= 0.1,
                    "seed {seed}: {beat:?} is {off} s off a beat"
                );
            }
        }
    }

    #[test]
    fn a_line_syncopated_in_sixteenths_corrects_a_wrong_hint() {
        // Played at 121 and given 112: half its onsets are on neither a beat nor an eighth, so
        // only a fit that counts the sixteenths finds the tempo it was played at.
        for seed in 1..=4 {
            let onsets = sixteenth_syncopation(121.0, seed, 30.0);
            let given = Given {
                tempo_bpm: Some(112.0),
                meter: Some(meter("4/4")),
                downbeat_secs: Some(LEAD),
            };
            let timing = estimate(&onsets, RATE, &given).unwrap();
            let series = beat_series(&onsets, RATE, &timing);
            let mut bpms: Vec<f64> = series.iter().map(|b| b.bpm).collect();
            bpms.sort_by(f64::total_cmp);
            let median = bpms[bpms.len() / 2];
            assert!(
                (median - 121.0).abs() <= 1.0,
                "seed {seed}: median {median}"
            );
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

    /// A phrase at 120 a minute from [`LEAD`], with the run of the take that showed the fault:
    /// C D E F as quarters, a run of four 80 ms notes (G F E D), then C E G C, C D, and E held,
    /// each note attacked as the last ends, so the run moves every later note off the first
    /// notes' beat by 0.32 s.
    fn a_run_inside_a_phrase() -> Vec<f32> {
        let midis_and_lengths = [
            (60, 0.5),
            (62, 0.5),
            (64, 0.5),
            (65, 0.5),
            (67, 0.08),
            (65, 0.08),
            (64, 0.08),
            (62, 0.08),
            (60, 0.5),
            (64, 0.5),
            (67, 0.5),
            (72, 0.5),
            (60, 0.5),
            (62, 0.5),
            (64, 1.0),
        ];
        let notes: Vec<(f64, f64)> = midis_and_lengths
            .iter()
            .map(|&(midi, secs)| (440.0 * ((f64::from(midi) - 69.0) / 12.0).exp2(), secs))
            .collect();
        let mut signal = synth::silence(RATE, LEAD);
        signal.extend(synth::note_sequence(RATE, &notes, 4, 0.5));
        signal.extend(synth::silence(RATE, 1.0));
        signal
    }

    /// Whether the timing's downbeat lies between the first and last of `onsets`.
    fn downbeat_within(timing: &Timing, onsets: &[Event]) -> bool {
        let frames = frames(onsets);
        let downbeat = *timing.downbeat_frame.value();
        frames.first().is_some_and(|&f| downbeat >= f)
            && frames.last().is_some_and(|&l| downbeat <= l)
    }

    #[test]
    fn a_given_tempo_puts_the_downbeat_among_a_runs_onsets() {
        let onsets = onsets_of(&a_run_inside_a_phrase());
        let given = Given {
            tempo_bpm: Some(120.0),
            ..Given::default()
        };
        let timing = estimate(&onsets, RATE, &given).unwrap();
        assert!(
            downbeat_within(&timing, &onsets),
            "{timing:?} {:?}",
            frames(&onsets)
        );
    }

    #[test]
    fn an_estimated_tempo_puts_the_downbeat_among_a_runs_onsets() {
        let onsets = onsets_of(&a_run_inside_a_phrase());
        let timing = estimate(&onsets, RATE, &Given::default()).unwrap();
        assert!(
            downbeat_within(&timing, &onsets),
            "{timing:?} {:?}",
            frames(&onsets)
        );
    }

    /// The take that showed the fault: C D E F G A B C twice, as quarters of 0.6 s (100 a
    /// minute) from 0.3 s in, each with a 200-frame rise and a 400-frame fall, recorded at
    /// `rate`: as a melody's events, through `Prepare` and `Melody`.
    fn scale_at_100(rate: u32) -> Vec<Event> {
        use crate::notes::Melody;
        use crate::prepare::Prepare;
        use crate::stage::Chunk;
        let mut samples = vec![0.0f32; (0.3 * f64::from(rate)) as usize];
        let mut phase = 0.0f64;
        for midi in [60, 62, 64, 65, 67, 69, 71, 72].repeat(2) {
            let hz = 440.0 * ((f64::from(midi) - 69.0) / 12.0).exp2();
            let n = (0.6 * f64::from(rate)) as usize;
            for i in 0..n {
                let envelope = (i as f64 / 200.0).min(1.0) * ((n - i) as f64 / 400.0).min(1.0);
                phase += std::f64::consts::TAU * hz / f64::from(rate);
                let tone = phase.sin() + 0.5 * (2.0 * phase).sin() + 0.25 * (3.0 * phase).sin();
                samples.push((0.3 * envelope * tone) as f32);
            }
        }
        samples.extend(vec![0.0; (0.3 * f64::from(rate)) as usize]);
        let (mut prepare, mut melody) = (Prepare::new(RATE), Melody::new(RATE));
        let (mut mono, mut events) = (Vec::new(), Vec::new());
        prepare
            .push(
                Chunk {
                    rate,
                    channels: 1,
                    start_frame: 0,
                    samples,
                },
                &mut mono,
            )
            .unwrap();
        prepare.finish(&mut mono).unwrap();
        for m in mono {
            melody.push(m, &mut events).unwrap();
        }
        melody.finish(&mut events).unwrap();
        events
    }

    /// A take whose beat level is a close call is not stated as a sure tempo: at one rate it
    /// reads 100, at another 50, and wherever the reading is not 100 it is weak.
    #[test]
    fn a_close_call_between_a_tempo_and_its_half_is_weak() {
        for rate in [22_050, 48_000] {
            let timing = estimate(&scale_at_100(rate), RATE, &Given::default()).unwrap();
            let (bpm, confidence) = estimated(&timing.tempo_bpm);
            assert!(
                bpm == 100.0 || confidence < WEAK_CONFIDENCE,
                "{rate} Hz: {bpm} bpm at confidence {confidence}"
            );
        }
    }

    #[test]
    fn timing_round_trips_through_json() {
        let timing = Timing {
            tempo_bpm: Origin::Estimated {
                value: 135.0,
                confidence: 0.75,
            },
            tempo_alternatives: Some((67.5, 270.0)),
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
        assert!(
            json.contains(r#""tempo_alternatives":[67.5,270.0]"#),
            "{json}"
        );
        // A given tempo has no alternatives to give, and the analysis leaves them out.
        let given = Timing {
            tempo_bpm: Origin::Given(135.0),
            tempo_alternatives: None,
            ..timing
        };
        let json = serde_json::to_string(&given).unwrap();
        assert!(!json.contains("tempo_alternatives"), "{json}");
        assert_eq!(serde_json::from_str::<Timing>(&json).unwrap(), given);
        let beat = BeatTempo {
            beat: 3,
            secs: 1.5,
            bpm: 120.5,
        };
        let back: BeatTempo = serde_json::from_str(&serde_json::to_string(&beat).unwrap()).unwrap();
        assert_eq!(back, beat);
    }
}
