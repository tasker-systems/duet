//! `duet capture`: a recorded take in; the exact performance (`.duet`, a part document), a
//! quantized score (`.abc` for a melody, or `.grid.duet`, a drum-grid document, for a rhythm)
//! and the analysis (`.capture.duet`, a `capture-analysis` document) out. Every document is
//! written through duet-schemas.
//!
//! Every estimate is stated (`statement`) on standard error, in each document's envelope
//! `statements`, and as a `%` comment in an `.abc`. Every refusal comes before anything is written, and nothing is
//! overwritten. Once the take is analysed, the performance (`.duet` and `.capture.duet`) is
//! written whatever becomes of the score, which is written, or reported as failed, after it.
//!
//! A melody take also writes its every pitch frame, as a `performance` stream,
//! `STEM.performance.duet.jsonl`, through duet-schemas (`PerformanceFile`). Its file is
//! created before the take is read, and its lines written once the take has been read, since
//! its head states the take's digest ([`Digester`]), which is known only then; until then they
//! are held, as the frames are for the reading. It is kept once the `.duet` and `.capture.duet`
//! are written, and removed again when the capture fails before then.
//!
//! Both the analysis and the performance state the take they were read from (`take`): the
//! SHA-256 of its decoded samples, with their rate, channel count and length, computed from
//! every chunk the source yields, beside `Prepare`.
//!
//! `duet capture compare` ([`compare`]) reads a melody take's recording and those files back,
//! and plots and plays its reading against its performance. `duet capture review` ([`review`])
//! does it for each moment of a moments file, every reading side by side, on one page.

pub mod compare;
pub mod review;
mod statement;

use std::fs;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use duet_abc::{Dynamics, WriteOptions, off_level_velocities};
use duet_capture::{
    A440_CENTS, ANALYSIS_RATE, Digester, Error, Event, FileSource, Given, Hits, Kind, LaneMode,
    Layer1, Mono, NoteModel, Performance, PerformanceFrames, PerformanceHead, Prepare, Quantized,
    ReadNote, Source, Stage, a4_cents_of_hz, assemble, estimate, merge_collisions, quantize_apart,
    quantize_offsets, read,
};
#[cfg(feature = "basic-pitch")]
use duet_capture::{BasicPitch, Thresholds};
use duet_core::{Control, Grid, Key, Part, Position, TimeMap, TimeSig};
use duet_schemas::{Document, Stream};

use crate::Context;
use crate::config::Config;
use crate::drums::{ReadOptions, grid_of_part, refuse_overwrite};
use crate::io::{tool, with_end};
use schemars::JsonSchema;

#[derive(Debug, Subcommand)]
pub enum CaptureCommand {
    /// Capture a sung or played melody: notes, their pitches and the key, written as
    /// STEM.duet (exactly as played) and STEM.abc (quantized to --grid), with every pitch
    /// frame in STEM.performance.duet.jsonl.
    Melody(TakeArgs),
    /// Capture a tapped or beatboxed rhythm: hits in lanes, written as STEM.duet (exactly
    /// as played) and STEM.grid.duet (quantized to --grid). Without --lane, each hit is told
    /// apart by sound as a kick, snare or closed hat.
    Rhythm(TakeArgs),
    /// Compare a melody take's reading against its performance: STEM.compare.svg plots the
    /// performance's pitch with the reading's notes at their keys on the take's tuning, and the
    /// residual between them; STEM.compare.wav holds the take on the left and the reading on the
    /// right as a sine, held through a gap under 0.2 s and silent through a longer one; standard
    /// output lists each note. A window (--from, --to) is named after it, as
    /// STEM.compare.FROM-TO.svg and .wav. Reads the take's recording and what `duet capture
    /// melody` wrote at STEM; nothing is overwritten.
    Compare(compare::CompareArgs),
    /// Review a melody take moment by moment, on one page: STEM.review/index.html shows each
    /// moment of --moments with every reading side by side (the notes each reads, compare's plot
    /// inline, and its clips as melody, steps, bends, legato and gapped), a box for a verdict, and
    /// a button that copies every verdict. The clips are in STEM.review/clips. Reads what `duet
    /// capture melody` wrote at STEM and each --reading; a directory that exists is refused.
    Review(review::ReviewArgs),
}

