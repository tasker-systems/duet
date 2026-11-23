//! The click `duet record --song` plays, and the arithmetic that turns the two streams' stamps
//! into where the take sits against bar 1.
//!
//! Nothing here opens a device. The click is a list of output frames (`Ticks`) and a voice that
//! writes them (`ClickVoice`); the clocks are `Stamp`s, the frame index of a buffer and the host
//! instant the system stamped on it, read from the input and from the output stream.

use anyhow::{Result, anyhow, bail};
use duet_core::Position;
use duet_schemas::song::Composition;

/// How long a tick sounds, in seconds.
const TICK_SECS: f64 = 0.02;
/// The loudest sample of a tick.
const PEAK: f32 = 0.5;
/// A tick's pitch on a downbeat, in hertz.
const DOWNBEAT_HZ: f64 = 1760.0;
/// A tick's pitch on any other beat, in hertz.
const BEAT_HZ: f64 = 880.0;

/// The seconds one beat lasts at `bpm` quarter notes a minute in a meter over `den`: a beat is
/// one `den`th note.
fn beat_secs(bpm: f64, den: u8) -> f64 {
    60.0 / bpm * 4.0 / f64::from(den)
}

/// Where the click ticks on the output stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ticks {
    /// Each tick's output frame, and whether it is a downbeat.
    pub(crate) frames: Vec<(u64, bool)>,
    /// The frame of bar 1's tick.
    pub(crate) bar_one: u64,
}

impl Ticks {
    /// The ticks of `lead_frames` of silence, one bar of count-in in the song's first meter at
    /// its first tempo, then every beat from `1.1.0` through the composition's maps until
    /// `until_secs` after bar 1. Bar 1's own tick is always there.
    ///
    /// # Errors
    ///
    /// When the composition's maps are wrong, or `until_secs` is not a number of seconds.
    pub(crate) fn of(
        composition: &Composition,
        rate: u32,
        lead_frames: u64,
        until_secs: f64,
    ) -> Result<Ticks> {
        if !(until_secs.is_finite() && until_secs >= 0.0) {
            bail!("the click cannot run until {until_secs} seconds");
        }
        let rate = f64::from(rate);
        let one = Position::bar(1);
        let first_sig = composition.meter_at(one).map_err(|e| anyhow!(e))?;
        let first_bpm = composition.tempo_at(one).map_err(|e| anyhow!(e))?;
        let count_beat = beat_secs(first_bpm, first_sig.den);
        let count_in = count_beat * f64::from(first_sig.num);
        // A tick `secs` after the start of the count-in.
        let frame = |secs: f64| lead_frames + (secs * rate).round() as u64;

        let mut frames = Vec::new();
        for k in 0..first_sig.num {
            frames.push((frame(f64::from(k) * count_beat), k == 0));
        }
        let bar_one = frame(count_in);
        frames.push((bar_one, true));

        let mut bar = 1_u32;
        'bars: loop {
            let sig = composition
                .meter_at(Position::bar(bar))
                .map_err(|e| anyhow!(e))?;
            for beat in 1..=u32::from(sig.num) {
                if bar == 1 && beat == 1 {
                    continue;
                }
                let at = Position::new(bar, beat, 0);
                let secs = composition.secs_between(one, at).map_err(|e| anyhow!(e))?;
                if secs >= until_secs {
                    break 'bars;
                }
                frames.push((frame(count_in + secs), beat == 1));
            }
            bar += 1;
        }
        Ok(Ticks { frames, bar_one })
    }
}

/// Writes the click into the output stream's buffers, one after another.
#[derive(Debug)]
pub(crate) struct ClickVoice {
    ticks: Ticks,
    rate: u32,
    channels: usize,
    /// The output frame the next `fill` starts at.
    frame: u64,
    /// The first tick that has not ended before `frame`.
    next: usize,
    /// How many frames a tick sounds.
    len: u64,
}

impl ClickVoice {
    pub(crate) fn new(ticks: Ticks, rate: u32, channels: u16) -> Self {
        ClickVoice {
            ticks,
            rate,
            channels: usize::from(channels.max(1)),
            frame: 0,
            next: 0,
            len: (f64::from(rate) * TICK_SECS).round() as u64,
        }
    }

    /// Writes the next `out.len() / channels` frames, interleaved: a tick at its frames and
    /// silence everywhere else. Advances the voice's own frame counter.
    pub(crate) fn fill(&mut self, out: &mut [f32]) {
        for frame_out in out.chunks_mut(self.channels) {
            let f = self.frame;
            while self
                .ticks
                .frames
                .get(self.next)
                .is_some_and(|(start, _)| start + self.len <= f)
            {
                self.next += 1;
            }
            let sample = match self.ticks.frames.get(self.next) {
                Some(&(start, down)) if start <= f => self.sample(f - start, down),
                _ => 0.0,
            };
            frame_out.fill(sample);
            self.frame += 1;
        }
    }

