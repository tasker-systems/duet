//! `duet capture review`: a take reviewed moment by moment, on one page.
//!
//! It reads what `duet capture melody` wrote at a stem, any other readings of the same take, the
//! take's audio, and a moments file, and writes a new directory (`STEM.review` unless `--dir`
//! names one), refused if it exists:
//!
//! - **`index.html`:** one self-contained page. For each moment it shows every reading side by
//!   side: the notes each reads in the window, `compare`'s plot of it inline (so an excursion's
//!   tooltip shows on hover), and players for its five renderings. A box takes the reviewer's
//!   verdict, and one button copies every verdict as text. The page also explains itself: the
//!   plot's marks, what each rendering plays, how a short gap plays, and what an excursion is.
//! - **`clips/`:** the audio the players play, each `compare`'s WAV of the moment: the take on
//!   the left, the reading's sine on the right. For each moment and reading there are `melody`
//!   (gaps bridged), `steps`, `bends`, `legato` and `gapped`. The whole take has its `melody`
//!   only.
//!
//! The plots and clips are `compare`'s (`super::compare::Window`), so a moment never draws or
//! plays differently here than there.
//!
//! **The moments file** is plain text, one moment per line: `FROM TO LABEL`, in seconds from the
//! start of the take, the label free text to the end of the line. A line starting with `#` is a
//! comment, and a blank line is skipped. A moment that does not start before it ends, or ends
//! past the take, is refused before anything is written.
//!
//! **Licence.** A clip is a copy of part of the recording. When a licence file sits beside the
//! take (`TAKE.license`, as the corpus keeps them), the command states it, and the page carries
//! it. Otherwise the command says so, and to keep the sheet and its clips out of git.
//!
//! **Where a verdict becomes a choice.** Today a verdict is text the reviewer copies out. Each box
//! carries its moment (`data-moment`, `data-from`, `data-to`, `data-label`) and the readings it
//! is over (`data-readings`), and the page's `verdicts()` gathers them. When the design of
//! per-moment choices over a take's readings is ruled, that is where a verdict becomes a choice
//! entry. No format for it is made here.
//!
//! **Where events would be emitted.** By the workbench's direction, duet's acts are events in a
//! session record. Three acts here would be: the sheet written (this command), a clip played and
//! a verdict given (both on the page, which cannot write a record yet). None is emitted today.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use duet_capture::{Analysis, Event, PerformanceFrames, ReadNote, Reading};

use super::compare::{
    LENGTH_SLACK_SECS, Window, analysis, excursions, performance, reading_of, svg, take, wav,
    with_suffix, write_bytes_new,
};
use schemars::JsonSchema;

/// The renderings each moment's clips are made in, with the name each is given on the page and
/// in a clip's file name.
const RENDERINGS: [(&str, wav::Gaps, wav::Render); 5] = [
    ("melody", wav::Gaps::Bridged, wav::Render::Melody),
    ("steps", wav::Gaps::Bridged, wav::Render::Steps),
    ("bends", wav::Gaps::Bridged, wav::Render::Bends),
    ("legato", wav::Gaps::Legato, wav::Render::Melody),
    ("gapped", wav::Gaps::Gapped, wav::Render::Melody),
];

/// How far apart, in seconds, two readings' note boundaries may be and still count as the same
/// notes, for the page's note that a moment's melody clips sound alike.
const SAME_NOTE_SECS: f64 = 0.03;

