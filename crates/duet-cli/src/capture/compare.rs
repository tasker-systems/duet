//! `duet capture compare`: a reading plotted and played against the performance it was read
//! from, so a listener can hear and see where it departs from the singing.
//!
//! It reads what `duet capture melody` wrote at a stem, and the take's audio, which neither
//! file names:
//!
//! - **`STEM.performance.duet.jsonl`:** the performance, every pitch frame;
//! - **`STEM.capture.duet`:** the analysis, whose `reading` (capture-analysis 1.1) holds the
//!   tuning and the notes, with each note's residual and the take's loss since 1.2, and whose events hold the onsets. `--reading` names another
//!   analysis of the same take, whose reading is compared instead; the stem's own is drawn
//!   beside it.
//!
//! It writes, each refused rather than overwritten, named after the analysis whose reading it
//! shows (`STEM.compare.svg` and `STEM.compare.wav`, or the other analysis's name with
//! `.compare.svg` and `.compare.wav` in place of `.capture.duet`):
//!
//! - **the plot** ([`svg`]): the performance's pitch in cents, the reading's notes as bars at
//!   `tuning + 100·key`, the residual shaded between them, between-keys notes marked, and the
//!   onsets as ticks;
//! - **the audio** ([`wav`]): the take on the left, as the analysis heard it (one channel, at
//!   the analysis rate, so it lines up with the frames sample for sample), and the reading on
//!   the right as one legato sine at the take's own tuning, gliding only through each note's
//!   slide in: its scoop where the residual measured one, else up to where its held pitch
//!   begins;
//! - **on standard output:** one line per note (its time, key, centre and whether it is between
//!   keys), with its residual's parts (held centre, drift, vibrato, scoop, fall and rest) when
//!   the reading has them, then the take's loss as its terms. A 1.1 reading has neither,
//!   and the output says so.
//!
//! `--from` and `--to` window all three to a stretch of the take, in seconds.
//!
//! **What it cannot check:** nothing in either file names the audio it was read from, so the
//! take is checked only by its length against the performance's.

mod svg;
mod wav;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use duet_capture::{
    Analysis, Event, FileSource, Origin, PerformanceFrames, PerformanceHead, Prepare, Reading,
    Source, Stage,
};
use duet_schemas::{Document, Stream};

use super::write_new;

/// The performance's frames may end up to this long after the take's last sample, in seconds:
/// a frame sits at the middle of its window, and the last window is padded.
const LENGTH_SLACK_SECS: f64 = 0.1;

/// What `duet capture compare` is given.
#[derive(Args)]
pub struct CompareArgs {
    /// The take's recording, as `duet capture melody` read it: the plot's and the audio's
    /// timeline, and the audio's left channel.
    input: PathBuf,
    /// The stem the take was captured to (`duet capture melody -o`): STEM.performance.duet.jsonl
    /// and STEM.capture.duet are read from it. Without it, the input's name without its
    /// extension, beside it.
    #[arg(short, long, value_name = "STEM")]
    output: Option<PathBuf>,
    /// Another analysis of the same take (a .capture.duet) whose reading is compared instead of
    /// the stem's, which is drawn beside it. The outputs are named after it.
    #[arg(long, value_name = "ANALYSIS")]
    reading: Option<PathBuf>,
    /// Where the window starts, in seconds from the start of the take.
    #[arg(long, value_name = "SECS")]
    from: Option<f64>,
    /// Where the window ends, in seconds from the start of the take.
    #[arg(long, value_name = "SECS")]
    to: Option<f64>,
}