    /// The tick's `n`th sample: a sine that dies away over the tick's length.
    fn sample(&self, n: u64, downbeat: bool) -> f32 {
        let hz = if downbeat { DOWNBEAT_HZ } else { BEAT_HZ };
        let t = n as f64 / f64::from(self.rate);
        let left = 1.0 - n as f64 / self.len as f64;
        (f64::from(PEAK) * left * left * (std::f64::consts::TAU * hz * t).sin()) as f32
    }
}

/// A stream's frame index and the host instant, in nanoseconds, the system stamped on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) frame: u64,
    pub(crate) nanos: u128,
}

/// How long a stream's stamps take to settle: a USB input's host stamps on macOS step by about
/// 0.9 ms in the first second, then run straight. Measured on the owner's Mac mini, 2026-11-21,
/// a USB webcam's input against the built-in output over a 10 s take of 940 buffers: first and
/// last stamps gave -91.5 ppm, a line through every stamp -6.6 ppm (pulled by the step), and a
/// line through those after the first second +4.4 ppm, which the recorded ticks bear out (bar 1's
/// and bar 3's agree to 0.2 ms). A line is fit through the stamps after it when the stream ran
/// longer than [`SETTLED_AFTER_NANOS`].
const SETTLE_NANOS: u128 = 1_000_000_000;

/// A stream longer than this is fit after its first [`SETTLE_NANOS`]; a shorter one through all
/// its stamps, since too few would be left.
const SETTLED_AFTER_NANOS: u128 = 3_000_000_000;

/// A line shorter than this measures no rate: a second, too short to tell a clock's rate from
/// its stamps' jitter.
const RATED_NANOS: u128 = 1_000_000_000;

/// Running least-squares sums of stamps, each relative to the first stamp (Welford's update,
/// so a long take loses no precision and nothing is stored).
#[derive(Debug, Clone, Copy, Default)]
struct Sums {
    n: u64,
    frame: f64,
    nanos: f64,
    frame_sq: f64,
    cross: f64,
}

impl Sums {
    fn add(&mut self, frame: f64, nanos: f64) {
        self.n += 1;
        let n = self.n as f64;
        let d_frame = frame - self.frame;
        self.frame += d_frame / n;
        self.nanos += (nanos - self.nanos) / n;
        self.frame_sq += d_frame * (frame - self.frame);
        self.cross += d_frame * (nanos - self.nanos);
    }
}

/// A stream's stamps, gathered one buffer at a time, off the audio thread: the line through them
/// is [`Fit::line`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Fit {
    first: Option<Stamp>,
    last: Option<Stamp>,
    all: Sums,
    settled: Sums,
}

impl Fit {
    pub(crate) fn add(&mut self, stamp: Stamp) {
        let first = *self.first.get_or_insert(stamp);
        let frame = stamp.frame as f64 - first.frame as f64;
        let since = stamp.nanos.saturating_sub(first.nanos);
        let nanos = (stamp.nanos as i128 - first.nanos as i128) as f64;
        self.all.add(frame, nanos);
        if since >= SETTLE_NANOS {
            self.settled.add(frame, nanos);
        }
        self.last = Some(stamp);
    }

    /// The least-squares line from frames to host nanoseconds: through the stamps after the
    /// first second when the stream ran longer than three, else through all of them. None with
    /// fewer than two frames apart to draw it through.
    pub(crate) fn line(&self) -> Option<Line> {
        let (first, last) = (self.first?, self.last?);
        let span = last.nanos.saturating_sub(first.nanos);
        let sums = if span > SETTLED_AFTER_NANOS && self.settled.n >= 2 {
            self.settled
        } else {
            self.all
        };
        if sums.n < 2 || sums.frame_sq <= 0.0 {
            return None;
        }
        Some(Line {
            origin: first,
            frame: sums.frame,
            nanos: sums.nanos,
            per_frame: sums.cross / sums.frame_sq,
            span,
        })
    }
}

/// A stream's clock as a line from its frames to host nanoseconds, fit through its stamps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Line {
    /// The first stamp, which the line's own values are relative to.
    origin: Stamp,
    /// A point on the line, the stamps' mean: frames and nanoseconds after `origin`.
    frame: f64,
    nanos: f64,
    /// Host nanoseconds a frame.
    per_frame: f64,
    /// Nanoseconds from the first stamp to the last.
    span: u128,
}

impl Line {
    /// The line through `stamps`, as [`Fit::line`] draws it.
    #[cfg(test)]
    pub(crate) fn fit(stamps: &[Stamp]) -> Option<Line> {
        let mut fit = Fit::default();
        for &stamp in stamps {
            fit.add(stamp);
        }
        fit.line()
    }