/// What `duet capture review` is given.
#[derive(Debug, Args, JsonSchema)]
pub struct ReviewArgs {
    /// The take's recording, as `duet capture melody` read it: the clips' left channel. In a
    /// project, a take's name too.
    input: PathBuf,
    /// The stem the take was captured to (`duet capture melody -o`). Without it, the input's
    /// name without its extension, beside it; for a project's take, its one reading in
    /// takes/TAKE/.
    #[arg(short, long, value_name = "STEM")]
    output: Option<PathBuf>,
    /// The moments to review: one per line, FROM TO LABEL, in seconds from the start of the
    /// take; a line starting with # is a comment.
    #[arg(long, value_name = "FILE")]
    moments: PathBuf,
    /// Another analysis of the same take, shown beside the stem's: NAME=ANALYSIS, such as
    /// rules=take-rules.capture.duet. Repeat it for more.
    #[arg(long = "reading", value_name = "NAME=ANALYSIS")]
    readings: Vec<String>,
    /// The directory the sheet is written to, new: index.html and clips/. Without it,
    /// STEM.review; for a project's take with no -o, takes/TAKE/review.
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

/// A moment of the take: its window in seconds, its label, and the line it came from.
#[derive(Debug, Clone, PartialEq)]
struct Moment {
    from: f64,
    to: f64,
    label: String,
    line: usize,
}

/// A reading the sheet shows: its name on the page, the analysis it is from, and that analysis.
struct Column {
    name: String,
    path: PathBuf,
    analysis: Analysis,
}

pub fn run(ctx: &crate::Context, args: ReviewArgs) -> Result<()> {
    // In a project, the take is given by name or found by its digest, its one reading in
    // takes/<take>/ is reviewed, and the sheet goes to takes/<take>/review/ (the project
    // design's §5).
    let input = crate::take::input(ctx, &args.input, args.output.is_none())?;
    let (output, dir) = match (&args.output, &input.take) {
        (None, Some(take)) => (
            Some(take.reading_stem()?),
            args.dir.clone().or_else(|| Some(take.dir().join("review"))),
        ),
        _ => (args.output.clone(), args.dir.clone()),
    };
    let args = &ReviewArgs {
        input: input.file,
        output,
        dir,
        ..args
    };
    let stem = args
        .output
        .clone()
        .unwrap_or_else(|| args.input.with_extension(""));
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| with_suffix(&stem, ".review"));
    // Refused before anything is read, as every capture command refuses: nothing is overwritten.
    if dir.exists() {
        bail!(
            "{} exists; review writes a new directory and never overwrites one, so move it or \
             name another with --dir",
            dir.display()
        );
    }
    let others = args
        .readings
        .iter()
        .map(|r| named_reading(r))
        .collect::<Result<Vec<_>>>()?;
    let moments_text = fs::read_to_string(&args.moments)
        .with_context(|| format!("{}: the moments file", args.moments.display()))?;
    let moments = parse_moments(&moments_text)
        .with_context(|| format!("{}: the moments file", args.moments.display()))?;
    if moments.is_empty() {
        bail!("{} names no moment", args.moments.display());
    }

    let performance_path = with_suffix(&stem, ".performance.duet.jsonl");
    let analysis_path = with_suffix(&stem, ".capture.duet");
    let frames = performance(&performance_path)?;
    let own = analysis(&analysis_path)?;
    let own_name = reader_name(reading_of(&own, &analysis_path)?);
    let mut columns = vec![Column {
        name: own_name,
        path: analysis_path.clone(),
        analysis: own,
    }];
    for (name, path) in others {
        let analysis = analysis(&path)?;
        reading_of(&analysis, &path)?;
        columns.push(Column {
            name,
            path,
            analysis,
        });
    }
    let mut names = BTreeSet::new();
    for column in &columns {
        if !names.insert(column.name.clone()) {
            bail!(
                "two readings are named {}: name each --reading apart from the stem's own, \
                 which is named after its reader",
                column.name
            );
        }
        if column.analysis.rate != frames.rate {
            bail!(
                "{} counts frames at {} Hz and {} at {} Hz: they are not of one performance",
                column.path.display(),
                column.analysis.rate,
                performance_path.display(),
                frames.rate
            );
        }
    }

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
    for (i, m) in moments.iter().enumerate() {
        if m.to > take_secs {
            bail!(
                "{}, moment {} (line {}): it ends at {} s, past the take's end at {take_secs:.2} \
                 s",
                args.moments.display(),
                i + 1,
                m.line,
                m.to
            );
        }
    }
    let licence = licence_of(&args.input);