pub fn run(args: &CompareArgs) -> Result<()> {
    let from_secs = args.from.unwrap_or(0.0);
    if !(from_secs.is_finite() && from_secs >= 0.0) {
        bail!("--from {from_secs}: the window starts at a number of seconds, 0 or more");
    }
    if let Some(to) = args.to
        && !(to.is_finite() && to > from_secs)
    {
        bail!("--to {to}: the window ends after it starts, at {from_secs} s");
    }

    let stem = args
        .output
        .clone()
        .unwrap_or_else(|| args.input.with_extension(""));
    let performance_path = with_suffix(&stem, ".performance.duet.jsonl");
    let analysis_path = with_suffix(&stem, ".capture.duet");
    let compared_path = args
        .reading
        .clone()
        .unwrap_or_else(|| analysis_path.clone());
    let out_stem = stem_of(&compared_path);
    let (svg_path, wav_path) = (
        with_suffix(&out_stem, ".compare.svg"),
        with_suffix(&out_stem, ".compare.wav"),
    );
    // Refused before anything is read, as capture refuses: nothing is overwritten.
    for path in [&svg_path, &wav_path] {
        if path.exists() {
            bail!(
                "{} exists; compare writes new files and never overwrites one, so move it",
                path.display()
            );
        }
    }

    let frames = performance(&performance_path)?;
    let compared = analysis(&compared_path)?;
    // With --reading, the stem's own analysis is read too, for its reading to draw beside.
    let own = match &args.reading {
        Some(_) => Some(analysis(&analysis_path)?),
        None => None,
    };
    for (analysis, path) in [
        (Some(&compared), &compared_path),
        (own.as_ref(), &analysis_path),
    ] {
        if let Some(analysis) = analysis
            && analysis.rate != frames.rate
        {
            bail!(
                "{} counts frames at {} Hz and {} at {} Hz: they are not of one performance",
                path.display(),
                analysis.rate,
                performance_path.display(),
                frames.rate
            );
        }
    }
    let reading = reading_of(&compared, &compared_path)?;
    let beside = own
        .as_ref()
        .map(|own| reading_of(own, &analysis_path))
        .transpose()?;
    let onsets: Vec<u64> = compared
        .events
        .iter()
        .filter_map(|e| match *e {
            Event::Onset { frame, .. } => Some(frame),
            _ => None,
        })
        .collect();

    let take = take(&args.input, frames.rate)?;
    let rate = f64::from(frames.rate);
    let take_secs = take.len() as f64 / rate;
    if let Some(last) = frames.len().checked_sub(1).map(|k| frames.frame(k))
        && last as f64 / rate > take_secs + LENGTH_SLACK_SECS
    {
        bail!(
            "{} is {take_secs:.2} s long, and {}'s frames run to {:.2} s: it is not the take \
             that performance was read from",
            args.input.display(),
            performance_path.display(),
            last as f64 / rate
        );
    }
    let from = (from_secs * rate).round() as u64;
    let to = args.to.map_or(take.len() as u64, |t| {
        ((t * rate).round() as u64).min(take.len() as u64)
    });
    if from >= to {
        bail!(
            "--from {from_secs}: {} is {take_secs:.2} s long, so the window holds nothing",
            args.input.display()
        );
    }

    let title = format!(
        "{}: {} on A4 = {:.1} Hz ({:+.0} cents{}), {} to {}",
        args.input.display(),
        compared_path.display(),
        reading.tuning.a4_hz(),
        reading.tuning.offset_cents(),
        if matches!(reading.tuning.a4, Origin::Given(_)) {
            ", given"
        } else {
            ", estimated"
        },
        seconds(from as f64 / rate),
        seconds(to as f64 / rate)
    );
    let plot = svg::render(&svg::Plot {
        title: &title,
        frames: &frames,
        from,
        to,
        reading,
        beside,
        onsets: &onsets,
    });
    let right = wav::sine(reading, frames.rate, from, to);
    let audio = wav::stereo(frames.rate, &take[from as usize..to as usize], &right);

    write_new(&svg_path, &plot)?;
    if let Err(e) = write_bytes_new(&wav_path, &audio) {
        let _ = fs::remove_file(&svg_path);
        return Err(e.context(format!(
            "compare wrote nothing: removed {} again",
            svg_path.display()
        )));
    }
    print!("{}", notes_table(&title, reading, from, to, rate));
    Ok(())
}

