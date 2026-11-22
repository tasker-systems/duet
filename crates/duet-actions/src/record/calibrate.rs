//! `duet record --calibrate` (ruling 18): the offset between a device pair, measured, since what
//! an ordinary microphone reports about its latency cannot be relied on.
//!
//! Nothing here opens a device. The calibration click is [`ticks`]; [`heard`] finds them in a
//! recording of channel 0 against where the streams' timestamps put them; and the measured
//! offsets are kept in `<user config dir>/calibration.toml` ([`Calibrations`]), one per pair of
//! input and output, by their names and rates. The numbers are the design's: 60 bpm in 4/4,
//! 12 ticks after a 1 s lead, each searched for 400 ms either side, the loudest sample there the
//! tick when it exceeds 8 times the floor (the median magnitude of the first 0.5 s), and stored
//! when 10 or more are heard within 2 ms of each other. The ticks are not timed one at a time by
//! a threshold: each heard tick's segment is aligned to the ticks' mean response by
//! cross-correlation, the mean is timed once (its first sample over 8 times its own floor), and
//! each tick is heard where that onset falls through its alignment (design, amended in fix
//! round 4). An input that passes a tick on in two bursts, as the owner's webcam does, moves no
//! tick from one burst to the other that way.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow};
use duet_core::TimeSig;
use duet_schemas::journal::At;
use duet_schemas::song::Composition;
use serde::{Deserialize, Serialize};

use super::click::Ticks;

/// The calibration click's tempo, in quarter notes a minute: a tick a second.
const BPM: f64 = 60.0;
/// How many ticks it plays: a bar of count-in and two bars.
pub(crate) const TICKS: usize = 12;
/// The silence before its first tick, in seconds.
pub(crate) const LEAD_SECS: f64 = 1.0;
/// How far either side of where the timestamps put a tick it is searched for, in seconds.
pub(crate) const SEARCH_SECS: f64 = 0.4;
/// How far before a heard tick's loudest sample its segment starts, in seconds: room for the
/// tick's rise and an input's ramp ahead of it, and how far the first alignment looks either way,
/// so a tick whose loudest sample is a reflection up to this late is still aligned.
const SEGMENT_BEFORE_SECS: f64 = 0.04;
/// How far after a heard tick's loudest sample its segment ends, in seconds: the tick dies away.
const SEGMENT_AFTER_SECS: f64 = 0.06;
/// How far either way of where it lies the alignment looks for a segment after its first pass.
const SETTLE_SECS: f64 = 0.002;
/// The most passes of the alignment.
const PASSES: usize = 8;
/// A segment has settled when a pass moves it by no more than this, in frames.
const SETTLED_FRAMES: f64 = 0.05;
/// The start of the aligned mean its floor is measured over, in seconds: the first quarter of the
/// segment's lead, ahead of a rise as long as 25 ms.
const LEAD_IN_SECS: f64 = 0.01;
/// The start of the recording the floor is measured over, in seconds.
const FLOOR_SECS: f64 = 0.5;
/// A tick is heard when its loudest sample exceeds this many times the floor, and the ticks' mean
/// at its first sample exceeding this many times its own.
const OVER_FLOOR: f32 = 8.0;
/// The fewest ticks heard that a calibration is stored from.
const FEWEST: usize = 10;
/// The widest spread of the ticks' errors that a calibration is stored from, in ms.
const WIDEST_MS: f64 = 2.0;

/// The calibration click at `rate`: [`LEAD_SECS`] of silence, then [`TICKS`] ticks at 60 bpm in
/// 4/4, every one in the downbeat's sound.
///
/// # Errors
///
/// None in practice: the composition is built here, and holds.
pub(crate) fn ticks(rate: u32) -> Result<Ticks> {
    let meter = TimeSig::new(4, 4).map_err(|e| anyhow!("{e}"))?;
    let composition = Composition::new(BPM, meter, None);
    let lead = (LEAD_SECS * f64::from(rate)).round() as u64;
    // A bar of count-in is four ticks; bar 1 and the seven beats after it make twelve.
    let after_bar_one = (TICKS - 4) as f64 - 0.5;
    let mut ticks = Ticks::of(&composition, rate, lead, after_bar_one * 60.0 / BPM)?;
    // Every tick is the downbeat's sound: a placement anchors on bar 1's downbeat, so that is the
    // sound the calibration measures (design, amended in 14cb53c).
    for tick in &mut ticks.frames {
        tick.1 = true;
    }
    Ok(ticks)
}

