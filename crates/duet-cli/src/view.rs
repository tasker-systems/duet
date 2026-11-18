//! `duet view` and `duet apply`: a document's text views, rendered and read back.
//!
//! The text forms are views of a document, not files of their own: `duet view` renders one, and
//! `duet apply` reads an edited one back. A view is a working surface. A person or an agent
//! edits it until it plays as intended, then applies it.
//!
//! # The views
//!
//! | View | Of | Lossless? |
//! |---|---|---|
//! | `notes` | a part | yes: every field of part 1.0 |
//! | `grid` | a drum grid | yes, for velocities on its three levels |
//! | `grid` | one drum track of a part, at a step | no: that track's starts and velocities only |
//!
//! # What each view drops
//!
//! - **`notes` of a part: nothing in the body.** It shows the normalized body, and always states
//!   the end.
//! - **`grid` of a drum grid: nothing in the body**, as long as every velocity is 0 or one of the
//!   grid's three levels. One that is not is refused, naming its lane, bar and step, unless
//!   `--snap-levels` shows it snapped; `apply --onto` then keeps the exact velocity of every cell
//!   whose level the view leaves unchanged.
//! - **`grid` of a part's drum track: everything but that track's starts and velocities**: every
//!   other track, the drum track's note lengths, controls and program, and the exact velocity
//!   behind a level shown snapped. `apply --onto` keeps all of them.
//! - **Every view drops the envelope's `tool` and `from`,** and shows its `statements` as `# `
//!   comment lines under the header, read-only.
//! - **Comments added to a view are not read back.** Only the header line is parsed; a note left in
//!   a comment is lost on `apply`.
//!
//! # The header line
//!
//! A view's first line names it, so it reads back without a file name:
//!
//! ```text
//! # duet notes v1 of part 1.0 from 3f9a1c0d5e2b7a41
//! # duet grid v1 of drum-grid 1.0 from 9c04e1b2a7d35f60
//! # duet grid v1 of part 1.0 track "Drums" step 1/16 from 0b7e33c19a5d2f84
//! ```
//!
//! The view and its syntax version, the kind and schema version it renders, which part of the
//! document a partial view shows (`track`, `step`, and `bars` when they were asked for), and a
//! 64-bit FNV-1a digest of the source document's body as compact JSON. `apply --onto` refuses a
//! document whose digest differs, naming both, unless `--force`. The digest guards against
//! mistakes, not against anyone.
//!
//! A view naming a newer view version, or a schema version this duet does not read, is refused
//! by name. Today's `# notefmt v1` and `# notefmt drums v1` lines read as `notes v1 of part 1.0`
//! and `grid v1 of drum-grid 1.0`, with no digest, so every file written before documents
//! applies. A view with no header (written by hand or by an agent) needs `--as notes|grid`, and is
//! read as leniently as ever.
//!
//! # Reading back over the source
//!
//! Cell by cell, against the source document, with no ids: a grid cell is already an address.
//!
//! - **A lossless view onto its source** replaces the body with what the view reads, and keeps
//!   the source's envelope (`from` and `statements`).
//! - **A grid view onto a part** changes only the drum track it names: an unchanged cell keeps its
//!   note exactly (length and velocity); a changed level sets that note's velocity; a new hit is
//!   a note one step long; an emptied cell removes the note. Every other track is untouched.
//! - **Refused, by name:** a conductor line (tempo, meter, key, end, tpq), the step or the channel
//!   changed in a partial view, a lane or a bar added or removed there, and a source changed since
//!   the view was rendered.

use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use duet_core::time::beats_from_ticks;
use duet_core::{Grid, Note, Part, Pitch, Position};
use duet_drums::DrumGrid;
use duet_schemas::{AnyDocument, Document, Kind, Version};
use serde::Serialize;

use crate::config::Config;
use crate::drums::{ReadOptions, grid_of_part, levels_for};
use crate::io::{Format, part_format, read_with_from, same_file, tool, with_end, write_bytes};

/// A view of a document.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum ViewName {
    /// notefmt text: a part, one note or control per line.
    Notes,
    /// A drum grid's text: one row per lane, one column per step.
    Grid,
}