/// The lines on standard output: the title, then one line per note shown, with its residual's
/// parts when the reading measured them (capture-analysis 1.2), then the take's loss as its
/// terms.
fn notes_table(title: &str, reading: &Reading, from: u64, to: u64, rate: f64) -> String {
    let parts = reading.residual.is_some();
    let mut out = format!("# {title}\n#     time  key    centre");
    if parts {
        out.push_str(&format!(
            "  {:>7}  {:>8}   {:<13}  {:<13}  {:<13}  {:>6}",
            "held", "drift/s", "vibrato", "scoop", "fall", "rest"
        ));
    }
    out.push('\n');
    for note in reading.notes.iter().filter(|n| svg::shown(n, from, to)) {
        let mut line = format!(
            "{:>10}  {:<5} {:>+6.1}c",
            format!("{:.3} s", note.start as f64 / rate),
            svg::key_name(note.key),
            note.centre,
        );
        if let Some(r) = &note.residual {
            let transition = |t: Option<(f32, f32)>| {
                t.map_or_else(|| "-".to_owned(), |(c, ms)| format!("{c:+.0}c {ms:.0} ms"))
            };
            line.push_str(&format!(
                "  {:>+6.1}c  {:>+8.1}   {:<13}  {:<13}  {:<13}  {:>5.1}c",
                r.centre,
                r.drift.cents_per_sec,
                r.modulation.map_or_else(
                    || "-".to_owned(),
                    |m| format!("{:.1} Hz ±{:.0}c", m.rate_hz, m.depth_cents)
                ),
                transition(r.scoop.map(|t| (t.cents, t.ms))),
                transition(r.fall.map(|t| (t.cents, t.ms))),
                r.rest_cents,
            ));
        }
        if note.between_keys {
            line.push_str("  between keys");
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    match &reading.residual {
        Some(take) => out.push_str(&format!(
            "# loss: pitch {:.1}, voicing {:.1}, unread {:.1}, notes {:.1} ({} frames read, {} \
             unread)\n",
            take.loss.pitch,
            take.loss.voicing,
            take.loss.unread,
            take.loss.notes,
            take.read_frames,
            take.unread_frames
        )),
        None => out.push_str(
            "# no residual or loss: this reading holds neither (it was written before \
             capture-analysis 1.2)\n",
        ),
    }
    out
}

/// The performance stream's frames.
fn performance(path: &Path) -> Result<PerformanceFrames> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "{}: compare reads the performance duet capture melody wrote",
            path.display()
        )
    })?;
    let stream =
        Stream::<PerformanceHead>::read(&text).with_context(|| path.display().to_string())?;
    let mut frames = PerformanceFrames::new(&stream.head);
    for block in &stream.records {
        frames
            .push(block)
            .with_context(|| path.display().to_string())?;
    }
    Ok(frames)
}

/// A capture-analysis document's body.
fn analysis(path: &Path) -> Result<Analysis> {
    let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    Ok(Document::<Analysis>::read(&text)
        .with_context(|| path.display().to_string())?
        .body)
}

/// An analysis's reading, which a rhythm take's and a 1.0 document's lack.
fn reading_of<'a>(analysis: &'a Analysis, path: &Path) -> Result<&'a Reading> {
    analysis.reading.as_ref().with_context(|| {
        format!(
            "{} holds no reading: it is a rhythm take's, or was written before capture-analysis \
             1.1; capture the melody again with duet capture melody",
            path.display()
        )
    })
}

/// The take's audio as the analysis heard it: one channel at `rate`, from its first frame.
fn take(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let named = || path.display().to_string();
    let mut source = FileSource::open(path)?;
    let mut prepare = Prepare::new(rate);
    let mut mono = Vec::new();
    while let Some(chunk) = source.next_chunk()? {
        prepare.push(chunk, &mut mono).with_context(named)?;
    }
    prepare.finish(&mut mono).with_context(named)?;
    let mut samples = Vec::new();
    for m in mono {
        let at = usize::try_from(m.start_frame).context("a frame past the address space")?;
        if samples.len() < at {
            samples.resize(at, 0.0);
        }
        samples.truncate(at);
        samples.extend_from_slice(&m.samples);
    }
    Ok(samples)
}

/// `stem` with `suffix` added to its name.
fn with_suffix(stem: &Path, suffix: &str) -> PathBuf {
    let mut path = stem.as_os_str().to_owned();
    path.push(suffix);
    PathBuf::from(path)
}

/// An analysis's stem: its name without `.capture.duet`, or else without `.duet`.
fn stem_of(analysis: &Path) -> PathBuf {
    let name = analysis.as_os_str().to_string_lossy();
    for suffix in [".capture.duet", ".duet"] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return PathBuf::from(stem);
        }
    }
    analysis.to_path_buf()
}

/// Seconds as the title gives them.
fn seconds(t: f64) -> String {
    format!("{t:.2} s")
}

/// Writes bytes to a file that must not exist yet, removing it again if it cannot be filled.
fn write_bytes_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| path.display().to_string())?;
    if let Err(e) = file.write_all(bytes) {
        let _ = fs::remove_file(path);
        return Err(e).with_context(|| path.display().to_string());
    }
    Ok(())
}