/// What both kinds of take are captured with. Anything about the timing left out is
/// estimated from the take and stated.
#[derive(Debug, Args, JsonSchema)]
pub struct TakeArgs {
    /// The recording: WAV, FLAC, MP3, Ogg Vorbis, or AAC in an MP4 (.m4a).
    input: PathBuf,
    /// The tempo, in quarter notes per minute, used exactly. A melody's must be a whole
    /// number: its ABC states the tempo in Q:, which holds only a whole number.
    #[arg(long)]
    tempo: Option<f64>,
    /// The meter, such as 4/4 or 6/8, used exactly.
    #[arg(long)]
    meter: Option<TimeSig>,
    /// A melody's key, as ABC names it in K: (Eb, Bm, F#m), used exactly. It changes how
    /// notes are spelled, never which notes they are.
    #[arg(long)]
    key: Option<String>,
    /// Where bar 1 starts, in seconds from the start of the recording.
    #[arg(long)]
    downbeat: Option<f64>,
    /// A melody's tuning: A4 in Hz (440, 442) or in cents from 440 Hz (+44c, -12c), used
    /// exactly. Without it, the tuning is estimated from the take and stated.
    #[arg(long, allow_hyphen_values = true)]
    tuning: Option<TuningArg>,
    /// What chooses a melody's notes from its performance: `hmm`, the note HMM (the default);
    /// `rules`, the segmentation rules; or `basic-pitch`, Spotify's Basic Pitch, an outside
    /// model, which chooses where notes start and end while the performance gives each its key
    /// on the take's tuning. A reader other than the HMM is stated when chosen, and the reader
    /// is written in STEM.capture.duet either way.
    #[arg(long, value_enum)]
    reader: Option<ReaderArg>,
    /// With `--reader basic-pitch`: the onset activation, 0 to 1, at which Basic Pitch starts a
    /// note. Its own default is 0.5; higher starts fewer notes.
    #[cfg(feature = "basic-pitch")]
    #[arg(long, value_name = "ACTIVATION", allow_hyphen_values = true)]
    onset_threshold: Option<f64>,
    /// With `--reader basic-pitch`: the note activation, 0 to 1, below which Basic Pitch ends a
    /// note. Its own default is 0.3; lower holds a note through a deeper dip.
    #[cfg(feature = "basic-pitch")]
    #[arg(long, value_name = "ACTIVATION", allow_hyphen_values = true)]
    frame_threshold: Option<f64>,
    /// With `--reader basic-pitch`: Basic Pitch keeps only notes longer than this, in
    /// milliseconds. Its own default is 127.7.
    #[cfg(feature = "basic-pitch")]
    #[arg(long, value_name = "MS", allow_hyphen_values = true)]
    min_note_ms: Option<f64>,
    /// The step the score is quantized to, such as 1/16 (the default) or 1/8t. Where two
    /// notes, or two hits on one lane, would share a step, the step is halved, down to 1/64,
    /// and what still shares one is merged; each is stated. The .duet is never quantized.
    #[arg(long)]
    grid: Option<Grid>,
    /// A rhythm's lane, such as hh or kick: every hit goes to it (a tapped take).
    #[arg(long)]
    lane: Option<String>,
    /// The output files' stem: STEM.duet, STEM.abc or STEM.grid.duet, and STEM.capture.duet.
    /// Without it, the input's name without its extension, beside it.
    #[arg(short, long, value_name = "STEM")]
    output: Option<PathBuf>,
}

/// A melody's reader, as `--reader` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum ReaderArg {
    Rules,
    Hmm,
    #[cfg(feature = "basic-pitch")]
    BasicPitch,
}

/// Which kind of take is captured.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Take {
    Melody,
    Rhythm,
}