impl ViewName {
    fn name(self) -> &'static str {
        match self {
            ViewName::Notes => "notes",
            ViewName::Grid => "grid",
        }
    }
}

/// The one syntax version of each view this duet reads and writes.
const VIEW_VERSION: u32 = 1;

#[derive(Args)]
pub struct ViewArgs {
    /// The document to render: a .duet, or a part in an outside format (.mid, .abc), or - for a
    /// document on standard input.
    input: PathBuf,
    /// The view: notes for a part, grid for a drum grid (the defaults), or grid of a part's
    /// drum track, which takes --grid.
    #[arg(long = "as", value_enum)]
    as_view: Option<ViewName>,
    /// The file to write, instead of standard output.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// The grid view of a part: the step, such as 1/16.
    #[arg(long)]
    grid: Option<Grid>,
    /// The grid view of a part: the drum track's name, when it is not the only track on the
    /// lane map's channel.
    #[arg(long)]
    track: Option<String>,
    /// The grid view of a part: how many bars it shows, when not to the part's end.
    #[arg(long)]
    bars: Option<u32>,
    /// Show velocities off the text's levels snapped to the nearest. `apply --onto` keeps the
    /// exact velocity behind every level left unchanged.
    #[arg(long)]
    snap_levels: bool,
}

#[derive(Args)]
pub struct ApplyArgs {
    /// The edited view, or - for standard input.
    view: PathBuf,
    /// The document the view was rendered from. What the view does not show is kept, and the
    /// document is written in place, or to -o.
    #[arg(long)]
    onto: Option<PathBuf>,
    /// The document to write: a new one from a lossless view, or with --onto, where the result
    /// goes instead of over the source.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// The view, for one with no header line.
    #[arg(long = "as", value_enum)]
    as_view: Option<ViewName>,
    /// A grid view of a part with no header line: its step.
    #[arg(long)]
    grid: Option<Grid>,
    /// A grid view of a part with no header line: its drum track.
    #[arg(long)]
    track: Option<String>,
    /// A grid view of a part with no header line: how many bars it shows.
    #[arg(long)]
    bars: Option<u32>,
    /// Apply over a document that has changed since the view was rendered.
    #[arg(long)]
    force: bool,
}

// --- The header line ---

/// What a view's header line says.
#[derive(Debug, Clone, PartialEq)]
struct Header {
    view: ViewName,
    version: u32,
    kind: String,
    schema: Version,
    /// A partial view: the drum track, its step, and the bars asked for.
    track: Option<String>,
    step: Option<Grid>,
    bars: Option<u32>,
    digest: Option<u64>,
}

impl Header {
    fn render(&self) -> String {
        let mut line = format!(
            "# duet {} v{} of {} {}",
            self.view.name(),
            self.version,
            self.kind,
            self.schema
        );
        if let Some(track) = &self.track {
            let quoted = track.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = write!(line, " track \"{quoted}\"");
        }
        if let Some(step) = self.step {
            let _ = write!(line, " step {step}");
        }
        if let Some(bars) = self.bars {
            let _ = write!(line, " bars {bars}");
        }
        if let Some(digest) = self.digest {
            let _ = write!(line, " from {digest:016x}");
        }
        line
    }