/// What a calibration heard: the median of the ticks' errors (where each was heard less where
/// the timestamps put it), how many were heard, and how far apart their errors lie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Heard {
    pub(crate) offset_ms: f64,
    pub(crate) ticks: usize,
    pub(crate) spread_ms: f64,
}

/// Why a calibration stores nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Unheard {
    /// The floor is digital silence: the input heard nothing.
    Silent,
    /// Fewer than ten ticks were heard.
    TooFew { heard: usize, of: usize },
    /// The ticks' errors spread wider than 2 ms.
    Spread { spread_ms: f64 },
}

impl fmt::Display for Unheard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unheard::Silent => write!(
                f,
                "the input heard nothing: its first {FLOOR_SECS} s are digital silence (on macOS, \
                 the terminal app needs microphone access: System Settings, Privacy & Security, \
                 Microphone); nothing is stored"
            ),
            Unheard::TooFew { heard, of } => write!(
                f,
                "heard {heard} of {of} ticks, and a calibration needs {FEWEST}: the click may be \
                 going to headphones, or the speakers are off or too quiet; if the click goes to \
                 headphones, hold one against the microphone; nothing is stored"
            ),
            Unheard::Spread { spread_ms } => write!(
                f,
                "the ticks heard lie {spread_ms:.2} ms apart, wider than {WIDEST_MS} ms: the room \
                 may be noisy, or the input's own processing (noise suppression) may move the \
                 sound; nothing is stored"
            ),
        }
    }
}

impl std::error::Error for Unheard {}

/// The ticks in `samples` (channel 0 at `rate`), against where the timestamps put them
/// (`expected_secs`). Each is located by its loudest sample within [`SEARCH_SECS`], heard when
/// that exceeds 8 times the floor; the heard ticks are aligned to their mean response
/// ([`align`]), and that mean is timed once ([`onset`]). Each tick's error is the mean's onset
/// placed back through that tick's alignment, less its expected time.
///
/// # Errors
///
/// [`Unheard`]: a silent floor, fewer than ten ticks heard, or their errors spread past 2 ms.
pub(crate) fn heard(samples: &[f32], rate: u32, expected_secs: &[f64]) -> Result<Heard, Unheard> {
    let rate_f = f64::from(rate);
    let floor = floor(&samples[..((FLOOR_SECS * rate_f) as usize).min(samples.len())]);
    if floor <= 0.0 {
        return Err(Unheard::Silent);
    }
    let over = OVER_FLOOR * floor;
    // Locate: each tick's loudest sample in its search, with the time it was expected at.
    let located: Vec<(usize, f64)> = expected_secs
        .iter()
        .filter_map(|&t| {
            let from = ((t - SEARCH_SECS).max(0.0) * rate_f).round() as usize;
            let to = (((t + SEARCH_SECS) * rate_f).round() as usize).min(samples.len());
            let (loudest, peak) = samples
                .get(from..to)?
                .iter()
                .map(|s| s.abs())
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            (peak > over).then_some((from + loudest, t))
        })
        .collect();
    let too_few = |heard| Unheard::TooFew {
        heard,
        of: expected_secs.len(),
    };
    if located.len() < FEWEST {
        return Err(too_few(located.len()));
    }
    // Align: each segment starts its lead before its tick's loudest sample.
    let lead = (SEGMENT_BEFORE_SECS * rate_f).round();
    let len = (lead + (SEGMENT_AFTER_SECS * rate_f).round()) as usize;
    let mut starts: Vec<f64> = located.iter().map(|&(at, _)| at as f64 - lead).collect();
    let mean = align(samples, &mut starts, len, rate);
    // Time the mean once, and place it back through each tick's alignment.
    let at = onset(&mean, rate).ok_or_else(|| too_few(0))?;
    let mut errors_ms: Vec<f64> = starts
        .iter()
        .zip(&located)
        .map(|(start, &(_, t))| ((start + at as f64) / rate_f - t) * 1000.0)
        .collect();
    errors_ms.sort_by(f64::total_cmp);
    let spread_ms = errors_ms[errors_ms.len() - 1] - errors_ms[0];
    if spread_ms > WIDEST_MS {
        return Err(Unheard::Spread { spread_ms });
    }
    let mid = errors_ms.len() / 2;
    let offset_ms = if errors_ms.len().is_multiple_of(2) {
        (errors_ms[mid - 1] + errors_ms[mid]) / 2.0
    } else {
        errors_ms[mid]
    };
    Ok(Heard {
        offset_ms,
        ticks: errors_ms.len(),
        spread_ms,
    })
}