pub fn run(ctx: &Context, command: CaptureCommand) -> Result<()> {
    let config = ctx.config();
    let (take, args) = match command {
        CaptureCommand::Melody(args) => (Take::Melody, args),
        CaptureCommand::Rhythm(args) => (Take::Rhythm, args),
        CaptureCommand::Compare(args) => return compare::run(&args),
        CaptureCommand::Review(args) => return review::run(&args),
    };
    let key = flags(take, &args)?;
    let grid = match args.grid {
        Some(grid) => grid,
        None => DEFAULT_GRID.parse()?,
    };
    // Refused before anything is read, like every refusal: a score on this grid could not be
    // quantized at all.
    grid.step_ticks(Part::default().ticks_per_quarter)
        .with_context(|| format!("--grid {grid}"))?;
    let outputs = Outputs::new(&args.input, args.output.as_deref(), take)?;
    // Basic Pitch is loaded, and its thresholds checked, before anything is read.
    #[cfg(feature = "basic-pitch")]
    let basic_pitch = basic_pitch_of(&args)?;

    // Every failure from here on is the take's, so it names the take. FileSource's own errors
    // name it already.
    let named = || args.input.display().to_string();
    // Basic Pitch reads the take's samples themselves, so they are kept for it.
    #[cfg(feature = "basic-pitch")]
    let keep = args.reader == Some(ReaderArg::BasicPitch);
    #[cfg(not(feature = "basic-pitch"))]
    let keep = false;
    let (mut events, kept, samples, audio) =
        analyse(&args.input, take, args.lane.is_some(), &outputs, keep)?;
    #[cfg(not(feature = "basic-pitch"))]
    let _ = samples;
    let mut read_said: Vec<String> = Vec::new();
    // A melody's notes are read from its performance, on its own tuning.
    let (performance, reading) = match kept {
        Some((file, frames)) => {
            let given = args.tuning.map(|t| t.a4_cents);
            let reading = frames
                .map(|frames| -> Result<_> {
                    Ok(match args.reader.unwrap_or(ReaderArg::Hmm) {
                        ReaderArg::Rules => read(&frames, &events, given),
                        ReaderArg::Hmm => NoteModel::default().read(&frames, &events, given),
                        #[cfg(feature = "basic-pitch")]
                        ReaderArg::BasicPitch => {
                            let read = basic_pitch
                                .as_ref()
                                .context("basic-pitch was not loaded")?
                                .read(&samples, &frames, &events, given)?;
                            read_said.push(statement::basic_pitch(&read));
                            read.reading
                        }
                    })
                })
                .transpose()
                .with_context(named)?;
            if let Some(reading) = &reading {
                events.extend(reading.notes.iter().map(ReadNote::event));
                events.sort_by_key(event_frame);
            }
            (Some(file), reading)
        }
        None => (None, None),
    };
    let what = match take {
        Take::Melody => "notes",
        Take::Rhythm => "hits",
    };
    // Checked before the tempo is estimated: silence has no onsets either, and "too few onsets,
    // give --tempo" would send the user after the wrong fix.
    if !events.iter().any(|e| match take {
        Take::Melody => matches!(e, Event::Note { .. }),
        Take::Rhythm => matches!(e, Event::Hit { .. }),
    }) {
        return Err(anyhow::Error::from(Error::NothingFound { what }).context(named()));
    }
    let given = Given {
        tempo_bpm: args.tempo,
        meter: args.meter,
        downbeat_secs: args.downbeat,
    };
    let timing = estimate(&events, ANALYSIS_RATE, &given).with_context(named)?;
    let kind = match take {
        Take::Melody => Kind::Melody,
        Take::Rhythm => Kind::Rhythm {
            lanes: config.lanes.clone(),
            asked: args.lane.clone(),
        },
    };
    let mut captured = assemble(&events, ANALYSIS_RATE, &timing, key, &kind).with_context(named)?;
    captured.analysis.take = audio;

    // The score's grid is settled before the analysis is written, since the analysis gives
    // each note's quantize offset in its steps. A failure here is the score's, reported once
    // the performance is written.
    let ms_per_quarter = 1000.0 * timing.beat_period_frames / f64::from(ANALYSIS_RATE);
    let quantized = quantize_apart(&captured.part, grid, &kind).and_then(|q| {
        let offsets = quantize_offsets(&captured.part, &q.part, q.grid, ms_per_quarter)?;
        Ok((q, offsets))
    });
    if let Ok((q, offsets)) = &quantized {
        captured.analysis.grid = Some(q.grid);
        captured.analysis.offsets.clone_from(offsets);
    }

    let meter = *timing.meter.value();
    let advice = reading
        .as_ref()
        .and_then(|reading| statement::tuning_advice(&reading.tuning));
    let mut said = statement::timing(&timing, ANALYSIS_RATE);
    if let Some(reading) = &reading {
        said.extend(statement::tuning(&reading.tuning));
        said.extend(statement::reader(reading));
        said.append(&mut read_said);
        said.push(statement::notes_read(reading));
        said.extend(statement::breathy(reading, ANALYSIS_RATE));
        said.extend(statement::rejoined(reading));
        said.extend(statement::held(reading));
        said.extend(statement::suggestions(reading, ANALYSIS_RATE));
    }
    said.extend(statement::key(captured.analysis.key.as_ref()));
    captured.analysis.reading = reading;
    let lead_in = statement::lead_in(
        captured.lead_in,
        captured.downbeat_tick,
        meter,
        captured.part.ticks_per_quarter,
    );
    let part_said = [said.clone(), vec![lead_in.clone()]].concat();
    // The part is the take's reading: a one-take comp, naming the take by its digest and the
    // reader that read it. A take that held no audio has no digest, and its part was made by
    // capture from nothing.
    let source = match &captured.analysis.take {
        Some(take) => {
            let reader = captured
                .analysis
                .reading
                .as_ref()
                .and_then(|reading| reading.reader.as_ref())
                .and_then(|reader| serde_json::to_value(reader.name).ok())
                .and_then(|name| name.as_str().map(String::from));
            duet_schemas::Source::reading(take.digest, reader)
        }
        None => crate::io::generated("capture", []),
    };
    let body = duet_schemas::PartBody::new(
        crate::io::fresh_id()?,
        source,
        with_end(&captured.part, config.end)?,
    );
    let mut part = Document::new(tool(), body);
    part.statements.clone_from(&part_said);
    let part = part.write()?;
    let analysis = Document::new(tool(), captured.analysis).write()?;

    // The performance is written whatever becomes of the score.
    for line in &part_said {
        eprintln!("{line}");
    }
    if let Some(advice) = advice {
        eprintln!("{advice}");
    }
    write_all(&[(&outputs.part, &part), (&outputs.analysis, &analysis)])?;
    if let Some(performance) = performance {
        ctx.wrote(&performance.keep(), false)?;
    }
    ctx.wrote(&outputs.part, false)?;
    ctx.wrote(&outputs.analysis, false)?;

    let wrote = || {
        format!(
            "{}: {} not written; {} and {} hold the performance",
            named(),
            outputs.score.display(),
            outputs.part.display(),
            outputs.analysis.display()
        )
    };
    let (quantized, _) = quantized.map_err(|e| anyhow::Error::from(e).context(wrote()))?;
    let scored = Scoring {
        take,
        kind: &kind,
        grid,
        grid_given: args.grid.is_some(),
        downbeat_tick: captured.downbeat_tick,
        meter,
        said,
        lead_in,
        config,
    }
    .score(quantized)
    .with_context(wrote)?;
    for line in &scored.stated {
        eprintln!("{line}");
    }
    write_new(&outputs.score, &scored.text).with_context(wrote)?;
    ctx.wrote(&outputs.score, false)
}