    /// Reads a view's first line: a `# duet` header, today's `# notefmt` line, or `None`.
    fn parse(line: &str) -> Result<Option<Header>> {
        let line = line.trim_end();
        if line.starts_with("# notefmt drums v1") {
            return Ok(Some(Header::of("grid", "drum-grid", DrumGrid::VERSION)));
        }
        if line.starts_with("# notefmt v1") {
            return Ok(Some(Header::of("notes", "part", Part::VERSION)));
        }
        let Some(rest) = line.strip_prefix("# duet ") else {
            return Ok(None);
        };
        let bad = |why: &str| anyhow::anyhow!("the header line '{line}' is not a view's: {why}");
        let mut words = Words(rest);
        let view = match words.next() {
            Some("notes") => ViewName::Notes,
            Some("grid") => ViewName::Grid,
            other => return Err(bad(&format!("{other:?} is not a view (notes or grid)"))),
        };
        let version = words
            .next()
            .and_then(|v| v.strip_prefix('v'))
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| bad("the view's version is v<number>"))?;
        if words.next() != Some("of") {
            return Err(bad("'of <kind> <version>' follows the view"));
        }
        let kind = words.next().ok_or_else(|| bad("no kind"))?.to_owned();
        let schema = words
            .next()
            .ok_or_else(|| bad("no schema version"))?
            .parse()
            .map_err(|e| bad(&format!("{e}")))?;
        let mut header = Header {
            view,
            version,
            kind,
            schema,
            track: None,
            step: None,
            bars: None,
            digest: None,
        };
        while let Some(word) = words.next() {
            match word {
                "track" => {
                    header.track = Some(words.quoted().ok_or_else(|| bad("track \"<name>\""))?)
                }
                "step" => {
                    let step = words.next().ok_or_else(|| bad("step <grid>"))?;
                    header.step = Some(step.parse().map_err(|e| bad(&format!("{e}")))?);
                }
                "bars" => {
                    let bars = words.next().and_then(|b| b.parse().ok());
                    header.bars = Some(bars.ok_or_else(|| bad("bars <number>"))?);
                }
                "from" => {
                    let hex = words.next().filter(|h| h.len() == 16);
                    let digest = hex.and_then(|h| u64::from_str_radix(h, 16).ok());
                    header.digest = Some(digest.ok_or_else(|| bad("from <16 hex digits>"))?);
                }
                other => return Err(bad(&format!("'{other}' is not part of a header"))),
            }
        }
        Ok(Some(header))
    }

    fn of(view: &str, kind: &str, schema: Version) -> Header {
        Header {
            view: if view == "grid" {
                ViewName::Grid
            } else {
                ViewName::Notes
            },
            version: 1,
            kind: kind.to_owned(),
            schema,
            track: None,
            step: None,
            bars: None,
            digest: None,
        }
    }

    fn partial(&self) -> bool {
        self.view == ViewName::Grid && self.kind == Part::NAME
    }

    /// Refuses a view version or a schema version this duet does not read, by name.
    fn check(&self, path: &Path) -> Result<()> {
        let shown = path.display();
        if self.version != VIEW_VERSION {
            bail!(
                "{shown} is a {} v{} view; this duet reads {} v{VIEW_VERSION}; update duet",
                self.view.name(),
                self.version,
                self.view.name()
            );
        }
        let reads = match (self.view, self.kind.as_str()) {
            (_, k) if k == Part::NAME => Part::VERSION,
            (ViewName::Grid, k) if k == DrumGrid::NAME => DrumGrid::VERSION,
            (view, kind) => bail!("{shown}: a {} view is of no kind {kind}", view.name()),
        };
        let found = self.schema;
        if found.major != reads.major {
            bail!(
                "{shown} is a view of {} {found}; this duet reads {} {}.x; `duet convert` reads \
                 it from a duet that knows {found}",
                self.kind,
                self.kind,
                reads.major
            );
        }
        if found.minor > reads.minor {
            bail!(
                "{shown} is a view of {} {found}; this duet reads {} up to {reads}; update duet",
                self.kind,
                self.kind
            );
        }
        Ok(())
    }
}

/// The words of a header line, with a quoted name read as one.
struct Words<'a>(&'a str);

impl<'a> Words<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let s = self.0.trim_start();
        if s.is_empty() {
            return None;
        }
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        self.0 = &s[end..];
        Some(&s[..end])
    }

    /// A `"…"` name, with `\"` and `\\` escapes.
    fn quoted(&mut self) -> Option<String> {
        let s = self.0.trim_start().strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = s.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => out.push(chars.next()?.1),
                '"' => {
                    self.0 = &s[i + 1..];
                    return Some(out);
                }
                c => out.push(c),
            }
        }
        None
    }
}

/// The 64-bit FNV-1a digest of a body as compact JSON: what a view's `from` names.
fn digest<T: Serialize>(body: &T) -> Result<u64> {
    let bytes = serde_json::to_vec(body)?;
    Ok(bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    }))
}

// --- Documents ---

/// A document a view is rendered from or applied onto.
enum Source {
    Part(Document<Part>),
    Grid(Document<DrumGrid>),
}

fn text_of(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading standard input")?;
        Ok(text)
    } else {
        fs::read_to_string(path).with_context(|| path.display().to_string())
    }
}