/// The median magnitude of `samples`: a floor.
fn floor(samples: &[f32]) -> f32 {
    let mut levels: Vec<f32> = samples.iter().map(|s| s.abs()).collect();
    levels.sort_by(f32::total_cmp);
    levels.get(levels.len() / 2).copied().unwrap_or(0.0)
}

/// Aligns the segments of `samples` that begin at `starts` (in frames, a fraction allowed), each
/// `len` long, to their mean, and gives that mean. Each segment is cross-correlated against the
/// mean and moved to its correlation's peak, to a fraction of a frame by a parabola through the
/// peak and its neighbours; the mean is taken again, and so on, until no segment moves by more
/// than [`SETTLED_FRAMES`] or after [`PASSES`] passes. The first pass looks as far as
/// [`SEGMENT_BEFORE_SECS`] either way, the rest [`SETTLE_SECS`]. The median move of each pass
/// is taken back off every segment, so the mean stays where the loudest samples put it. This
/// times each tick against the others by its whole response, not by any one point on it.
fn align(samples: &[f32], starts: &mut [f64], len: usize, rate: u32) -> Vec<f32> {
    let rate_f = f64::from(rate);
    let wide = (SEGMENT_BEFORE_SECS * rate_f).round() as isize;
    let narrow = (SETTLE_SECS * rate_f).round() as isize;
    for pass in 0..PASSES {
        let mean = mean_of(samples, starts, len);
        let reach = if pass == 0 { wide } else { narrow };
        let mut moved: Vec<f64> = starts
            .iter()
            .map(|&start| best_lag(samples, start.round() as isize, &mean, reach) - start)
            .collect();
        let mut sorted = moved.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        for m in &mut moved {
            *m -= median;
        }
        for (start, m) in starts.iter_mut().zip(&moved) {
            *start += m;
        }
        if moved.iter().all(|m| m.abs() <= SETTLED_FRAMES) {
            break;
        }
    }
    mean_of(samples, starts, len)
}

/// The mean of the segments of `samples` beginning at `starts`, each `len` long, read between
/// frames by a straight line, and as silence outside the recording.
fn mean_of(samples: &[f32], starts: &[f64], len: usize) -> Vec<f32> {
    let at = |i: isize| {
        usize::try_from(i)
            .ok()
            .and_then(|i| samples.get(i))
            .copied()
            .unwrap_or(0.0)
    };
    let mut mean = vec![0.0_f64; len];
    for &start in starts {
        let whole = start.floor();
        let part = start - whole;
        let whole = whole as isize;
        for (n, m) in mean.iter_mut().enumerate() {
            let i = whole + n as isize;
            *m += f64::from(at(i)) * (1.0 - part) + f64::from(at(i + 1)) * part;
        }
    }
    let count = starts.len().max(1) as f64;
    mean.into_iter().map(|m| (m / count) as f32).collect()
}