/// The grid a score is quantized to when no `--grid` is given.
const DEFAULT_GRID: &str = "1/16";

/// What a take's score is made from, besides the quantized part.
struct Scoring<'a> {
    take: Take,
    kind: &'a Kind,
    /// The grid asked for, and whether `--grid` asked for it.
    grid: Grid,
    grid_given: bool,
    downbeat_tick: u64,
    meter: TimeSig,
    /// The estimates the performance states, which the score states too.
    said: Vec<String>,
    /// What the performance states about its lead-in.
    lead_in: String,
    config: &'a Config,
}

/// A score's text, and the lines it states beyond the performance's, for standard error.
struct Scored {
    text: String,
    stated: Vec<String>,
}

impl Scoring<'_> {
    /// The score of `quantized`: its empty lead-in removed, what still shares a step merged,
    /// a melody's velocities snapped to the ABC's dynamics, and each of those stated. A
    /// rhythm's score is a drum-grid document, with those statements in its envelope and every
    /// velocity kept.
    fn score(self, quantized: Quantized) -> Result<Scored> {
        let mut stated = Vec::new();
        if quantized.grid != self.grid {
            stated.push(statement::refined(
                self.grid,
                self.grid_given,
                quantized.grid,
                self.take == Take::Rhythm,
            ));
        }
        let trimmed = without_empty_lead_in(&quantized.part, self.downbeat_tick, self.meter)?;
        if trimmed.is_some() {
            stated.push(statement::LEAD_IN_REMOVED.to_owned());
        }
        let mut score = trimmed.unwrap_or(quantized.part);
        for merge in merge_collisions(&mut score, self.kind)? {
            stated.push(match self.take {
                Take::Melody => statement::merged_notes(&merge),
                Take::Rhythm => {
                    let lane = self
                        .config
                        .lanes
                        .by_key(merge.kept.pitch.midi())
                        .map_or_else(
                            || format!("key {}", merge.kept.pitch.midi()),
                            |l| l.name.clone(),
                        );
                    statement::merged_hits(&merge, &lane)
                }
            });
        }

        let mut score_said = self.said;
        if !stated.iter().any(|l| l == statement::LEAD_IN_REMOVED) {
            score_said.push(self.lead_in);
        }
        score_said.extend(stated.iter().cloned());
        let text = match self.take {
            Take::Melody => {
                let options = WriteOptions::default().with_dynamics(Dynamics::Nearest);
                let abc = duet_abc::write(&with_end(&score, self.config.end)?, &options)?;
                // The ABC writer states the snap in the file itself.
                let moved = off_level_velocities(&score);
                if moved > 0 {
                    stated.push(statement::snapped(
                        moved,
                        "the nearest dynamic's level in the .abc",
                    ));
                }
                commented(&abc, "%", &score_said)
            }
            Take::Rhythm => {
                let options = ReadOptions {
                    grid: quantized.grid,
                    track: None,
                    bars: None,
                    snap_levels: false,
                };
                // Every velocity kept exactly: a drum-grid document holds any velocity, so nothing
                // is snapped to the text's levels, and nothing needs stating.
                let grid = grid_of_part(&score, &options, self.config)?;
                let mut doc = Document::new(tool(), grid);
                doc.statements = score_said;
                doc.write()?
            }
        };
        Ok(Scored { text, stated })
    }
}