/// Reads a document by its envelope, or a part from an outside format with its provenance.
/// The body comes back normalized, as every reader gives it.
fn load(path: &Path, what: &str) -> Result<Source> {
    let format = part_format(path, None, what)?;
    if format != Format::Duet {
        let (part, from) = read_with_from(path, format, what)?;
        let mut doc = Document::new(tool(), part);
        doc.from = from;
        return Ok(Source::Part(doc));
    }
    let text = text_of(path)?;
    match AnyDocument::read(&text).with_context(|| path.display().to_string())? {
        AnyDocument::Part(mut doc) => {
            doc.body.normalize()?;
            Ok(Source::Part(doc))
        }
        AnyDocument::DrumGrid(mut doc) => {
            doc.body.normalize()?;
            Ok(Source::Grid(doc))
        }
        other => bail!(
            "{} is a {} document, which has no text view",
            path.display(),
            other.kind()
        ),
    }
}

// --- duet view ---

pub fn run_view(args: ViewArgs, config: &Config) -> Result<()> {
    let source = load(&args.input, "duet view reads")?;
    let grid_flags: Vec<&str> = [
        (args.grid.is_some(), "--grid"),
        (args.track.is_some(), "--track"),
        (args.bars.is_some(), "--bars"),
    ]
    .into_iter()
    .filter_map(|(set, flag)| set.then_some(flag))
    .collect();
    let text = match (&source, args.as_view) {
        (Source::Part(doc), None | Some(ViewName::Notes)) => {
            if !grid_flags.is_empty() || args.snap_levels {
                bail!(
                    "{} are for the grid view of a part's drum track (--as grid)",
                    if grid_flags.is_empty() {
                        "--snap-levels".to_owned()
                    } else {
                        grid_flags.join(", ")
                    }
                );
            }
            let header = Header {
                digest: Some(digest(&doc.body)?),
                ..Header::of("notes", Part::NAME, Part::VERSION)
            };
            framed(
                &header,
                &doc.statements,
                &[],
                &with_end(&doc.body, config.end)?.to_text(),
            )
        }
        (Source::Grid(doc), None | Some(ViewName::Grid)) => {
            if !grid_flags.is_empty() {
                bail!(
                    "{} are for the grid view of a part's drum track; a drum grid has its own",
                    grid_flags.join(", ")
                );
            }
            let (grid, said) = shown_levels(&doc.body, args.snap_levels)?;
            let header = Header {
                digest: Some(digest(&doc.body)?),
                ..Header::of("grid", DrumGrid::NAME, DrumGrid::VERSION)
            };
            framed(&header, &doc.statements, &said, &grid.to_text()?)
        }
        (Source::Part(doc), Some(ViewName::Grid)) => {
            let Some(step) = args.grid else {
                bail!(
                    "the grid view of a part needs --grid, the step, such as --grid 1/16; duet \
                     never guesses it"
                );
            };
            let track = drum_track_name(&doc.body, args.track.as_deref(), config)?;
            let options = ReadOptions {
                grid: step,
                track: Some(track.clone()),
                bars: args.bars,
                snap_levels: false,
            };
            let exact = grid_of_part(&doc.body, &options, config)?;
            let (grid, said) = shown_levels(&exact, args.snap_levels)?;
            let header = Header {
                track: Some(track),
                step: Some(step),
                bars: args.bars,
                digest: Some(digest(&doc.body)?),
                ..Header::of("grid", Part::NAME, Part::VERSION)
            };
            framed(&header, &doc.statements, &said, &grid.to_text()?)
        }
        (Source::Grid(_), Some(ViewName::Notes)) => bail!(
            "{} is a drum-grid document, and the notes view is of a part; duet drums render makes \
             a part of a grid",
            args.input.display()
        ),
    };
    match &args.output {
        Some(path) => write_bytes(path, text.as_bytes()),
        None => write_bytes(Path::new("-"), text.as_bytes()),
    }
}