    fs::create_dir_all(
        dir.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .with_context(|| dir.display().to_string())?;
    fs::create_dir(&dir).with_context(|| dir.display().to_string())?;
    let written = write_sheet(
        &dir,
        args,
        &frames,
        &take,
        &columns,
        &moments,
        licence.as_deref(),
    );
    match written {
        Ok(clips) => {
            match &licence {
                Some(text) => eprintln!(
                    "licence: {} (from {}); every clip is a copy of part of the recording, so \
                     credit it as that licence asks",
                    licence_summary(text, &args.input),
                    licence_path(&args.input).display()
                ),
                None => eprintln!(
                    "licence: no {} beside the take; the clips are copies of the recording, so \
                     keep {} out of git, as fixtures/ is",
                    licence_path(&args.input).display(),
                    dir.display()
                ),
            }
            println!(
                "wrote {}: {} moments, {} readings ({}), {clips} clips",
                dir.join("index.html").display(),
                moments.len(),
                columns.len(),
                columns
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            Err(e.context(format!(
                "review wrote nothing: removed {} again",
                dir.display()
            )))
        }
    }
}

/// `NAME=ANALYSIS`, as `--reading` takes it. A name is letters, digits, `-` and `_`, so it can
/// name a clip's file.
pub(super) fn named_reading(arg: &str) -> Result<(String, PathBuf)> {
    let Some((name, path)) = arg.split_once('=') else {
        bail!("--reading {arg}: give it as NAME=ANALYSIS, such as rules=take-rules.capture.duet");
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("--reading {arg}: a name is letters, digits, - and _ only");
    }
    if path.is_empty() {
        bail!("--reading {arg}: name the analysis after the =");
    }
    Ok((name.to_owned(), PathBuf::from(path)))
}

/// The stem's own reading's name on the page: its reader's, as the document names it.
pub(super) fn reader_name(reading: &Reading) -> String {
    reading
        .reader
        .and_then(|r| serde_json::to_value(r.name).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "reading".to_owned())
}

/// The moments in `text`, each checked to start at 0 or later and before it ends.
fn parse_moments(text: &str) -> Result<Vec<Moment>> {
    let mut moments = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.splitn(3, char::is_whitespace);
        let (Some(from), Some(rest)) = (parts.next(), parts.next()) else {
            bail!("line {line}: a moment is FROM TO LABEL, in seconds");
        };
        let label = parts.next().unwrap_or("").trim().to_owned();
        let number = |s: &str, what: &str| -> Result<f64> {
            s.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .with_context(|| format!("line {line}: {what} {s:?} is not a number of seconds"))
        };
        let from = number(from, "FROM")?;
        let to = number(rest, "TO")?;
        if from < 0.0 {
            bail!("line {line}: the moment starts at {from} s, before the take does");
        }
        if to <= from {
            bail!("line {line}: the moment ends at {to} s, not after it starts at {from} s");
        }
        moments.push(Moment {
            from,
            to,
            label,
            line,
        });
    }
    Ok(moments)
}

/// Where a take's licence notice sits: `TAKE.license`, as the corpus keeps them.
fn licence_path(take: &Path) -> PathBuf {
    with_suffix(take, ".license")
}

/// The take's licence notice, when one sits beside it.
fn licence_of(take: &Path) -> Option<String> {
    fs::read_to_string(licence_path(take))
        .ok()
        .filter(|t| !t.trim().is_empty())
}

/// A licence notice in brief: its first two paragraphs after a first line naming the file (as
/// the corpus's notices open), each joined into one line. For the corpus that is the copyright
/// line and the sentence naming the licence.
fn licence_summary(text: &str, take: &Path) -> String {
    let name = take.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut lines = text.lines().map(str::trim).peekable();
    if lines.peek().copied() == name.as_deref() {
        lines.next();
    }
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in lines {
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            if paragraphs.len() == 2 {
                break;
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() && paragraphs.len() < 2 {
        paragraphs.push(current);
    }
    paragraphs.join(" ")
}

/// Writes the clips and the page into `dir`, which is new; the number of clips written.
fn write_sheet(
    dir: &Path,
    args: &ReviewArgs,
    frames: &PerformanceFrames,
    take: &[f32],
    columns: &[Column],
    moments: &[Moment],
    licence: Option<&str>,
) -> Result<usize> {
    let clips = dir.join("clips");
    fs::create_dir(&clips).with_context(|| clips.display().to_string())?;
    let rate = f64::from(frames.rate);
    let readings: Vec<&Reading> = columns
        .iter()
        .map(|c| c.analysis.reading.as_ref().context("a reading"))
        .collect::<Result<_>>()?;
    let own = readings[0];
    let window = |i: usize, from: u64, to: u64| -> (Window<'_>, Vec<u64>) {
        let onsets = columns[i]
            .analysis
            .events
            .iter()
            .filter_map(|e| match *e {
                Event::Onset { frame, .. } => Some(frame),
                _ => None,
            })
            .collect();
        (
            Window {
                input: &args.input,
                compared: &columns[i].path,
                frames,
                take,
                reading: readings[i],
                beside: (i > 0).then_some(own),
                onsets: &[],
                from,
                to,
            },
            onsets,
        )
    };
    let mut count = 0;
    let mut write_clip = |name: &str, bytes: Vec<u8>| -> Result<()> {
        write_bytes_new(&clips.join(name), &bytes)?;
        count += 1;
        Ok(())
    };

    // The whole take, its melody only.
    let whole_to = take.len() as u64;
    let mut whole = Vec::new();
    for (i, column) in columns.iter().enumerate() {
        let (mut w, onsets) = window(i, 0, whole_to);
        w.onsets = &onsets;
        let clip = format!("take-{}.wav", column.name);
        write_clip(&clip, w.audio(wav::Gaps::Bridged, wav::Render::Melody))?;
        whole.push(Cell {
            name: &column.name,
            notes: notes_line(readings[i], 0, whole_to, rate),
            plot: w.plot(wav::Gaps::Bridged, wav::Render::Melody),
            clips: vec![("melody", format!("clips/{clip}"))],
        });
    }

    let mut sections = Vec::new();
    for (m_i, moment) in moments.iter().enumerate() {
        let n = m_i + 1;
        let from = (moment.from * rate).round() as u64;
        let to = ((moment.to * rate).round() as u64).min(whole_to);
        let mut cells = Vec::new();
        for (i, column) in columns.iter().enumerate() {
            let (mut w, onsets) = window(i, from, to);
            w.onsets = &onsets;
            let mut players = Vec::new();
            for (label, gaps, render) in RENDERINGS {
                let clip = format!("m{n}-{}-{label}.wav", column.name);
                write_clip(&clip, w.audio(gaps, render))?;
                players.push((label, format!("clips/{clip}")));
            }
            cells.push(Cell {
                name: &column.name,
                notes: notes_line(readings[i], from, to, rate),
                plot: w.plot(wav::Gaps::Bridged, wav::Render::Melody),
                clips: players,
            });
        }
        let alike = readings.len() > 1 && same_notes(&readings, from, to, rate);
        sections.push(Section {
            n,
            moment,
            cells,
            alike,
        });
    }

    let page = page(args, columns, &whole, &sections, licence);
    super::write_new(&dir.join("index.html"), &page)?;
    Ok(count)
}

/// One reading's part of a moment on the page.
struct Cell<'a> {
    name: &'a str,
    notes: String,
    plot: String,
    clips: Vec<(&'static str, String)>,
}

/// One moment on the page.
struct Section<'a> {
    n: usize,
    moment: &'a Moment,
    cells: Vec<Cell<'a>>,
    /// Whether every reading reads the same notes here, so their melody clips sound alike.
    alike: bool,
}

/// The notes a reading reads in `from..to`, as the page lists them: key, times, `~` when
/// between keys, and its excursions in the window.
fn notes_line(reading: &Reading, from: u64, to: u64, rate: f64) -> String {
    let notes: Vec<String> = reading
        .notes
        .iter()
        .filter(|n| svg::shown(n, from, to))
        .map(|n| note_text(n, rate))
        .collect();
    if notes.is_empty() {
        "no note".to_owned()
    } else {
        notes.join(" · ")
    }
}

fn note_text(note: &ReadNote, rate: f64) -> String {
    let mut text = format!(
        "{}{} {:.2}–{:.2} s",
        svg::key_name(note.key),
        if note.between_keys { "~" } else { "" },
        note.start as f64 / rate,
        note.end as f64 / rate
    );
    let ex = excursions(note, rate);
    if ex != "-" {
        text.push_str(&format!(" (excursions: {ex})"));
    }
    text
}

/// Whether every reading reads the same notes in `from..to`: the same keys, each boundary
/// within [`SAME_NOTE_SECS`] once cut to the window, since only the window is heard.
fn same_notes(readings: &[&Reading], from: u64, to: u64, rate: f64) -> bool {
    let shown = |r: &Reading| -> Vec<(u8, u64, u64)> {
        r.notes
            .iter()
            .filter(|n| svg::shown(n, from, to))
            .map(|n| (n.key, n.start.max(from), n.end.min(to)))
            .collect()
    };
    let first = shown(readings[0]);
    let near = |a: u64, b: u64| (a.abs_diff(b) as f64) / rate <= SAME_NOTE_SECS;
    readings[1..].iter().all(|r| {
        let other = shown(r);
        other.len() == first.len()
            && other
                .iter()
                .zip(&first)
                .all(|(a, b)| a.0 == b.0 && near(a.1, b.1) && near(a.2, b.2))
    })
}

/// Text made safe inside HTML, in an element or a quoted attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The page.
fn page(
    args: &ReviewArgs,
    columns: &[Column],
    whole: &[Cell],
    sections: &[Section],
    licence: Option<&str>,
) -> String {
    let take_name = args.input.file_name().map_or_else(
        || args.input.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let reading_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let readings_attr = escape(&reading_names.join(" "));
    let licence_html = match licence {
        Some(text) => format!(
            "<p class=\"licence\"><b>Licence.</b> Every clip here is a copy of part of the \
             recording, so credit it as its licence asks. {}</p><details \
             class=\"notice\"><summary>The licence notice in full</summary><pre>{}</pre></details>",
            escape(&licence_summary(text, &args.input)),
            escape(text.trim())
        ),
        None => "<p class=\"licence warn\"><b>No licence file sits beside this take.</b> Every \
                 clip here is a copy of part of the recording: keep this sheet and its clips out \
                 of git.</p>"
            .to_owned(),
    };
    let nav: String = sections
        .iter()
        .map(|s| format!("<a href=\"#m{}\">{} · {:.2} s</a>", s.n, s.n, s.moment.from))
        .collect();
    let mut body = String::new();
    body.push_str(&format!(
        "<details class=\"whole\"><summary>The whole take, each reading's melody</summary>\
         <div class=\"cols\">{}</div></details>\n",
        whole.iter().map(cell_html).collect::<String>()
    ));
    for s in sections {
        let alike = if s.alike {
            "<p class=\"alike\">Every reading reads the same notes here, so their melody clips \
             sound alike: listen to steps or bends for what differs.</p>"
        } else {
            ""
        };
        body.push_str(&format!(
            "<section id=\"m{n}\">\n<h2><span class=\"n\">{n}</span> {label} <span \
             class=\"t\">{from:.2}–{to:.2} s</span></h2>\n{alike}<div class=\"cols\">{cells}</div>\n\
             <label>Your verdict<textarea data-moment=\"{n}\" data-from=\"{from}\" \
             data-to=\"{to}\" data-label=\"{label}\" data-readings=\"{readings_attr}\" \
             placeholder=\"Is each reading right by ear? Each step: a sung ornament, a note, or \
             nothing?\"></textarea></label>\n</section>\n",
            n = s.n,
            label = escape(&s.moment.label),
            from = s.moment.from,
            to = s.moment.to,
            cells = s.cells.iter().map(cell_html).collect::<String>(),
        ));
    }
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title} review</title><style>{STYLE}</style></head><body>\n\
         <header><div class=\"tools\"><label class=\"repeat\"><input type=\"checkbox\" \
         id=\"repeat\"> Repeat each clip</label><button id=\"copy\">Copy my verdicts</button></div>\n\
         <h1>{title}: {count} moments</h1>\n\
         <p>The take on the left, each reading's sine on the right. Each moment shows every \
         reading side by side: the notes it reads, its plot, and its clips. Readings: {readings}.\
         </p>\n{licence_html}\n{GUIDE}\n<nav>{nav}</nav></header>\n{body}\
         <script>{SCRIPT}</script>\n</body></html>\n",
        title = escape(&take_name),
        count = sections.len(),
        readings = escape(&reading_names.join(", ")),
    )
}

fn cell_html(cell: &Cell) -> String {
    let players: String = cell
        .clips
        .iter()
        .take(3)
        .map(|(label, src)| player(label, src))
        .collect();
    let more: String = cell
        .clips
        .iter()
        .skip(3)
        .map(|(label, src)| player(label, src))
        .collect();
    let more = if more.is_empty() {
        String::new()
    } else {
        format!("<details><summary>legato · gapped</summary>{more}</details>")
    };
    format!(
        "<div class=\"col\"><h3>{name}</h3><p class=\"reads\">{notes}</p>{plot}{players}{more}</div>",
        name = escape(cell.name),
        notes = escape(&cell.notes),
        plot = inline_svg(&cell.plot),
    )
}

fn player(label: &str, src: &str) -> String {
    format!(
        "<div class=\"pl\"><span>{label}</span><audio controls preload=\"none\" \
         src=\"{src}\"></audio></div>",
        src = escape(src)
    )
}

/// `compare`'s plot, inline, sized by its viewBox to its column.
fn inline_svg(svg: &str) -> String {
    match svg.find("<svg ") {
        Some(at) => {
            let rest = &svg[at + 5..];
            // Drop the fixed width and height, keeping the viewBox.
            let mut attrs = rest.splitn(2, '>');
            let head = attrs.next().unwrap_or("");
            let tail = attrs.next().unwrap_or("");
            let head: String = head
                .split(' ')
                .filter(|a| !a.starts_with("width=") && !a.starts_with("height="))
                .collect::<Vec<_>>()
                .join(" ");
            format!("<svg class=\"plot\" {head}>{tail}")
        }
        None => String::new(),
    }
}

/// What the page explains about itself: the things a reviewer needed explained before they
/// could rule.
const GUIDE: &str = "<details class=\"guide\"><summary>How to read this sheet</summary>\
<p><b>The plot.</b> Blue line: the performance, the pitch heard frame by frame (broken where \
unvoiced). Green bars: the reading's notes at their keys on the take's tuning; orange bars sit \
between two keys. Pale red: the residual, between each bar and the pitch over its frames. Amber \
boxes: excursions, each held at its key plus its offset (hover one for its cents and length). \
Dotted grey: the melody's sine, as its pitch. Dashed purple, in every reading's plot but the \
first: the first reading's notes, beside it. Ticks under the plot: onsets. Grey lines: one per \
key, the C lines darker.</p>\
<p><b>The clips.</b> <i>melody</i>: the notes' keys, gliding through each note's slide in. \
<i>steps</i>: the keys, with each excursion held at its offset, so an ornament the reading keeps \
inside a note is heard. <i>bends</i>: inside each note, the voice's own pitch; between notes, as \
melody. So bends plays what was sung wherever a note covers it, whatever the reader made of it: \
it is the voice, trimmed to the reading's notes, not the reader's judgment. <i>legato</i>: the \
melody held through every gap. <i>gapped</i>: the melody silent in every gap.</p>\
<p><b>A short gap.</b> Between two notes less than 0.2 s apart, the sine holds the note before's \
key (except in gapped), so where a reader left a short stretch unread you hear a moment where \
the sine stops following the voice, not silence.</p>\
<p><b>An excursion</b> is a pitch held briefly inside a note while the score keeps one note: \
steps plays it, and the melody does not. The other choice is a note of its own, in the melody. \
Asking \"an ornament, a note, or nothing?\" is asking which of the two each held pitch is.</p>\
</details>";

const STYLE: &str = ":root{--bg:#fbfaf7;--fg:#1d1d1b;--muted:#6b6a65;--line:#e3e0d8;--accent:#b5541c;--card:#fff;--warn:#a4161a}\
@media (prefers-color-scheme: dark){:root{--bg:#161614;--fg:#ecebe6;--muted:#a09f98;--line:#33322e;--accent:#e58a52;--card:#1f1f1c;--warn:#f08080}}\
body{background:var(--bg);color:var(--fg);font:15px/1.45 -apple-system,system-ui,sans-serif;margin:0;padding:16px}\
header,section,details.whole{max-width:1200px;margin:0 auto 16px}\
h1{font-size:20px;margin:0 0 6px}header p{color:var(--muted);margin:4px 0}.licence.warn{color:var(--warn)}\
nav{display:flex;flex-wrap:wrap;gap:6px;margin:10px 0}\
nav a{border:1px solid var(--line);border-radius:6px;padding:2px 8px;color:var(--fg);text-decoration:none;font-variant-numeric:tabular-nums}\
section,details.whole{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px}\
details.guide{margin:8px 0;color:var(--fg)}details.guide p{color:var(--fg)}\
h2{font-size:16px;margin:0 0 6px}h2 .n{color:var(--accent);margin-right:4px}h2 .t{color:var(--muted);font-weight:400}\
.alike{color:var(--accent);margin:0 0 8px}\
.cols{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:14px}\
.col h3{margin:0 0 4px;font-size:13px;text-transform:uppercase;letter-spacing:.05em;color:var(--muted)}\
.reads{margin:0 0 6px;font-variant-numeric:tabular-nums}\
svg.plot{display:block;width:100%;height:auto;background:#fff;border:1px solid var(--line);border-radius:6px}\
.pl{display:flex;align-items:center;gap:8px;margin:4px 0}.pl span{width:56px;color:var(--muted);font-size:13px}\
audio{height:32px;flex:1;min-width:0}\
details summary{color:var(--muted);font-size:13px;cursor:pointer;margin-top:4px}\
label{display:block;margin-top:10px;font-size:13px;color:var(--muted)}\
.tools{float:right;display:flex;align-items:center;gap:12px}.tools label{margin:0}\
details.notice pre{white-space:pre-wrap;font:12px/1.4 ui-monospace,monospace;color:var(--muted)}\
textarea{width:100%;box-sizing:border-box;min-height:52px;margin-top:4px;font:inherit;color:var(--fg);background:var(--bg);border:1px solid var(--line);border-radius:6px;padding:6px}\
#copy{background:var(--accent);color:#fff;border:0;border-radius:6px;padding:6px 12px;font:inherit;cursor:pointer}";

/// The page's script: one clip plays at a time, repeat on demand, verdicts kept in this browser
/// for this sheet, and copied out as text.
const SCRIPT: &str = r#"
const key = "duet-review:" + location.pathname;
let saved = {}; try { saved = JSON.parse(localStorage.getItem(key) || "{}"); } catch (e) {}
document.querySelectorAll("textarea").forEach(t => {
  t.value = saved[t.dataset.moment] || "";
  t.addEventListener("input", () => {
    saved[t.dataset.moment] = t.value;
    try { localStorage.setItem(key, JSON.stringify(saved)); } catch (e) {}
  });
});
const audios = [...document.querySelectorAll("audio")];
audios.forEach(a => a.addEventListener("play", () => audios.forEach(o => { if (o !== a) o.pause(); })));
document.getElementById("repeat").addEventListener("change", e => audios.forEach(a => { a.loop = e.target.checked; }));
// Each verdict with its moment and the readings it is over. This is where a verdict would become
// a choice entry, once per-moment choices over a take's readings are designed; until then it is
// text the reviewer copies out.
function verdicts() {
  return [...document.querySelectorAll("textarea")]
    .filter(t => t.value.trim())
    .map(t => ({ moment: +t.dataset.moment, from: +t.dataset.from, to: +t.dataset.to,
                 label: t.dataset.label, readings: t.dataset.readings.split(" "), text: t.value.trim() }));
}
document.getElementById("copy").addEventListener("click", async () => {
  const text = verdicts().map(v => `${v.moment}. ${v.from}–${v.to} s ${v.label}: ${v.text}`).join("\n")
    || "(no verdicts yet)";
  try { await navigator.clipboard.writeText(text); document.getElementById("copy").textContent = "Copied"; }
  catch (e) { prompt("Copy these:", text); }
});
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_are_read_with_their_labels_and_comments_skipped() {
        let text = "# a comment\n\n1.20 1.85 turn on D#3: one note?\n2.05\t2.70 approach\n3 4\n";
        let moments = parse_moments(text).unwrap();
        assert_eq!(
            moments,
            vec![
                Moment {
                    from: 1.2,
                    to: 1.85,
                    label: "turn on D#3: one note?".into(),
                    line: 3
                },
                Moment {
                    from: 2.05,
                    to: 2.7,
                    label: "approach".into(),
                    line: 4
                },
                Moment {
                    from: 3.0,
                    to: 4.0,
                    label: String::new(),
                    line: 5
                },
            ]
        );
    }