/// Refuses a flag the take cannot use, before anything is read, and reads the key.
/// Basic Pitch, loaded with the thresholds given, when it reads the take. A threshold given
/// for another reader is refused: it would change nothing.
#[cfg(feature = "basic-pitch")]
fn basic_pitch_of(args: &TakeArgs) -> Result<Option<BasicPitch>> {
    let given = [
        ("--onset-threshold", args.onset_threshold),
        ("--frame-threshold", args.frame_threshold),
        ("--min-note-ms", args.min_note_ms),
    ];
    if args.reader != Some(ReaderArg::BasicPitch) {
        if let Some((flag, _)) = given.iter().find(|(_, v)| v.is_some()) {
            bail!("{flag} is for --reader basic-pitch; no other reader decodes with it");
        }
        return Ok(None);
    }
    let own = Thresholds::BASIC_PITCH;
    let thresholds = Thresholds {
        onset_threshold: args.onset_threshold.unwrap_or(own.onset_threshold),
        frame_threshold: args.frame_threshold.unwrap_or(own.frame_threshold),
        minimum_note_length_ms: args.min_note_ms.unwrap_or(own.minimum_note_length_ms),
    };
    Ok(Some(BasicPitch::new()?.with_thresholds(thresholds)?))
}

fn flags(take: Take, args: &TakeArgs) -> Result<Option<Key>> {
    if let Some(bpm) = args.tempo
        && !(bpm.is_finite() && bpm > 0.0)
    {
        bail!("--tempo {bpm}: a tempo must be a positive number of quarter notes a minute");
    }
    match take {
        Take::Melody => {
            if args.lane.is_some() {
                bail!("--lane is for a rhythm take; a melody's notes are pitched, not in lanes");
            }
            if let Some(bpm) = args.tempo
                && bpm.is_finite()
                && bpm.fract() != 0.0
            {
                bail!(
                    "--tempo {bpm}: a melody's tempo must be a whole number of bpm, because \
                     its ABC states the tempo as Q:1/4=<bpm> and ABC's Q: holds only a whole \
                     number; give --tempo {} or {}",
                    bpm.floor(),
                    bpm.ceil()
                );
            }
        }
        Take::Rhythm => {
            if args.key.is_some() {
                bail!("--key is for a melody take; a rhythm take has no key");
            }
            if args.tuning.is_some() {
                bail!("--tuning is for a melody take; a rhythm take's hits have no pitch");
            }
            if args.reader.is_some() {
                bail!("--reader is for a melody take; a rhythm take's hits are not read as notes");
            }
        }
    }
    args.key
        .as_deref()
        .map(|name| duet_abc::key_from_name(name).with_context(|| format!("--key {name}")))
        .transpose()
}

/// The files a capture writes, each checked before any is written.
struct Outputs {
    /// The performance as a part document.
    part: PathBuf,
    score: PathBuf,
    analysis: PathBuf,
    /// A melody's performance stream.
    performance: Option<PathBuf>,
}

impl Outputs {
    /// The outputs at `stem`, or at the input's name without its extension. Refused when one
    /// is the input or already exists, since capture never overwrites a file.
    fn new(input: &Path, stem: Option<&Path>, take: Take) -> Result<Outputs> {
        let stem = stem.map_or_else(|| input.with_extension(""), Path::to_path_buf);
        let named = |ext: &str| {
            let mut path = stem.clone().into_os_string();
            path.push(ext);
            PathBuf::from(path)
        };
        let outputs = Outputs {
            part: named(".duet"),
            score: named(match take {
                Take::Melody => ".abc",
                Take::Rhythm => ".grid.duet",
            }),
            analysis: named(".capture.duet"),
            performance: (take == Take::Melody).then(|| named(".performance.duet.jsonl")),
        };
        let all: Vec<&PathBuf> = [&outputs.part, &outputs.score, &outputs.analysis]
            .into_iter()
            .chain(&outputs.performance)
            .collect();
        for &path in &all {
            refuse_overwrite(input, path, "capture")?;
        }
        for path in all {
            if path.exists() {
                bail!(
                    "{} exists; capture writes new files and never overwrites one, so move it \
                     or name another stem with -o",
                    path.display()
                );
            }
        }
        Ok(outputs)
    }
}

/// A melody's tuning as `--tuning` gives it: A4, in absolute cents.
#[derive(Debug, Clone, Copy)]
pub struct TuningArg {
    a4_cents: f64,
}

/// Given as `--tuning` spells it, so its schema is a string's.
impl JsonSchema for TuningArg {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("TuningArg")
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A4 in Hz (440, 442) or in cents from 440 Hz (+0c, -12c), at most 100 cents either way.",
        })
    }
}

/// The furthest, in cents, a given tuning may sit from 440 Hz: a key either way. Further is a
/// take in another key, not another tuning.
const MAX_TUNING_CENTS: f64 = 100.0;