/// The grid as the view shows it: refused when a velocity is off the levels, unless snapped,
/// which a statement then says.
fn shown_levels(grid: &DrumGrid, snap: bool) -> Result<(DrumGrid, Vec<String>)> {
    if !snap {
        return Ok((levels_for(grid.clone(), false)?, Vec::new()));
    }
    let snapped = grid.snapped_to_levels();
    let moved = crate::drums::moved_velocities(grid, &snapped);
    let said = if moved == 0 {
        Vec::new()
    } else {
        vec![format!(
            "this view shows {moved} velocities snapped to the levels; apply --onto keeps them \
             exact where their level is unchanged"
        )]
    };
    Ok((snapped, said))
}

/// The view's text: the header line, the document's statements and the view's own as comments,
/// then the view.
fn framed(header: &Header, statements: &[String], said: &[String], body: &str) -> String {
    let mut out = header.render();
    out.push('\n');
    for line in statements.iter().chain(said) {
        let _ = writeln!(out, "# {line}");
    }
    out.push_str(body);
    out
}

/// The drum track a grid view of a part shows: the one named, or the only one on the lane
/// map's channel.
fn drum_track_name(part: &Part, named: Option<&str>, config: &Config) -> Result<String> {
    if let Some(name) = named {
        return Ok(name.to_owned());
    }
    let channel = config.lanes.channel();
    let on: Vec<&str> = part
        .tracks
        .iter()
        .filter(|t| t.channel == channel)
        .map(|t| t.name.as_str())
        .collect();
    match on.as_slice() {
        [one] => Ok((*one).to_owned()),
        [] => bail!(
            "the part has no track on channel {channel}, the drum channel; name it with --track"
        ),
        several => bail!(
            "the part has {} tracks on channel {channel} ({}); name the one to show with --track",
            several.len(),
            several.join(", ")
        ),
    }
}

// --- duet apply ---

pub fn run_apply(args: ApplyArgs, config: &Config) -> Result<()> {
    let text = text_of(&args.view)?;
    let shown = args.view.display();
    let header = Header::parse(text.lines().next().unwrap_or(""))?;
    if let Some(header) = &header {
        header.check(&args.view)?;
        if let Some(asked) = args.as_view
            && asked != header.view
        {
            bail!(
                "{shown} is a {} view by its header line, not a {} view",
                header.view.name(),
                asked.name()
            );
        }
        let given: Vec<&str> = [
            (args.grid.is_some(), "--grid"),
            (args.track.is_some(), "--track"),
            (args.bars.is_some(), "--bars"),
        ]
        .into_iter()
        .filter_map(|(set, flag)| set.then_some(flag))
        .collect();
        if !given.is_empty() {
            bail!(
                "{shown} names its part in its header line; {} are for a view with no header",
                given.join(", ")
            );
        }
    }
    let Some(view) = header.as_ref().map(|h| h.view).or(args.as_view) else {
        bail!(
            "{shown} has no header line saying what view it is; name it with --as notes or --as \
             grid"
        );
    };
    match (&args.onto, &args.output) {
        (None, None) => bail!(
            "apply needs --onto <document>, the document the view was rendered from, or -o \
             <new.duet> for a new document"
        ),
        (None, Some(output)) => apply_new(&text, view, header.as_ref(), output, &args, config),
        (Some(onto), output) => {
            let output = output.clone().unwrap_or_else(|| onto.clone());
            apply_onto(&text, view, header.as_ref(), onto, &output, &args, config)
        }
    }
}

/// A lossless view into a new document.
fn apply_new(
    text: &str,
    view: ViewName,
    header: Option<&Header>,
    output: &Path,
    args: &ApplyArgs,
    config: &Config,
) -> Result<()> {
    let partial =
        header.is_some_and(Header::partial) || args.grid.is_some() || args.track.is_some();
    refuse_named_output(output)?;
    let bytes = match view {
        ViewName::Grid if partial => bail!(
            "{} is a grid view of one track of a part, which is lossy: it makes no document of \
             its own; apply it --onto the part it was rendered from",
            args.view.display()
        ),
        ViewName::Notes => {
            let mut part = Part::from_text(text)?;
            part.normalize()?;
            // Every document duet writes states where its part ends, as `convert` does.
            Document::new(tool(), with_end(&part, config.end)?).write()?
        }
        ViewName::Grid => {
            let grid = read_grid_view(text, config)?;
            Document::new(tool(), grid).write()?
        }
    };
    write_bytes(output, bytes.as_bytes())
}