/// Where in `samples` the segment best matching `mean` begins, within `reach` frames of
/// `around`: the peak of their cross-correlation, to a fraction of a frame.
fn best_lag(samples: &[f32], around: isize, mean: &[f32], reach: isize) -> f64 {
    // The stretch of the recording every lag reads, silence outside it.
    let from = around - reach;
    let width = mean.len() + 2 * reach as usize;
    let stretch: Vec<f32> = (0..width as isize)
        .map(|n| {
            usize::try_from(from + n)
                .ok()
                .and_then(|i| samples.get(i))
                .copied()
                .unwrap_or(0.0)
        })
        .collect();
    let correlation: Vec<f64> = (0..=2 * reach as usize)
        .map(|k| {
            stretch[k..k + mean.len()]
                .iter()
                .zip(mean)
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum()
        })
        .collect();
    let (k, _) = correlation
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap_or((reach as usize, &0.0));
    let part = match (k.checked_sub(1), correlation.get(k + 1)) {
        (Some(before), Some(&after)) => {
            let (before, here) = (correlation[before], correlation[k]);
            let bend = before - 2.0 * here + after;
            if bend < 0.0 {
                0.5 * (before - after) / bend
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    (from + k as isize) as f64 + part
}

/// Where the aligned `mean` of the ticks' segments is heard: the first sample, from the
/// segment's start to the mean's loudest, whose magnitude exceeds 8 times the mean's own floor
/// (the median magnitude of its first [`LEAD_IN_SECS`]). The mean's floor is lower than the
/// recording's, since averaging quietens noise that is not the tick's, so the onset is found
/// lower on the tick's rise than a single tick's could be. None when nothing exceeds it.
fn onset(mean: &[f32], rate: u32) -> Option<usize> {
    let lead_in = ((LEAD_IN_SECS * f64::from(rate)).round() as usize).min(mean.len());
    let over = OVER_FLOOR * floor(&mean[..lead_in]);
    let (loudest, _) = mean
        .iter()
        .map(|s| s.abs())
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    mean[..=loudest].iter().position(|s| s.abs() > over)
}

/// An input and an output, by their names and rates: what a calibration is kept for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pair {
    pub(crate) input: String,
    pub(crate) output: String,
    pub(crate) input_rate: u32,
    pub(crate) output_rate: u32,
}

impl fmt::Display for Pair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} Hz) → {} ({} Hz)",
            self.input, self.input_rate, self.output, self.output_rate
        )
    }
}

/// A pair's measured offset: where its ticks are heard less where the timestamps put them, in ms.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Calibration {
    pub(crate) pair: Pair,
    pub(crate) offset_ms: f64,
    pub(crate) ticks: usize,
    pub(crate) spread_ms: f64,
    pub(crate) measured: At,
}

/// One `[[pair]]` table of `calibration.toml`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    input: String,
    output: String,
    input_rate: u32,
    output_rate: u32,
    offset_ms: f64,
    ticks: usize,
    spread_ms: f64,
    measured: At,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    pair: Vec<Entry>,
}

const HEADER: &str = "\
# Each device pair's measured offset, written by `duet record --calibrate`, and applied by
# `duet record --song` for the same input and output at the same rates. Machine-local.
";

/// Every calibration this machine holds, one per pair.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Calibrations {
    pub(crate) pairs: Vec<Calibration>,
}

impl Calibrations {
    /// The calibrations in `path`: none when it does not exist.
    ///
    /// # Errors
    ///
    /// When it exists and cannot be read as calibrations, naming the path.
    pub(crate) fn read(path: &Path) -> Result<Calibrations> {
        if !path.exists() {
            return Ok(Calibrations::default());
        }
        let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
        let file: File = toml::from_str(&text)
            .with_context(|| format!("{}: not calibrations duet reads", path.display()))?;
        let pairs = file
            .pair
            .into_iter()
            .map(|e| Calibration {
                pair: Pair {
                    input: e.input,
                    output: e.output,
                    input_rate: e.input_rate,
                    output_rate: e.output_rate,
                },
                offset_ms: e.offset_ms,
                ticks: e.ticks,
                spread_ms: e.spread_ms,
                measured: e.measured,
            })
            .collect();
        Ok(Calibrations { pairs })
    }

    /// The calibration of `pair`, if this machine holds one.
    pub(crate) fn find(&self, pair: &Pair) -> Option<&Calibration> {
        self.pairs.iter().find(|c| &c.pair == pair)
    }

    /// Keeps `calibration`, in place of one of the same pair.
    pub(crate) fn set(&mut self, calibration: Calibration) {
        match self.pairs.iter_mut().find(|c| c.pair == calibration.pair) {
            Some(held) => *held = calibration,
            None => self.pairs.push(calibration),
        }
    }