impl std::str::FromStr for TuningArg {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<TuningArg, String> {
        let text = s.trim();
        let offset = if let Some(cents) = text.strip_suffix('c') {
            let cents: f64 = cents.parse().map_err(|_| {
                format!("{s:?}: cents from 440 Hz are a signed number before c, such as +44c")
            })?;
            if !cents.is_finite() {
                return Err(format!("{s:?}: cents must be a finite number"));
            }
            cents
        } else {
            let hz: f64 = text.parse().map_err(|_| {
                format!("{s:?}: a tuning is A4 in Hz (440, 442) or cents from 440 Hz (+44c)")
            })?;
            if !(hz.is_finite() && hz > 0.0) {
                return Err(format!("{s:?}: A4 in Hz must be a positive number"));
            }
            a4_cents_of_hz(hz) - A440_CENTS
        };
        if offset.abs() > MAX_TUNING_CENTS {
            return Err(format!(
                "{s:?} is {offset:+.0} cents from 440 Hz; a tuning is at most \
                 {MAX_TUNING_CENTS} cents (a key) either way of it"
            ));
        }
        Ok(TuningArg {
            a4_cents: A440_CENTS + offset,
        })
    }
}

/// Where an event sits on the take's timeline, which events are kept in the order of.
fn event_frame(event: &Event) -> u64 {
    match *event {
        Event::Onset { frame, .. } | Event::Hit { frame, .. } => frame,
        Event::Note { start, .. } => start,
        _ => u64::MAX,
    }
}

/// The take's analysis stage. A melody's is its performance, written to the file as it
/// settles and kept as frames for its reading; a rhythm's gives its hits.
enum Analyser {
    Melody {
        stage: Box<Performance>,
        file: PerformanceFile,
        frames: Option<PerformanceFrames>,
    },
    Rhythm(Box<Hits>),
}

impl Analyser {
    fn push(&mut self, mono: Mono, events: &mut Vec<Event>) -> Result<()> {
        match self {
            Self::Melody { stage, .. } => {
                let mut kept = Vec::new();
                stage.push(mono, &mut kept)?;
                self.keep(kept, events)
            }
            Self::Rhythm(hits) => Ok(hits.push(mono, events)?),
        }
    }

    fn finish(&mut self, events: &mut Vec<Event>) -> Result<()> {
        match self {
            Self::Melody { stage, .. } => {
                let mut kept = Vec::new();
                stage.finish(&mut kept)?;
                self.keep(kept, events)
            }
            Self::Rhythm(hits) => Ok(hits.finish(events)?),
        }
    }

    /// Writes what the performance settled to its file, keeps its frames, and passes its
    /// onsets on as events.
    fn keep(&mut self, kept: Vec<Layer1>, events: &mut Vec<Event>) -> Result<()> {
        let Self::Melody { file, frames, .. } = self else {
            return Ok(());
        };
        for layer in &kept {
            match layer {
                Layer1::Head(head) => *frames = Some(PerformanceFrames::new(head)),
                Layer1::Frames(block) => frames
                    .as_mut()
                    .context("a performance's frames came before its head")?
                    .push(block)?,
                &Layer1::Onset { frame, strength } => {
                    events.push(Event::Onset { frame, strength });
                }
                _ => {}
            }
        }
        file.write(kept)
    }
}

/// The take's settled events: the file through `Prepare` and the take's analysis stage, to
/// the end of the stream, and every chunk through the take's [`Digester`]. For a melody, the
/// events are its onsets, and its performance file, written in full but not yet kept, and its
/// frames come with them. The audio's digest is last: `None` when the file held no audio.
fn analyse(
    input: &Path,
    take: Take,
    asked: bool,
    outputs: &Outputs,
    keep_samples: bool,
) -> Result<Analysed> {
    let mut source = FileSource::open(input)?;
    let mut prepare = Prepare::new(ANALYSIS_RATE);
    let mut stage = match (take, &outputs.performance) {
        (Take::Melody, Some(path)) => Analyser::Melody {
            stage: Box::new(Performance::new(ANALYSIS_RATE)),
            file: PerformanceFile::create(path)?,
            frames: None,
        },
        (Take::Melody, None) => bail!("a melody take has a performance file"),
        (Take::Rhythm, _) => Analyser::Rhythm(Box::new(Hits::new(
            ANALYSIS_RATE,
            if asked {
                LaneMode::Asked
            } else {
                LaneMode::Beatbox
            },
        ))),
    };
    let (mut mono, mut events, mut samples) = (Vec::new(), Vec::new(), Vec::new());
    // The source's errors name the file; the stages' are named here.
    let named = || input.display().to_string();
    let mut digester = Digester::new();
    while let Some(chunk) = source.next_chunk()? {
        digester.push(&chunk).with_context(named)?;
        prepare.push(chunk, &mut mono).with_context(named)?;
        for m in mono.drain(..) {
            if keep_samples {
                samples.extend_from_slice(&m.samples);
            }
            stage.push(m, &mut events).with_context(named)?;
        }
    }
    prepare.finish(&mut mono).with_context(named)?;
    for m in mono.drain(..) {
        if keep_samples {
            samples.extend_from_slice(&m.samples);
        }
        stage.push(m, &mut events).with_context(named)?;
    }
    stage.finish(&mut events).with_context(named)?;
    let audio = digester.finish();
    let kept = match stage {
        Analyser::Melody {
            mut file, frames, ..
        } => {
            file.finish(audio)?;
            Some((file, frames))
        }
        Analyser::Rhythm(_) => None,
    };
    Ok((events, kept, samples, audio))
}