/// A document is written as a `.duet`, or to standard output.
fn refuse_named_output(output: &Path) -> Result<()> {
    if output != Path::new("-") && part_format(output, None, "apply writes")? != Format::Duet {
        bail!("{}: apply writes a document, a .duet", output.display());
    }
    Ok(())
}

fn read_grid_view(text: &str, config: &Config) -> Result<DrumGrid> {
    for notice in &config.text_notices {
        eprintln!("duet: {notice}");
    }
    Ok(DrumGrid::from_text_in(text, &config.text_lanes)?)
}

/// A view onto the document it was rendered from.
fn apply_onto(
    text: &str,
    view: ViewName,
    header: Option<&Header>,
    onto: &Path,
    output: &Path,
    args: &ApplyArgs,
    config: &Config,
) -> Result<()> {
    if part_format(onto, None, "apply --onto changes")? != Format::Duet || onto == Path::new("-") {
        bail!(
            "{}: --onto names the document to change, a .duet",
            onto.display()
        );
    }
    refuse_named_output(output)?;
    if args.view != Path::new("-") && same_file(&args.view, onto) {
        bail!(
            "{} is the view itself, not the document it was rendered from",
            onto.display()
        );
    }
    let source = load(onto, "apply --onto reads")?;
    let now = match &source {
        Source::Part(doc) => digest(&doc.body)?,
        Source::Grid(doc) => digest(&doc.body)?,
    };
    if let Some(was) = header.and_then(|h| h.digest)
        && was != now
    {
        if !args.force {
            bail!(
                "{} has changed since the view was rendered: the view is from {was:016x}, and \
                 the document is now {now:016x}; render the view again, or pass --force to apply \
                 it anyway",
                onto.display()
            );
        }
        eprintln!(
            "duet: applying over {}, which has changed since the view was rendered ({was:016x}, \
             now {now:016x}), as --force asks",
            onto.display()
        );
    }
    let partial = header.map_or(
        view == ViewName::Grid && matches!(source, Source::Part(_)),
        Header::partial,
    );
    let bytes = match (source, view) {
        (Source::Part(mut doc), ViewName::Notes) => {
            let mut part = Part::from_text(text)?;
            part.normalize()?;
            doc.body = part;
            doc.tool = tool();
            doc.write()?
        }
        (Source::Grid(mut doc), ViewName::Grid) if !partial => {
            let shown = read_grid_view(text, config)?;
            doc.body = keep_exact(&doc.body, shown);
            doc.body.check()?;
            doc.tool = tool();
            doc.write()?
        }
        (Source::Part(mut doc), ViewName::Grid) => {
            let (track, step, bars) = match header {
                Some(h) => (h.track.clone(), h.step, h.bars),
                None => (args.track.clone(), args.grid, args.bars),
            };
            let Some(step) = step else {
                bail!(
                    "{} has no header line naming its step; a grid view of a part needs --grid to \
                     be placed",
                    args.view.display()
                );
            };
            let track = drum_track_name(&doc.body, track.as_deref(), config)?;
            let shown = read_grid_view(text, config)?;
            merge_track(&mut doc.body, &shown, &track, step, bars, config)?;
            doc.tool = tool();
            doc.write()?
        }
        (source, view) => {
            let kind = match source {
                Source::Part(_) => Part::NAME,
                Source::Grid(_) => DrumGrid::NAME,
            };
            bail!(
                "{} is a {} view, and {} is a {kind} document it cannot be applied onto",
                args.view.display(),
                header.map_or_else(
                    || view.name().to_owned(),
                    |h| format!("{} view of a {}", h.view.name(), h.kind)
                ),
                onto.display()
            )
        }
    };
    write_bytes(output, bytes.as_bytes())
}

/// The grid a full view reads, with the source's exact velocity kept in every cell whose level
/// the view leaves as the source shows it.
fn keep_exact(source: &DrumGrid, mut shown: DrumGrid) -> DrumGrid {
    if shown.lanes != source.lanes
        || shown.bar_steps != source.bar_steps
        || shown.levels != source.levels
    {
        return shown;
    }
    let levels = source.levels;
    for (cell, exact) in shown.vel.data_mut().iter_mut().zip(source.vel.data()) {
        if *cell == levels.nearest(*exact) {
            *cell = *exact;
        }
    }
    shown
}

