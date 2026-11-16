//! `duet capture`: a recorded take in; the exact performance (`.duet`, a song document), a
//! quantized score (`.abc` for a melody, or `.grid.duet`, a drum-grid document, for a rhythm)
//! and the analysis (`.capture.duet`, a `capture-analysis` document) out. Every document is
//! written through duet-schemas.
//!
//! Every estimate is stated ([`statement`]) on standard error, in each document's envelope
//! `statements`, and as a `%` comment in an `.abc`. Every refusal comes before anything is written, and nothing is
//! overwritten. Once the take is analysed, the performance (`.duet` and `.capture.duet`) is
//! written whatever becomes of the score, which is written, or reported as failed, after it.
//!
//! A melody take also writes its every pitch frame, as a `performance` stream,
//! `STEM.performance.duet.jsonl`, through duet-schemas ([`PerformanceFile`]). Its lines are
//! written as the frames settle; it is kept once the `.duet` and `.capture.duet` are written,
//! and removed again when the capture fails before then.
//!
//! `duet capture compare` ([`compare`]) reads a melody take's recording and those files back,
//! and plots and plays its reading against its performance.

mod compare;
mod statement;

use std::fs;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use duet_abc::{Dynamics, WriteOptions, off_level_velocities};
use duet_capture::{
    A440_CENTS, ANALYSIS_RATE, Error, Event, FileSource, Given, Hits, Kind, LaneMode, Layer1, Mono,
    NoteModel, Performance, PerformanceFrames, PerformanceHead, Prepare, Quantized, ReadNote,
    Source, Stage, a4_cents_of_hz, assemble, estimate, merge_collisions, quantize_apart,
    quantize_offsets, read,
};
use duet_core::{Control, Grid, Key, Position, Song, TimeMap, TimeSig};
use duet_schemas::{Document, Stream};

use crate::config::Config;
use crate::drums::{ReadOptions, grid_of_song, refuse_overwrite};
use crate::io::{tool, with_end};

#[derive(Subcommand)]
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
}

/// What both kinds of take are captured with. Anything about the timing left out is
/// estimated from the take and stated.
#[derive(Args)]
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
    /// What chooses a melody's notes from its performance: `hmm`, the note HMM (the default),
    /// or `rules`, the segmentation rules, stated when chosen. The reader is written in
    /// STEM.capture.duet either way.
    #[arg(long, value_enum)]
    reader: Option<ReaderArg>,
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
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReaderArg {
    Rules,
    Hmm,
}

/// Which kind of take is captured.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Take {
    Melody,
    Rhythm,
}

pub fn run(command: CaptureCommand, config: &Config) -> Result<()> {
    let (take, args) = match command {
        CaptureCommand::Melody(args) => (Take::Melody, args),
        CaptureCommand::Rhythm(args) => (Take::Rhythm, args),
        CaptureCommand::Compare(args) => return compare::run(&args),
    };
    let key = flags(take, &args)?;
    let grid = match args.grid {
        Some(grid) => grid,
        None => DEFAULT_GRID.parse()?,
    };
    // Refused before anything is read, like every refusal: a score on this grid could not be
    // quantized at all.
    grid.step_ticks(Song::default().ticks_per_quarter)
        .with_context(|| format!("--grid {grid}"))?;
    let outputs = Outputs::new(&args.input, args.output.as_deref(), take)?;

    // Every failure from here on is the take's, so it names the take. FileSource's own errors
    // name it already.
    let named = || args.input.display().to_string();
    let (mut events, kept) = analyse(&args.input, take, args.lane.is_some(), &outputs)?;
    // A melody's notes are read from its performance, on its own tuning.
    let (performance, reading) = match kept {
        Some((file, frames)) => {
            let given = args.tuning.map(|t| t.a4_cents);
            let reading = frames.map(|frames| match args.reader.unwrap_or(ReaderArg::Hmm) {
                ReaderArg::Rules => read(&frames, &events, given),
                ReaderArg::Hmm => NoteModel::default().read(&frames, &events, given),
            });
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

    // The score's grid is settled before the analysis is written, since the analysis gives
    // each note's quantize offset in its steps. A failure here is the score's, reported once
    // the performance is written.
    let ms_per_quarter = 1000.0 * timing.beat_period_frames / f64::from(ANALYSIS_RATE);
    let quantized = quantize_apart(&captured.song, grid, &kind).and_then(|q| {
        let offsets = quantize_offsets(&captured.song, &q.song, q.grid, ms_per_quarter)?;
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
        said.push(statement::notes_read(reading));
    }
    said.extend(statement::key(captured.analysis.key.as_ref()));
    captured.analysis.reading = reading;
    let lead_in = statement::lead_in(
        captured.lead_in,
        captured.downbeat_tick,
        meter,
        captured.song.ticks_per_quarter,
    );
    let song_said = [said.clone(), vec![lead_in.clone()]].concat();
    let mut song = Document::new(tool(), with_end(&captured.song, config.end)?);
    song.statements.clone_from(&song_said);
    let song = song.write()?;
    let analysis = Document::new(tool(), captured.analysis).write()?;

    // The performance is written whatever becomes of the score.
    for line in &song_said {
        eprintln!("{line}");
    }
    if let Some(advice) = advice {
        eprintln!("{advice}");
    }
    write_all(&[(&outputs.song, &song), (&outputs.analysis, &analysis)])?;
    if let Some(performance) = performance {
        performance.keep();
    }

    let wrote = || {
        format!(
            "{}: {} not written; {} and {} hold the performance",
            named(),
            outputs.score.display(),
            outputs.song.display(),
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
    write_new(&outputs.score, &scored.text).with_context(wrote)
}

/// The grid a score is quantized to when no `--grid` is given.
const DEFAULT_GRID: &str = "1/16";

/// What a take's score is made from, besides the quantized song.
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
        let trimmed = without_empty_lead_in(&quantized.song, self.downbeat_tick, self.meter)?;
        if trimmed.is_some() {
            stated.push(statement::LEAD_IN_REMOVED.to_owned());
        }
        let mut score = trimmed.unwrap_or(quantized.song);
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
                let grid = grid_of_song(&score, &options, self.config)?;
                let mut doc = Document::new(tool(), grid);
                doc.statements = score_said;
                doc.write()?
            }
        };
        Ok(Scored { text, stated })
    }
}

/// Refuses a flag the take cannot use, before anything is read, and reads the key.
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
    /// The performance as a song document.
    song: PathBuf,
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
            song: named(".duet"),
            score: named(match take {
                Take::Melody => ".abc",
                Take::Rhythm => ".grid.duet",
            }),
            analysis: named(".capture.duet"),
            performance: (take == Take::Melody).then(|| named(".performance.duet.jsonl")),
        };
        let all: Vec<&PathBuf> = [&outputs.song, &outputs.score, &outputs.analysis]
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
                self.keep(kept, events)?;
                match self {
                    Self::Melody { file, .. } => file.flush(),
                    Self::Rhythm(_) => Ok(()),
                }
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
/// the end of the stream. For a melody, the events are its onsets, and its performance file,
/// written in full but not yet kept, and its frames come with them.
fn analyse(
    input: &Path,
    take: Take,
    asked: bool,
    outputs: &Outputs,
) -> Result<(Vec<Event>, Option<Kept>)> {
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
    let (mut mono, mut events) = (Vec::new(), Vec::new());
    // The source's errors name the file; the stages' are named here.
    let named = || input.display().to_string();
    while let Some(chunk) = source.next_chunk()? {
        prepare.push(chunk, &mut mono).with_context(named)?;
        for m in mono.drain(..) {
            stage.push(m, &mut events).with_context(named)?;
        }
    }
    prepare.finish(&mut mono).with_context(named)?;
    for m in mono.drain(..) {
        stage.push(m, &mut events).with_context(named)?;
    }
    stage.finish(&mut events).with_context(named)?;
    let kept = match stage {
        Analyser::Melody { file, frames, .. } => Some((file, frames)),
        Analyser::Rhythm(_) => None,
    };
    Ok((events, kept))
}