/// What [`analyse`] gives: the take's events, a melody's performance, the samples when they are
/// kept, and the take's digest.
type Analysed = (
    Vec<Event>,
    Option<Kept>,
    Vec<f32>,
    Option<duet_capture::Take>,
);

/// A melody's performance file, written but not yet kept, and its frames: none when the take
/// held no audio at all.
type Kept = (PerformanceFile, Option<PerformanceFrames>);

/// A melody's `performance` stream, written through duet-schemas: the envelope and head first,
/// then one block of frames a line. The head states the take ([`finish`](PerformanceFile::finish)),
/// whose digest is known only once the whole take is read, so each line is made as its frames
/// settle and held until then. It is created new, and removed again when dropped before
/// [`keep`](PerformanceFile::keep), so a capture that fails leaves it behind no more than its
/// other outputs.
struct PerformanceFile {
    path: PathBuf,
    file: Option<BufWriter<fs::File>>,
    /// Boxed: with the take it states, it is the file's largest part.
    head: Option<Box<PerformanceHead>>,
    /// Each block of frames' line, in order, until the head is written.
    lines: Vec<String>,
}

impl PerformanceFile {
    /// Creates the file, which must not exist yet.
    fn create(path: &Path) -> Result<PerformanceFile> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| path.display().to_string())?;
        Ok(PerformanceFile {
            path: path.to_path_buf(),
            file: Some(BufWriter::new(file)),
            head: None,
            lines: Vec::new(),
        })
    }

    /// Keeps the head, the first time it comes, and makes every block of frames' line.
    fn write(&mut self, kept: Vec<Layer1>) -> Result<()> {
        for layer in kept {
            match layer {
                Layer1::Head(head) if self.head.is_none() => self.head = Some(Box::new(head)),
                Layer1::Frames(block) if self.head.is_some() => {
                    self.lines
                        .push(Stream::<PerformanceHead>::record_line(&block)?)
                }
                Layer1::Frames(_) => bail!("a performance's frames came before its head"),
                _ => {}
            }
        }
        Ok(())
    }

    /// Writes the head, stating `take`, then every line held, once the take is read. A take
    /// whose performance never began (no audio) writes nothing.
    fn finish(&mut self, take: Option<duet_capture::Take>) -> Result<()> {
        let Some(mut head) = self.head.as_deref().copied() else {
            return Ok(());
        };
        head.take = take;
        let first = Stream::<PerformanceHead>::new(tool(), head).head_line()?;
        let lines = std::mem::take(&mut self.lines);
        let path = self.path.display().to_string();
        let out = self.out()?;
        for line in std::iter::once(&first).chain(&lines) {
            out.write_all(line.as_bytes())
                .with_context(|| path.clone())?;
        }
        out.flush().with_context(|| path.clone())
    }

    fn out(&mut self) -> Result<&mut BufWriter<fs::File>> {
        self.file
            .as_mut()
            .with_context(|| format!("{} is already closed", self.path.display()))
    }

    /// Keeps the file, the capture having written its performance, and gives back its path.
    fn keep(mut self) -> PathBuf {
        self.file = None;
        std::mem::take(&mut self.path)
    }
}