/// Merges a grid view of `track` onto the part, cell by cell against the grid the part shows
/// at that step: an unchanged cell keeps its note exactly; a changed level sets its velocity; a
/// new hit is a note one step long; an emptied cell removes its note.
fn merge_track(
    part: &mut Part,
    shown: &DrumGrid,
    track: &str,
    step: Grid,
    bars: Option<u32>,
    config: &Config,
) -> Result<()> {
    let source = DrumGrid::from_part(
        &with_end(part, config.end)?,
        step,
        &config.lanes,
        Some(track),
        bars,
    )?
    .snapped_to_levels();
    refuse_unplaceable(&source, shown)?;
    let map = part.time_map()?;
    let step_ticks = step.step_ticks(part.ticks_per_quarter)?;
    let beats = beats_from_ticks(step_ticks, part.ticks_per_quarter);
    let columns = source.steps();
    // The tick each column starts on.
    let mut ticks = Vec::with_capacity(columns);
    for (i, steps) in source.bar_steps.iter().enumerate() {
        let start = map.tick(Position::bar(i as u32 + 1))?;
        ticks.extend((0..u64::from(*steps)).map(|s| start + s * step_ticks));
    }
    let Some(drums) = part.tracks.iter_mut().find(|t| t.name == track) else {
        bail!("the part has no track named \"{track}\"");
    };
    for (lane_index, lane) in source.lanes.iter().enumerate() {
        for (column, tick) in ticks.iter().enumerate() {
            let was = source.vel.data()[lane_index * columns + column];
            let now = shown.vel.data()[lane_index * columns + column];
            if was == now {
                continue;
            }
            let at = |n: &Note| n.pitch.midi() == lane.key && map.tick(n.at).ok() == Some(*tick);
            match (was, now) {
                (0, vel) => {
                    let place = drums
                        .notes
                        .iter()
                        .position(|n| map.tick(n.at).is_ok_and(|t| t > *tick))
                        .unwrap_or(drums.notes.len());
                    drums.notes.insert(
                        place,
                        Note::new(map.position(*tick), Pitch::new(lane.key)?, beats, vel),
                    );
                }
                (_, 0) => drums.notes.retain(|n| !at(n)),
                (_, vel) => {
                    if let Some(note) = drums.notes.iter_mut().find(|n| at(n)) {
                        note.vel = vel;
                    }
                }
            }
        }
    }
    part.check_end()?;
    Ok(())
}