    /// Frames per host second, or none when the stamps span under a second.
    pub(crate) fn rate(&self) -> Option<f64> {
        (self.span >= RATED_NANOS && self.per_frame > 0.0).then(|| 1e9 / self.per_frame)
    }

    /// The host instant, in nanoseconds, the line puts `frame` at.
    pub(crate) fn nanos_at(&self, frame: u64) -> i128 {
        let from_origin = frame as f64 - self.origin.frame as f64;
        let nanos = self.nanos + (from_origin - self.frame) * self.per_frame;
        self.origin.nanos as i128 + nanos.round() as i128
    }
}

/// The input frames, unrounded, from the input's frame 0 to where the output's frame
/// `out_frame` played: its playback instant on the output's line, less the input's line at frame
/// 0, counted at the input's measured rate (its nominal one when its line is under a second).
pub(crate) fn frames_after(input: &Line, output: &Line, out_frame: u64, nominal_in: u32) -> f64 {
    let r_in = input.rate().unwrap_or(f64::from(nominal_in));
    let since_input = output.nanos_at(out_frame) - input.nanos_at(0);
    since_input as f64 * r_in / 1e9
}

/// How fast the input's clock ran against the output's, in parts per million, from the two
/// lines' slopes against their nominal rates: positive when the input's ran fast. None when
/// either line is too short to measure.
pub(crate) fn clock_ppm(
    input: &Line,
    output: &Line,
    nominal_in: u32,
    nominal_out: u32,
) -> Option<f64> {
    let r_in = input.rate()?;
    let r_out = output.rate()?;
    Some(((r_in / f64::from(nominal_in)) / (r_out / f64::from(nominal_out)) - 1.0) * 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use duet_core::TimeSig;
    use duet_schemas::song::{MeterAt, TempoAt};

    fn song(bpm: f64, num: u8, den: u8) -> Composition {
        Composition::new(bpm, TimeSig::new(num, den).unwrap(), None)
    }

    #[test]
    fn the_count_in_is_one_bar_of_the_first_meter_and_bar_one_follows_it() {
        let ticks = Ticks::of(&song(92.0, 3, 4), 48_000, 4800, 0.0).unwrap();
        let at = |k: f64| 4800 + (k * 60.0 / 92.0 * 48_000.0).round() as u64;
        assert_eq!(
            ticks.frames,
            vec![
                (at(0.0), true),
                (at(1.0), false),
                (at(2.0), false),
                (at(3.0), true)
            ]
        );
        assert_eq!(ticks.bar_one, at(3.0));
    }

    #[test]
    fn a_beat_is_the_meters_denominator() {
        let ticks = Ticks::of(&song(120.0, 6, 8), 48_000, 0, 0.0).unwrap();
        // An eighth at 120 quarters a minute is a quarter of a second.
        assert_eq!(ticks.bar_one, 6 * 12_000);
    }

    #[test]
    fn ticks_follow_a_tempo_change_on_the_composition() {
        let mut c = song(120.0, 4, 4);
        c.tempo.push(TempoAt::new(Position::new(2, 1, 0), 60.0));
        // Bar 1 at 120 is 2 s; bar 2 at 60 is 4 s; stop before bar 3.
        let ticks = Ticks::of(&c, 1000, 0, 5.9).unwrap();
        let song_beats: Vec<u64> = ticks
            .frames
            .iter()
            .filter(|(f, _)| *f >= ticks.bar_one)
            .map(|(f, _)| f - ticks.bar_one)
            .collect();
        assert_eq!(song_beats, vec![0, 500, 1000, 1500, 2000, 3000, 4000, 5000]);
    }

    #[test]
    fn ticks_follow_a_meter_change_on_the_composition() {
        let mut c = song(60.0, 4, 4);
        c.meter
            .push(MeterAt::new(Position::bar(2), TimeSig::new(3, 4).unwrap()));
        let ticks = Ticks::of(&c, 1000, 0, 7.5).unwrap();
        let downs: Vec<u64> = ticks
            .frames
            .iter()
            .filter(|(f, d)| *d && *f >= ticks.bar_one)
            .map(|(f, _)| f - ticks.bar_one)
            .collect();
        assert_eq!(downs, vec![0, 4000, 7000]);
    }

    #[test]
    fn the_voice_writes_a_tick_only_at_its_frames_and_silence_between() {
        let ticks = Ticks {
            frames: vec![(100, true), (2000, false)],
            bar_one: 100,
        };
        let mut voice = ClickVoice::new(ticks, 48_000, 2);
        let mut out = vec![0.0_f32; 2 * 3000];
        // Two fills, so the counter carries across buffers.
        let (a, b) = out.split_at_mut(2 * 1234);
        voice.fill(a);
        voice.fill(b);
        let frame = |f: usize| (out[2 * f], out[2 * f + 1]);
        let len = 960;
        for f in 0..3000 {
            let in_tick = (100..100 + len).contains(&f) || (2000..2000 + len).contains(&f);
            let (l, r) = frame(f);
            assert_eq!(l, r);
            if !in_tick {
                assert_eq!(l, 0.0, "frame {f}");
            }
        }
        let peak =
            |from: usize, to: usize| (from..to).map(|f| frame(f).0.abs()).fold(0.0_f32, f32::max);
        assert!(peak(100, 100 + len) > 0.3 && peak(100, 100 + len) <= 0.5);
        assert!(peak(2000, 2000 + len) > 0.3 && peak(2000, 2000 + len) <= 0.5);
    }

    fn line(stamps: &[Stamp]) -> Line {
        Line::fit(stamps).expect("two stamps a frame apart or more")
    }

    fn span(frames: u64, nanos: u128) -> Line {
        line(&[
            Stamp { frame: 0, nanos: 0 },
            Stamp {
                frame: frames,
                nanos,
            },
        ])
    }

    #[test]
    fn the_offset_is_bar_ones_playback_less_the_takes_first_capture_in_whole_frames() {
        let input = line(&[
            Stamp { frame: 0, nanos: 0 },
            Stamp {
                frame: 48_000,
                nanos: 1_000_000_000,
            },
        ]);
        let output = line(&[
            Stamp {
                frame: 0,
                nanos: 5_000_000,
            },
            Stamp {
                frame: 48_000,
                nanos: 1_005_000_000,
            },
        ]);
        assert_eq!(frames_after(&input, &output, 4800, 48_000).round(), 5040.0);
    }

    #[test]
    fn a_take_that_started_after_bar_one_has_bar_one_before_its_first_frame() {
        let input = line(&[
            Stamp {
                frame: 0,
                nanos: 2_000_000_000,
            },
            Stamp {
                frame: 48_000,
                nanos: 3_000_000_000,
            },
        ]);
        let output = span(48_000, 1_000_000_000);
        assert!(frames_after(&input, &output, 0, 48_000) < 0.0);
    }

    #[test]
    fn matched_clocks_drift_zero_ppm_and_a_fast_input_drifts_positive() {
        let output = span(48_000_000, 1_000_000_000_000);
        let matched = span(48_000_000, 1_000_000_000_000);
        assert!(clock_ppm(&matched, &output, 48_000, 48_000).unwrap().abs() < 1e-9);
        let fast = span(48_002_400, 1_000_000_000_000);
        let ppm = clock_ppm(&fast, &output, 48_000, 48_000).unwrap();
        assert!((ppm - 50.0).abs() < 0.01, "{ppm}");
    }

    /// Every 512 frames for `secs` at 48 kHz on the host's clock from `start` nanoseconds, the
    /// buffers of the first second stamped `early` nanoseconds early: a clock model settling, as
    /// a USB input's does on macOS (measured 0.9 ms on the owner's Mac mini, 2026-11-21).
    fn settling(start: u128, secs: u64, early: u128) -> Vec<Stamp> {
        (0..secs * 48_000 / 512)
            .map(|k| {
                let frame = k * 512;
                let on_line = start + u128::from(frame) * 1_000_000_000 / 48_000;
                let nanos = if frame < 48_000 {
                    on_line - early
                } else {
                    on_line
                };
                Stamp { frame, nanos }
            })
            .collect()
    }

    /// What the stamps measure the stream by.
    fn measured(stamps: &[Stamp]) -> Line {
        line(stamps)
    }

    #[test]
    fn a_settling_step_in_the_first_second_moves_neither_the_offset_nor_the_drift() {
        // The input settles 0.9 ms; the output, 5 ms behind it, runs straight. Both clocks are
        // the host's, so there is no drift, and bar 1 (output frame 4800) plays 105 ms after the
        // input's line puts its frame 0: 5040 frames.
        let input = measured(&settling(1_000_000_000, 10, 900_000));
        let output = measured(&settling(1_005_000_000, 10, 0));
        assert_eq!(frames_after(&input, &output, 4800, 48_000).round(), 5040.0);
        let ppm = clock_ppm(&input, &output, 48_000, 48_000).unwrap();
        assert!(ppm.abs() < 0.01, "{ppm}");
    }

    #[test]
    fn a_span_under_a_second_measures_no_drift() {
        let short = span(24_000, 500_000_000);
        let long = span(48_000, 1_000_000_000);
        assert_eq!(short.rate(), None);
        assert_eq!(clock_ppm(&short, &long, 48_000, 48_000), None);
        assert_eq!(clock_ppm(&long, &short, 48_000, 48_000), None);
    }
}