    /// Writes them all to `path`, through a file beside it renamed over it, its directory made
    /// if it is not there.
    ///
    /// # Errors
    ///
    /// When the directory or the file cannot be written, naming it.
    pub(crate) fn write(&self, path: &Path) -> Result<()> {
        let file = File {
            pair: self
                .pairs
                .iter()
                .map(|c| Entry {
                    input: c.pair.input.clone(),
                    output: c.pair.output.clone(),
                    input_rate: c.pair.input_rate,
                    output_rate: c.pair.output_rate,
                    offset_ms: c.offset_ms,
                    ticks: c.ticks,
                    spread_ms: c.spread_ms,
                    measured: c.measured,
                })
                .collect(),
        };
        let text = format!("{HEADER}\n{}", toml::to_string(&file)?);
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        }
        let beside = path.with_extension("toml.writing");
        fs::File::create(&beside)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .with_context(|| beside.display().to_string())?;
        fs::rename(&beside, path).with_context(|| path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::click::ClickVoice;

    const RATE: u32 = 48_000;

    /// The calibration click's expected times, as an ideal clock puts them.
    fn expected() -> Vec<f64> {
        ticks(RATE)
            .unwrap()
            .frames
            .iter()
            .map(|(f, _)| *f as f64 / f64::from(RATE))
            .collect()
    }

    /// A recording: noise at 0.001 RMS throughout, and a tick of `ClickVoice`'s sound at each
    /// expected time plus `errors_ms[i]`, left out where it is `None`.
    fn recording(errors_ms: &[Option<f64>]) -> Vec<f32> {
        recording_with(errors_ms, false)
    }

    /// As [`recording`], and with `pre_sound`, a quieter sound before each tick that sounds (about
    /// 10 times the floor, 100 to 300 ms before it), as the owner's webcam records one.
    fn recording_with(errors_ms: &[Option<f64>], pre_sound: bool) -> Vec<f32> {
        let expected = expected();
        let len = ((expected.last().unwrap() + 1.0) * f64::from(RATE)) as usize;
        let frames: Vec<(u64, bool)> = expected
            .iter()
            .zip(errors_ms)
            .filter_map(|(t, e)| {
                // Every calibration tick is the downbeat's sound.
                e.map(|e| (((t + e / 1000.0) * f64::from(RATE)).round() as u64, true))
            })
            .collect();
        let bar_one = frames.first().map_or(0, |f| f.0);
        let mut out = vec![0.0_f32; len];
        ClickVoice::new(Ticks { frames, bar_one }, RATE, 1).fill(&mut out);
        // A uniform noise of half-width a has an RMS of a / sqrt(3).
        let a = 0.001 * 3.0_f64.sqrt();
        if pre_sound {
            // Only before a tick that sounds: the input's processing makes it from the tick.
            let sounds: Vec<bool> = errors_ms.iter().map(Option::is_some).collect();
            add_pre_sounds(&mut out, &expected, &sounds);
        }
        let mut seed = 0x2545_f491_u64;
        for s in &mut out {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let unit = (seed >> 11) as f64 / (1_u64 << 53) as f64;
            *s += ((unit * 2.0 - 1.0) * a) as f32;
        }
        out
    }

    /// Adds 5 ms of 1 kHz at 10 times the noise's median magnitude (a / 2), 100 to 300 ms before
    /// each expected tick `sounds` marks.
    fn add_pre_sounds(out: &mut [f32], expected: &[f64], sounds: &[bool]) {
        let level = 10.0 * 0.001 * 3.0_f64.sqrt() / 2.0;
        for (i, t) in expected.iter().enumerate() {
            if !sounds[i] {
                continue;
            }
            let before = 0.1 + 0.2 * (i as f64 / 11.0);
            let start = ((t - before) * f64::from(RATE)).round() as usize;
            for n in 0..(RATE as usize / 200) {
                let phase = std::f64::consts::TAU * 1000.0 * n as f64 / f64::from(RATE);
                out[start + n] += (level * phase.sin()) as f32;
            }
        }
    }

    #[test]
    fn the_calibration_click_is_twelve_downbeats_at_60_bpm_after_a_second() {
        let t = expected();
        assert_eq!(t.len(), 12);
        for (k, secs) in t.iter().enumerate() {
            assert!((secs - (1.0 + k as f64)).abs() < 1e-9, "{t:?}");
        }
        // Every one in the downbeat's sound, which a placement anchors on.
        assert!(ticks(RATE).unwrap().frames.iter().all(|&(_, down)| down));
    }

    #[test]
    fn ticks_at_a_known_error_with_noise_and_one_missing_give_that_error() {
        let mut errors = vec![Some(-8.0); 12];
        errors[5] = None;
        let heard = heard(&recording(&errors), RATE, &expected()).expect("heard");
        assert_eq!(heard.ticks, 11);
        // A clean, sharp attack: heard within 0.3 ms of where it starts (fix rounds 2 and 4).
        assert!((heard.offset_ms + 8.0).abs() < 0.3, "{heard:?}");
        assert!(heard.spread_ms < 0.05, "{heard:?}");
    }

    /// Fix round 1 (design amended in e7fa1d5): a quieter sound before each tick is not the tick.
    #[test]
    fn a_quieter_sound_before_each_tick_is_not_heard_as_it() {
        let mut errors = vec![Some(-8.0); 12];
        errors[5] = None;
        let heard = heard(&recording_with(&errors, true), RATE, &expected()).expect("heard");
        assert_eq!(heard.ticks, 11);
        // A clean, sharp attack: heard within 0.3 ms of where it starts (fix rounds 2 and 4).
        assert!((heard.offset_ms + 8.0).abs() < 0.3, "{heard:?}");
    }

    /// A tick as the owner's webcam passes it on: the downbeat's 1760 Hz tone whose level ramps up linearly
    /// over 25 ms to `level`, then dies away over 20 ms.
    fn ramped_tick(level: f64) -> Vec<f32> {
        let rate = f64::from(RATE);
        let (rise, fall) = ((0.025 * rate) as usize, (0.020 * rate) as usize);
        (0..rise + fall)
            .map(|n| {
                let envelope = if n < rise {
                    n as f64 / rise as f64
                } else {
                    1.0 - (n - rise) as f64 / fall as f64
                };
                let phase = std::f64::consts::TAU * 1760.0 * n as f64 / rate;
                (level * envelope * phase.sin()) as f32
            })
            .collect()
    }

    /// Ruling of fix round 2 (design amended in 2aaa244): a tick whose sound ramps up is heard at
    /// the same point on its rise however loud it arrives. Since fix round 4 that point is where
    /// the ramp leaves the ticks' mean's floor, near its start, not where it reaches half its peak.
    #[test]
    fn ramped_ticks_at_levels_four_times_apart_are_heard_at_one_point_on_their_rise() {
        let expected = expected();
        // Noise only, then a pre-sound before every tick, and the ramped ticks 8 ms early.
        let mut out = recording(&[None; 12]);
        add_pre_sounds(&mut out, &expected, &[true; 12]);
        for (i, t) in expected.iter().enumerate() {
            let level = 0.1 + 0.3 * ((i * 5) % 12) as f64 / 11.0; // 0.1 to 0.4
            let start = ((t - 0.008) * f64::from(RATE)).round() as usize;
            for (n, s) in ramped_tick(level).into_iter().enumerate() {
                out[start + n] += s;
            }
        }
        let heard = heard(&out, RATE, &expected).expect("heard");
        assert_eq!(heard.ticks, 12);
        assert!(heard.spread_ms <= 1.0, "{heard:?}");
        // The ramp's start, to where it first rises out of the mean's floor.
        assert!((heard.offset_ms + 8.0).abs() < 0.3, "{heard:?}");
    }

    /// A burst of the downbeat's 1760 Hz tone at `level`, starting at full level and dying away
    /// over `secs`: a sharp attack.
    fn burst(level: f64, secs: f64) -> impl Iterator<Item = f32> {
        let rate = f64::from(RATE);
        let len = (secs * rate) as usize;
        (0..len).map(move |n| {
            let left = 1.0 - n as f64 / len as f64;
            let phase = std::f64::consts::TAU * 1760.0 * n as f64 / rate;
            (level * left * left * phase.sin()) as f32
        })
    }

    /// Ruling of fix round 4: a tick as the owner's webcam passes it on, a weak burst and then a
    /// stronger one 6 ms later, whose first burst lies near half the second's peak, is heard at
    /// its first burst's onset however its bursts' levels vary from tick to tick.
    #[test]
    fn two_bursts_a_tick_are_heard_at_the_first_bursts_onset() {
        let expected = expected();
        let mut out = recording(&[None; 12]);
        // A pre-sound 45 ms before every tick, 5 ms of 1 kHz at 10 times the floor.
        let level = 10.0 * 0.001 * 3.0_f64.sqrt() / 2.0;
        for t in &expected {
            let start = ((t - 0.008 - 0.045) * f64::from(RATE)).round() as usize;
            for n in 0..(RATE as usize / 200) {
                let phase = std::f64::consts::TAU * 1000.0 * n as f64 / f64::from(RATE);
                out[start + n] += (level * phase.sin()) as f32;
            }
        }
        for (i, t) in expected.iter().enumerate() {
            // The second burst at 0.02 to 0.04 (2 times apart); the first at 0.4 to 0.6 of it.
            let main = 0.02 + 0.02 * ((i * 5) % 12) as f64 / 11.0;
            let first = main * (0.4 + 0.2 * ((i * 7) % 12) as f64 / 11.0);
            let start = ((t - 0.008) * f64::from(RATE)).round() as usize;
            for (n, s) in burst(first, 0.004).enumerate() {
                out[start + n] += s;
            }
            let later = start + (0.006 * f64::from(RATE)) as usize;
            for (n, s) in burst(main, 0.03).enumerate() {
                out[later + n] += s;
            }
        }
        let heard = heard(&out, RATE, &expected).expect("heard");
        assert_eq!(heard.ticks, 12);
        assert!((heard.offset_ms + 8.0).abs() < 0.3, "{heard:?}");
        assert!(heard.spread_ms <= 1.0, "{heard:?}");
    }

    #[test]
    fn nine_ticks_heard_store_nothing() {
        let mut errors = vec![Some(-8.0); 12];
        for i in [2, 6, 10] {
            errors[i] = None;
        }
        let unheard = heard(&recording(&errors), RATE, &expected()).unwrap_err();
        assert_eq!(unheard, Unheard::TooFew { heard: 9, of: 12 });
        let said = unheard.to_string();
        assert!(said.contains("hold one against the microphone"), "{said}");
    }

    #[test]
    fn ticks_spread_past_2_ms_store_nothing() {
        let errors: Vec<Option<f64>> = (0..12)
            .map(|i| Some(if i % 2 == 0 { -8.0 } else { -11.0 }))
            .collect();
        let unheard = heard(&recording(&errors), RATE, &expected()).unwrap_err();
        let Unheard::Spread { spread_ms } = unheard else {
            panic!("{unheard:?}");
        };
        assert!((spread_ms - 3.0).abs() < 0.05, "{spread_ms}");
        assert!(!unheard.to_string().contains("headphones"));
    }

    #[test]
    fn digital_silence_is_refused_as_silent() {
        let silence = vec![0.0_f32; 13 * RATE as usize];
        assert_eq!(heard(&silence, RATE, &expected()), Err(Unheard::Silent));
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-calibrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn calibration(input: &str, offset_ms: f64) -> Calibration {
        Calibration {
            pair: Pair {
                input: input.to_owned(),
                output: "External Headphones".to_owned(),
                input_rate: 48_000,
                output_rate: 48_000,
            },
            offset_ms,
            ticks: 12,
            spread_ms: 0.31,
            measured: "2026-11-22T10:04:05.123Z".parse().unwrap(),
        }
    }

    #[test]
    fn the_file_round_trips_replaces_a_pair_and_misses_other_rates() {
        let path = scratch("round-trip/calibration.toml");
        let mut all = Calibrations::read(&path).expect("a missing file is empty");
        assert_eq!(all, Calibrations::default());
        all.set(calibration("Logitech Webcam C930e", -8.123_456_789));
        all.set(calibration("Scarlett 2i2 USB", -1.5));
        all.write(&path).expect("write");
        let back = Calibrations::read(&path).expect("read");
        assert_eq!(back, all);

        let mut again = back.clone();
        again.set(calibration("Logitech Webcam C930e", -7.9));
        assert_eq!(again.pairs.len(), 2, "the same pair is replaced");
        let webcam = calibration("Logitech Webcam C930e", 0.0).pair;
        assert_eq!(again.find(&webcam).map(|c| c.offset_ms), Some(-7.9));
        let at_44 = Pair {
            input_rate: 44_100,
            ..webcam
        };
        assert_eq!(again.find(&at_44), None);
    }

    #[test]
    fn an_unreadable_file_names_its_path() {
        let path = scratch("unreadable.toml");
        std::fs::write(&path, "[[pair]]\ninput = 3\n").unwrap();
        let why = format!("{:#}", Calibrations::read(&path).unwrap_err());
        assert!(why.contains(&path.display().to_string()), "{why}");
    }
}