/// A melody's performance file, written but not yet kept, and its frames: none when the take
/// held no audio at all.
type Kept = (PerformanceFile, Option<PerformanceFrames>);

/// A melody's `performance` stream, written through duet-schemas a line at a time as its frames
/// settle: the envelope and head first, then one block of frames a line. It is created new, and
/// removed again when dropped before [`keep`](PerformanceFile::keep), so a capture that fails
/// leaves it behind no more than its other outputs.
struct PerformanceFile {
    path: PathBuf,
    file: Option<BufWriter<fs::File>>,
    headed: bool,
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
            headed: false,
        })
    }

    /// Writes the head, the first time it comes, and every block of frames.
    fn write(&mut self, kept: Vec<Layer1>) -> Result<()> {
        for layer in kept {
            let line = match layer {
                Layer1::Head(head) if !self.headed => {
                    self.headed = true;
                    Stream::<PerformanceHead>::new(tool(), head).head_line()?
                }
                Layer1::Frames(block) if self.headed => {
                    Stream::<PerformanceHead>::record_line(&block)?
                }
                Layer1::Frames(_) => bail!("a performance's frames came before its head"),
                _ => continue,
            };
            self.out()?
                .write_all(line.as_bytes())
                .with_context(|| self.path.display().to_string())?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.out()?
            .flush()
            .with_context(|| self.path.display().to_string())
    }

    fn out(&mut self) -> Result<&mut BufWriter<fs::File>> {
        self.file
            .as_mut()
            .with_context(|| format!("{} is already closed", self.path.display()))
    }

    /// Keeps the file: the capture has written its performance.
    fn keep(mut self) {
        self.file = None;
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
/// song starts in `meter`, and the change to it at bar 2 is dropped. `None` when the score
/// has no lead-in, or a note or control still lies in it.
///
/// The song is one `assemble` built, so its only meter change is the one after a pickup, and
/// it has no tempo or key changes and names no end.
fn without_empty_lead_in(score: &Song, downbeat_tick: u64, meter: TimeSig) -> Result<Option<Song>> {
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

    /// A song with a one-beat pickup before 4/4 and notes at `ticks`.
    fn with_pickup(ticks: &[u64]) -> Song {
        let mut song = Song::default();
        song.time_sig = sig("1/4");
        song.time_changes.push(TimeChange {
            at: Position::bar(2),
            time_sig: sig("4/4"),
        });
        let map = song.time_map().expect("map");
        let mut track = Track::new("melody", 1);
        for &t in ticks {
            track.notes.push(Note::new(
                map.position(t),
                Pitch::new(60).expect("C4"),
                1.0,
                80,
            ));
        }
        song.tracks.push(track);
        song
    }

    #[test]
    fn an_empty_pickup_is_taken_out() {
        let song = with_pickup(&[480, 960, 2400]);
        let out = without_empty_lead_in(&song, 480, sig("4/4"))
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
        let song = with_pickup(&[0, 480]);
        assert!(
            without_empty_lead_in(&song, 480, sig("4/4"))
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn a_song_with_no_lead_in_is_left_alone() {
        let song = with_pickup(&[0]);
        assert!(
            without_empty_lead_in(&song, 0, sig("4/4"))
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
