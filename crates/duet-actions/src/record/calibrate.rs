//! `duet record --calibrate` (ruling 18): the offset between a device pair, measured, since what
//! an ordinary microphone reports about its latency cannot be relied on.
//!
//! Nothing here opens a device. The calibration click is [`ticks`]; [`heard`] finds them in a
//! recording of channel 0 against where the streams' timestamps put them; and the measured
//! offsets are kept in `<user config dir>/calibration.toml` ([`Calibrations`]), one per pair of
//! input and output, by their names and rates. The numbers are the design's: 60 bpm in 4/4,
//! 12 ticks after a 1 s lead, each searched for 400 ms either side, the loudest sample there the
//! tick, heard at the first sample past 8 times the floor (the median magnitude of the first
//! 0.5 s) in the 20 ms up to it, and stored when 10 or more are heard within 2 ms of each other.

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
/// How far before a search's loudest sample its onset is looked for, in seconds.
const ONSET_SECS: f64 = 0.02;
/// The start of the recording the floor is measured over, in seconds.
const FLOOR_SECS: f64 = 0.5;
/// A tick is heard where the magnitude first exceeds this many times the floor.
const OVER_FLOOR: f32 = 8.0;
/// The fewest ticks heard that a calibration is stored from.
const FEWEST: usize = 10;
/// The widest spread of the ticks' errors that a calibration is stored from, in ms.
const WIDEST_MS: f64 = 2.0;

/// The calibration click at `rate`: [`LEAD_SECS`] of silence, then [`TICKS`] ticks at 60 bpm in
/// 4/4, the first of each four accented.
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
    Ticks::of(&composition, rate, lead, after_bar_one * 60.0 / BPM)
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

/// The ticks in `samples` (channel 0 at `rate`), each searched for within [`SEARCH_SECS`] of its
/// `expected_secs` and heard at its [`onset`]: the first sample over 8 times the floor in the
/// 20 ms up to the search's loudest sample.
///
/// # Errors
///
/// [`Unheard`]: a silent floor, fewer than ten ticks heard, or their errors spread past 2 ms.
pub(crate) fn heard(samples: &[f32], rate: u32, expected_secs: &[f64]) -> Result<Heard, Unheard> {
    let rate_f = f64::from(rate);
    let floor_end = ((FLOOR_SECS * rate_f) as usize).min(samples.len());
    let mut levels: Vec<f32> = samples[..floor_end].iter().map(|s| s.abs()).collect();
    levels.sort_by(f32::total_cmp);
    let floor = levels.get(levels.len() / 2).copied().unwrap_or(0.0);
    if floor <= 0.0 {
        return Err(Unheard::Silent);
    }
    let over = OVER_FLOOR * floor;
    let mut errors_ms: Vec<f64> = expected_secs
        .iter()
        .filter_map(|&t| {
            let from = ((t - SEARCH_SECS).max(0.0) * rate_f).round() as usize;
            let to = (((t + SEARCH_SECS) * rate_f).round() as usize).min(samples.len());
            let at = onset(samples.get(from..to)?, rate, over)?;
            Some(((from + at) as f64 / rate_f - t) * 1000.0)
        })
        .collect();
    if errors_ms.len() < FEWEST {
        return Err(Unheard::TooFew {
            heard: errors_ms.len(),
            of: expected_secs.len(),
        });
    }
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

/// Where in `search` a tick is heard: the loudest sample is the tick, and it is heard at the first
/// sample over `over` in the [`ONSET_SECS`] up to and including that loudest one. None when none
/// there is over it. A first crossing anywhere in the search is not used: on the owner's webcam a
/// quieter sound precedes each tick by 100 to 400 ms, likely the input's own processing (design,
/// amended in e7fa1d5).
pub(crate) fn onset(search: &[f32], rate: u32, over: f32) -> Option<usize> {
    let loudest = search
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?
        .0;
    let back = (ONSET_SECS * f64::from(rate)).round() as usize;
    let from = loudest.saturating_sub(back);
    search[from..=loudest]
        .iter()
        .position(|s| s.abs() > over)
        .map(|i| from + i)
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
            .enumerate()
            .filter_map(|(i, (t, e))| {
                e.map(|e| {
                    (
                        ((t + e / 1000.0) * f64::from(RATE)).round() as u64,
                        i % 4 == 0,
                    )
                })
            })
            .collect();
        let bar_one = frames.first().map_or(0, |f| f.0);
        let mut out = vec![0.0_f32; len];
        ClickVoice::new(Ticks { frames, bar_one }, RATE, 1).fill(&mut out);
        // A uniform noise of half-width a has an RMS of a / sqrt(3).
        let a = 0.001 * 3.0_f64.sqrt();
        if pre_sound {
            // The noise's median magnitude is a / 2: ten times it, for 5 ms of 1 kHz.
            let level = 10.0 * a / 2.0;
            // Only before a tick that sounds: the input's processing makes it from the tick.
            for (i, t) in expected.iter().enumerate() {
                if errors_ms[i].is_none() {
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
        let mut seed = 0x2545_f491_u64;
        for s in &mut out {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let unit = (seed >> 11) as f64 / (1_u64 << 53) as f64;
            *s += ((unit * 2.0 - 1.0) * a) as f32;
        }
        out
    }

    #[test]
    fn the_calibration_click_is_twelve_ticks_at_60_bpm_after_a_second() {
        let t = expected();
        assert_eq!(t.len(), 12);
        for (k, secs) in t.iter().enumerate() {
            assert!((secs - (1.0 + k as f64)).abs() < 1e-9, "{t:?}");
        }
    }

    #[test]
    fn ticks_at_a_known_error_with_noise_and_one_missing_give_that_error() {
        let mut errors = vec![Some(-8.0); 12];
        errors[5] = None;
        let heard = heard(&recording(&errors), RATE, &expected()).expect("heard");
        assert_eq!(heard.ticks, 11);
        assert!((heard.offset_ms + 8.0).abs() < 0.05, "{heard:?}");
        assert!(heard.spread_ms < 0.05, "{heard:?}");
    }

    /// Fix round 1 (design amended in e7fa1d5): a quieter sound before each tick is not the tick.
    #[test]
    fn a_quieter_sound_before_each_tick_is_not_heard_as_it() {
        let mut errors = vec![Some(-8.0); 12];
        errors[5] = None;
        let heard = heard(&recording_with(&errors, true), RATE, &expected()).expect("heard");
        assert_eq!(heard.ticks, 11);
        assert!((heard.offset_ms + 8.0).abs() < 0.05, "{heard:?}");
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