    #[test]
    fn a_moment_that_does_not_start_before_it_ends_is_refused_with_its_line() {
        let err = parse_moments("# x\n2.0 1.0 backwards\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
        let err = parse_moments("-1 1 before\n").unwrap_err();
        assert!(err.to_string().contains("before the take"), "{err}");
        let err = parse_moments("1.0 soon later\n").unwrap_err();
        assert!(err.to_string().contains("TO \"soon\""), "{err}");
        let err = parse_moments("1.0\n").unwrap_err();
        assert!(err.to_string().contains("FROM TO LABEL"), "{err}");
    }

    #[test]
    fn a_reading_is_named_with_file_safe_characters() {
        assert_eq!(
            named_reading("rules=a b/take-rules.capture.duet").unwrap(),
            (
                "rules".to_owned(),
                PathBuf::from("a b/take-rules.capture.duet")
            )
        );
        assert!(named_reading("rules").is_err());
        assert!(named_reading("ru les=x").is_err());
        assert!(named_reading("rules=").is_err());
    }

    #[test]
    fn the_inline_plot_keeps_its_view_box_and_drops_its_size() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="672" height="738" viewBox="0 0 672 738"><g/></svg>"#;
        assert_eq!(
            inline_svg(svg),
            r#"<svg class="plot" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 672 738"><g/></svg>"#
        );
    }

    #[test]
    fn a_licence_is_stated_by_its_copyright_and_licence_lines() {
        let text = "take.flac\nCopyright 2026 Someone\n\nThis recording is licensed under\nCC BY 4.0.\n\nMore.\n";
        assert_eq!(
            licence_summary(text, Path::new("corpus/take.flac")),
            "Copyright 2026 Someone This recording is licensed under CC BY 4.0."
        );
    }

    #[test]
    fn text_is_escaped_for_an_attribute() {
        assert_eq!(
            escape(r#"a "b" <c> & d"#),
            "a &quot;b&quot; &lt;c&gt; &amp; d"
        );
    }
}