impl Drop for PerformanceFile {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The quantized score of an assembled take with its lead-in taken out, when quantizing has
/// left no note in the lead-in: every note and control moves back by `downbeat_tick`, the
/// part starts in `meter`, and the change to it at bar 2 is dropped. `None` when the score
/// has no lead-in, or a note or control still lies in it.
///
/// The part is one `assemble` built, so its only meter change is the one after a pickup, and
/// it has no tempo or key changes and names no end.
fn without_empty_lead_in(score: &Part, downbeat_tick: u64, meter: TimeSig) -> Result<Option<Part>> {
    if downbeat_tick == 0 {
        return Ok(None);
    }
    let from = score.time_map()?;
    let to = TimeMap::new(score.ticks_per_quarter, meter)?;
    let moved = |at: Position| -> Result<Option<Position>> {
        let tick = from.tick(at)?;
        Ok(tick.checked_sub(downbeat_tick).map(|t| to.position(t)))
    };
    let mut out = score.clone();
    out.time_sig = meter;
    out.time_changes.retain(|c| c.at != Position::bar(2));
    for track in &mut out.tracks {
        for note in &mut track.notes {
            let Some(at) = moved(note.at)? else {
                return Ok(None);
            };
            note.at = at;
        }
        for control in &mut track.controls {
            let at = match control {
                Control::Cc { at, .. } | Control::Bend { at, .. } | Control::Program { at, .. } => {
                    at
                }
                other => bail!("capture cannot move a {other:?} out of a lead-in"),
            };
            let Some(new) = moved(*at)? else {
                return Ok(None);
            };
            *at = new;
        }
    }
    Ok(Some(out))
}

/// `text` with `lines` as comments starting `prefix`, after its first line: the notefmt or
/// drum grid version line, or the ABC tune's `X:`.
fn commented(text: &str, prefix: &str, lines: &[String]) -> String {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let mut out = format!("{first}\n");
    for line in lines {
        out.push_str(&format!("{prefix} {line}\n"));
    }
    out.push_str(rest);
    out
}

/// Writes each file, none of which may exist yet. When one fails, the ones this call wrote
/// are removed again, so a failed capture leaves nothing behind, and the error says so.
fn write_all(files: &[(&Path, &str)]) -> Result<()> {
    for (i, (path, text)) in files.iter().enumerate() {
        if let Err(e) = write_new(path, text) {
            let removed: Vec<String> = files[..i]
                .iter()
                .filter(|(p, _)| fs::remove_file(p).is_ok())
                .map(|(p, _)| p.display().to_string())
                .collect();
            if removed.is_empty() {
                return Err(e);
            }
            return Err(e.context(format!(
                "capture wrote nothing: removed {} again",
                removed.join(" and ")
            )));
        }
    }
    Ok(())
}

/// Writes a file that must not exist yet. A file it created but could not fill is removed.
fn write_new(path: &Path, text: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| path.display().to_string())?;
    if let Err(e) = file.write_all(text.as_bytes()) {
        let _ = fs::remove_file(path);
        return Err(e).with_context(|| path.display().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use duet_core::{Note, Pitch, TimeChange, Track};

    fn sig(s: &str) -> TimeSig {
        s.parse().expect("a time signature")
    }

    /// A part with a one-beat pickup before 4/4 and notes at `ticks`.
    fn with_pickup(ticks: &[u64]) -> Part {
        let mut part = Part::default();
        part.time_sig = sig("1/4");
        part.time_changes.push(TimeChange {
            at: Position::bar(2),
            time_sig: sig("4/4"),
        });
        let map = part.time_map().expect("map");
        let mut track = Track::new("melody", 1);
        for &t in ticks {
            track.notes.push(Note::new(
                map.position(t),
                Pitch::new(60).expect("C4"),
                1.0,
                80,
            ));
        }
        part.tracks.push(track);
        part
    }

    #[test]
    fn an_empty_pickup_is_taken_out() {
        let part = with_pickup(&[480, 960, 2400]);
        let out = without_empty_lead_in(&part, 480, sig("4/4"))
            .expect("moves")
            .expect("the pickup is empty");
        assert_eq!(out.time_sig, sig("4/4"));
        assert!(out.time_changes.is_empty());
        let at: Vec<String> = out.tracks[0]
            .notes
            .iter()
            .map(|n| n.at.to_string())
            .collect();
        assert_eq!(at, ["1.1.0", "1.2.0", "2.1.0"]);
    }

    #[test]
    fn a_pickup_with_a_note_in_it_stays() {
        let part = with_pickup(&[0, 480]);
        assert!(
            without_empty_lead_in(&part, 480, sig("4/4"))
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn a_part_with_no_lead_in_is_left_alone() {
        let part = with_pickup(&[0]);
        assert!(
            without_empty_lead_in(&part, 0, sig("4/4"))
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn a_failed_write_removes_what_was_written() {
        let dir = std::env::temp_dir().join(format!("duet-capture-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let first = dir.join("take.notes");
        let second = dir.join("missing").join("take.abc");
        let err = write_all(&[(&first, "notes\n"), (&second, "abc\n")]).unwrap_err();
        assert!(!first.exists(), "the .notes was left behind");
        let message = format!("{err:#}");
        assert!(
            message.contains("removed") && message.contains("take.notes"),
            "{message}"
        );
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn comments_go_after_the_first_line() {
        let lines = ["tempo: estimated 92 bpm".to_owned()];
        assert_eq!(
            commented("X:1\nT:x\n", "%", &lines),
            "X:1\n% tempo: estimated 92 bpm\nT:x\n"
        );
    }
}