/// Refuses what a partial view cannot place, naming it: the conductor, the step or the channel
/// changed, a lane or a bar added or removed.
fn refuse_unplaceable(source: &DrumGrid, shown: &DrumGrid) -> Result<()> {
    let read_only = "a partial view shows it read-only; change it in the notes view, which shows \
                     the whole part";
    let (a, b) = (source.conductor(), shown.conductor());
    let fields: [(&str, String, String); 9] = [
        ("title", format!("{:?}", a.title), format!("{:?}", b.title)),
        (
            "tpq",
            a.ticks_per_quarter.to_string(),
            b.ticks_per_quarter.to_string(),
        ),
        (
            "tempo",
            format!("{:?}", a.tempo_bpm),
            format!("{:?}", b.tempo_bpm),
        ),
        ("time", a.time_sig.to_string(), b.time_sig.to_string()),
        ("key", format!("{:?}", a.key), format!("{:?}", b.key)),
        (
            "a tempo change",
            format!("{:?}", a.tempo_changes),
            format!("{:?}", b.tempo_changes),
        ),
        (
            "a time change",
            format!("{:?}", a.time_changes),
            format!("{:?}", b.time_changes),
        ),
        (
            "a key change",
            format!("{:?}", a.key_changes),
            format!("{:?}", b.key_changes),
        ),
        (
            "end",
            source.stated_end().to_string(),
            shown.stated_end().to_string(),
        ),
    ];
    for (what, part, view) in fields {
        if part != view {
            bail!(
                "the view changes the part's {what} (the part's is {part}, the view's {view}); {read_only}"
            );
        }
    }
    if source.grid != shown.grid {
        bail!(
            "the view's step is {}, but it was rendered at {}; {read_only}",
            shown.grid,
            source.grid
        );
    }
    if source.channel != shown.channel {
        bail!(
            "the view's channel is {}, but the track's is {}; {read_only}",
            shown.channel,
            source.channel
        );
    }
    let names = |g: &DrumGrid| {
        g.lanes
            .iter()
            .map(|l| format!("{} ({})", l.name, l.key))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if shown.lanes != source.lanes {
        bail!(
            "the view lists the lanes {}, but the track plays {}; a partial view cannot add or \
             remove a lane",
            names(shown),
            names(source)
        );
    }
    if shown.bar_steps != source.bar_steps {
        bail!(
            "the view shows {} bars, but the track runs to {}; a partial view cannot add or remove \
             a bar",
            shown.bar_count(),
            source.bar_count()
        );
    }
    if shown.levels != source.levels {
        bail!("the view changes the velocity levels; {read_only}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_line_round_trips() {
        let header = Header {
            track: Some("The \"Kit\"".into()),
            step: Some("1/8t".parse().unwrap()),
            bars: Some(4),
            digest: Some(0x0b7e_33c1_9a5d_2f84),
            ..Header::of("grid", "part", Version::new(1, 0))
        };
        let line = header.render();
        assert_eq!(
            line,
            "# duet grid v1 of part 1.0 track \"The \\\"Kit\\\"\" step 1/8t bars 4 from 0b7e33c19a5d2f84"
        );
        assert_eq!(Header::parse(&line).unwrap(), Some(header));
        let notes = Header::parse("# duet notes v1 of part 1.0 from 3f9a1c0d5e2b7a41")
            .unwrap()
            .unwrap();
        assert_eq!(
            (notes.view, notes.digest),
            (ViewName::Notes, Some(0x3f9a_1c0d_5e2b_7a41))
        );
    }

    #[test]
    fn todays_lines_read_as_v1_of_1_0_and_others_as_no_header() {
        let notes = Header::parse("# notefmt v1: positions are bar.beat.tick")
            .unwrap()
            .unwrap();
        assert_eq!(
            (notes.view, notes.kind.as_str(), notes.digest),
            (ViewName::Notes, "part", None)
        );
        let grid = Header::parse("# notefmt drums v1: one row per lane")
            .unwrap()
            .unwrap();
        assert_eq!(
            (grid.view, grid.kind.as_str()),
            (ViewName::Grid, "drum-grid")
        );
        assert_eq!(Header::parse("tpq 480").unwrap(), None);
        assert_eq!(Header::parse("# a comment").unwrap(), None);
        for bad in [
            "# duet tab v1 of part 1.0",
            "# duet notes 1 of part 1.0",
            "# duet notes v1 of part",
            "# duet notes v1 of part 1.0 from xyz",
            "# duet notes v1 of part 1.0 colour red",
        ] {
            assert!(Header::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_newer_view_or_schema_version_is_refused_by_name() {
        let path = Path::new("x.notes");
        let check = |line: &str| {
            Header::parse(line)
                .unwrap()
                .unwrap()
                .check(path)
                .map_err(|e| e.to_string())
        };
        assert!(check("# duet notes v1 of part 1.0").is_ok());
        let err = check("# duet notes v2 of part 1.0").unwrap_err();
        assert!(
            err.contains("notes v2 view; this duet reads notes v1"),
            "{err}"
        );
        let err = check("# duet notes v1 of part 1.4").unwrap_err();
        assert!(
            err.contains("view of part 1.4; this duet reads part up to 1.0"),
            "{err}"
        );
        let err = check("# duet notes v1 of part 2.0").unwrap_err();
        assert!(
            err.contains("part 1.x") && err.contains("duet convert"),
            "{err}"
        );
        let err = check("# duet grid v1 of lute-tab 1.0").unwrap_err();
        assert!(err.contains("no kind lute-tab"), "{err}");
    }

    #[test]
    fn the_digest_is_fnv_1a() {
        // FNV-1a's published test vectors: "" and "a".
        let fnv = |bytes: &[u8]| {
            bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        };
        assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(digest(&"a").unwrap(), fnv(b"\"a\""));
    }
}
