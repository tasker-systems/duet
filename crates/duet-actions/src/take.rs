//! `duet take add | locate | list | forget`: a project's takes, and how any command finds one
//! (the project design's §4, ruled in temper `01a1eb60`, decision 4).
//!
//! - **A take is added explicitly.** `take add` decodes the file once and computes its take: the
//!   digest of its decoded samples, which is its identity, and their rate, channels and frames.
//!   By default the file is copied byte for byte to `takes/<take>/<file>`, never transcoded. With
//!   `--named` nothing is copied, and the file's absolute path goes in `.duet/takes.local.toml`,
//!   which is this machine's.
//! - **Finding a take** ([`find`]): `takes/<take>/<file>` first, then `takes.local.toml`. The file
//!   found is checked against the take's digest, once per command. A missing or changed file is
//!   refused, naming the take, its digest, where duet looked, and `duet take locate`.
//! - **No symlinks, ever.** A take is a file in the project, or a path in the local map.
//! - **A take is never removed by a command.** `take forget` drops its entry and its local path,
//!   and leaves every file in `takes/<take>/`.
//!
//! **The journal.** Each write of `.duet/project.duet` is a `doc.written`, as every document write
//! is. `take add`, and `duet record` into a project, also record a `take.added`, naming the take by
//! its name and digest: it is what the journal's index names a take by. `.duet/takes.local.toml`
//! is machine-local and no document, so its writes are not recorded.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use duet_capture::{Digester, FileSource, Gap, Source as _, Take};
use duet_schemas::journal::TakeAdded;
use duet_schemas::project::{Date, FileName, Name, ProjectTake};
use duet_schemas::{Document, PartBody, Project as ProjectDoc, Source};
use schemars::JsonSchema;

use crate::io::{same_file, stdout, tool};
use crate::{Context, PROJECT_FILE, Project};

/// The directory a project's takes are copied into, under its root.
pub const TAKES_DIR: &str = "takes";

/// Where a named take lives on this machine, under a project's root.
pub const TAKES_LOCAL_FILE: &str = ".duet/takes.local.toml";

#[derive(Debug, Subcommand)]
pub enum TakeCommand {
    /// Add a recording to the project as a take. It is decoded once, for its digest, rate,
    /// channels and frames. By default it is copied byte for byte to takes/TAKE/FILE;
    /// --named copies nothing and keeps its path in .duet/takes.local.toml, this machine's.
    Add(AddInput),
    /// Find a take again at another path, checked against its digest, and keep that path in
    /// .duet/takes.local.toml.
    Locate(LocateInput),
    /// List every take, and where it is found, or that it is not: takes/TAKE/FILE first,
    /// then .duet/takes.local.toml, each checked against its digest.
    List(ListInput),
    /// Drop a take from the project, and its path from .duet/takes.local.toml. Every file in
    /// takes/TAKE/ is left for you to delete.
    Forget(ForgetInput),
}

/// Add a recording to the project as a take, copied into takes/TAKE/ or, with --named, named
/// where it is.
#[derive(Debug, Args, JsonSchema)]
pub struct AddInput {
    /// The recording: WAV, FLAC, MP3, Ogg Vorbis, or AAC in an MP4 (.m4a).
    pub file: PathBuf,
    /// The take's name in the project: lowercase letters and digits, words joined by single
    /// hyphens. Default: the file's name without its extension, spelled so.
    #[arg(long)]
    pub name: Option<Name>,
    /// Copy nothing: keep the file where it is, its absolute path in .duet/takes.local.toml.
    #[arg(long)]
    pub named: bool,
}

/// Find a take again at another path, checked against its digest.
#[derive(Debug, Args, JsonSchema)]
pub struct LocateInput {
    /// The take's name in the project.
    pub take: Name,
    /// Where the take's file is now.
    pub path: PathBuf,
}

/// List every take, and where it is found, or that it is not.
#[derive(Debug, Args, JsonSchema)]
pub struct ListInput {}

/// Drop a take from the project; its files are left where they are.
#[derive(Debug, Args, JsonSchema)]
pub struct ForgetInput {
    /// The take's name in the project.
    pub take: Name,
}

/// Runs `duet take …`.
///
/// # Errors
///
/// Outside a project; when a file does not read or a take is refused; when a file cannot be
/// written.
pub fn run(ctx: &Context, command: TakeCommand) -> Result<()> {
    let Some(project) = ctx.project() else {
        bail!(
            "no project here: a take belongs to a project, and there is no {} from the working \
             directory up, nor a DUET_PROJECT; `duet init` makes one",
            PROJECT_FILE
        );
    };
    match command {
        TakeCommand::Add(input) => add(ctx, project, input),
        TakeCommand::Locate(input) => locate(project, &input),
        TakeCommand::List(ListInput {}) => stdout(&listed(project)?),
        TakeCommand::Forget(input) => forget(ctx, project, &input),
    }
}

fn add(ctx: &Context, project: &Project, input: AddInput) -> Result<()> {
    let AddInput { file, name, named } = input;
    let mut doc = read_project(&project.root)?;
    let name = match name {
        Some(name) => name,
        None => name_from(&file)?,
    };
    refuse_name_taken(&doc.body, &name)?;
    if !file.is_file() {
        bail!("{}: no such file", file.display());
    }
    let file_name: FileName = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .parse()
        .map_err(anyhow::Error::msg)
        .with_context(|| file.display().to_string())?;
    let take = take_of(&file)?;
    refuse_digest_taken(&doc.body, &take, &file)?;

    let copied = Path::new(TAKES_DIR)
        .join(name.as_str())
        .join(file_name.as_str());
    let said = if named {
        let at = fs::canonicalize(&file).with_context(|| file.display().to_string())?;
        set_local(&project.root, &name, Some(&at))?;
        format!(
            "named at {}, in {TAKES_LOCAL_FILE}; nothing copied",
            at.display()
        )
    } else {
        let dest = project.root.join(&copied);
        if !(dest.exists() && same_file(&file, &dest)) {
            copy_new(&file, &dest)?;
        }
        format!("copied to {}", copied.display())
    };
    doc.body
        .takes
        .push(ProjectTake::new(name.clone(), take, file_name, today()?));
    if let Err(e) = write_project(ctx, &project.root, &mut doc) {
        if !named {
            // The copy belongs to an entry that was never written, so it goes too.
            let _ = fs::remove_file(project.root.join(&copied));
            let _ = fs::remove_dir(project.root.join(TAKES_DIR).join(name.as_str()));
        } else {
            let _ = set_local(&project.root, &name, None);
        }
        return Err(e);
    }
    ctx.emit(TakeAdded::new(name.as_str(), take.digest));
    stdout(&format!(
        "added take {name}: {}, {}; {said}\n",
        take.digest,
        format_of(&take)
    ))
}

fn locate(project: &Project, input: &LocateInput) -> Result<()> {
    let doc = read_project(&project.root)?;
    let take = taken(&doc.body, &input.take)?;
    if !input.path.is_file() {
        bail!("{}: no such file", input.path.display());
    }
    let found = take_of(&input.path)?;
    if found.digest != take.take.digest {
        bail!(
            "{} is not the take {}: its digest is {}, and the take's is {}",
            input.path.display(),
            take.name,
            found.digest,
            take.take.digest
        );
    }
    let at = fs::canonicalize(&input.path).with_context(|| input.path.display().to_string())?;
    set_local(&project.root, &take.name, Some(&at))?;
    let mut said = format!(
        "take {}: found at {}, named in {TAKES_LOCAL_FILE}\n",
        take.name,
        at.display()
    );
    let copied = copied_path(take);
    if project.root.join(&copied).is_file() {
        let _ = writeln!(
            said,
            "{} is looked at first, and is there",
            copied.display()
        );
    }
    stdout(&said)
}

fn forget(ctx: &Context, project: &Project, input: &ForgetInput) -> Result<()> {
    let mut doc = read_project(&project.root)?;
    let take = taken(&doc.body, &input.take)?.clone();
    let (linked, unreadable) = linked_by(&project.root, &take)?;
    doc.body.takes.retain(|t| t.name != take.name);
    write_project(ctx, &project.root, &mut doc)?;
    set_local(&project.root, &take.name, None)?;
    for what in &linked {
        let how = if what.ends_with("/song.duet") {
            "still links"
        } else {
            "is still comped from"
        };
        eprintln!(
            "{what} {how} the take {} ({}), which the project no longer names",
            take.name, take.take.digest
        );
    }
    for line in &unreadable {
        eprintln!("{line}");
    }
    let dir = Path::new(TAKES_DIR).join(take.name.as_str());
    let mut left = Vec::new();
    if let Ok(entries) = fs::read_dir(project.root.join(&dir)) {
        for entry in entries.flatten() {
            left.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    left.sort();
    let mut said = format!("forgot take {} ({})\n", take.name, take.take.digest);
    if !left.is_empty() {
        let _ = writeln!(
            said,
            "left in {}/, for you to delete: {}",
            dir.display(),
            left.join(", ")
        );
    }
    stdout(&said)
}

/// What `duet take list` prints: each take, and where it is found, or why not.
fn listed(project: &Project) -> Result<String> {
    let doc = read_project(&project.root)?;
    if doc.body.takes.is_empty() {
        return Ok("takes none\n".to_owned());
    }
    let local = read_local(&project.root)?;
    let mut out = String::new();
    for take in &doc.body.takes {
        let _ = writeln!(
            out,
            "{} ({}), {}, {}: {}",
            take.name,
            take.file,
            take.take.digest,
            format_of(&take.take),
            look(&project.root, take, &local).describe()
        );
    }
    Ok(out)
}

/// A take's file, found and checked against its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Where the file is.
    pub path: PathBuf,
    /// Whether it was found in `takes/` or through the local map.
    pub how: How,
}

/// How a take's file was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum How {
    /// At `takes/<take>/<file>`, copied in.
    Copied,
    /// At the path `.duet/takes.local.toml` names.
    Named,
}

/// What looking for a take found.
#[derive(Debug)]
pub(crate) enum Looked {
    Found(Located),
    /// The file at this path is not the take: its digest is another's, or it holds no audio.
    Changed {
        path: PathBuf,
        holds: String,
        looked: Vec<String>,
    },
    /// No file is at any place looked.
    Missing {
        looked: Vec<String>,
    },
    /// The file found does not read as audio.
    Unreadable {
        path: PathBuf,
        why: String,
    },
}

impl Looked {
    /// One line, for `take list` and `project show`.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Found(Located {
                path,
                how: How::Copied,
            }) => format!("found at {}", path.display()),
            Self::Found(Located {
                path,
                how: How::Named,
            }) => format!("found at {}, named in {TAKES_LOCAL_FILE}", path.display()),
            Self::Changed { path, holds, .. } => {
                format!("changed: {} holds {holds}, not this take", path.display())
            }
            Self::Missing { looked } => format!("not found: looked at {}", looked.join(" and ")),
            Self::Unreadable { path, why } => {
                format!("does not read: {}: {why}", path.display())
            }
        }
    }
}

/// Finds `take`'s file, for any command: `takes/<take>/<file>` first, then the path
/// `.duet/takes.local.toml` names, checked against its digest. Call it once per command: each
/// call decodes the file.
///
/// # Errors
///
/// When the file is missing, does not read, or is not the take: the refusal names the take, its
/// digest, where duet looked, and `duet take locate`.
pub fn find(root: &Path, take: &ProjectTake) -> Result<Located> {
    let local = read_local(root)?;
    let locate = format!(
        "`duet take locate {} <path>` names where it is now",
        take.name
    );
    match look(root, take, &local) {
        Looked::Found(found) => Ok(found),
        Looked::Missing { looked } => bail!(
            "take {} ({}) is not found: duet looked at {}; {locate}",
            take.name,
            take.take.digest,
            looked.join(" and ")
        ),
        Looked::Changed {
            path,
            holds,
            looked,
        } => bail!(
            "take {} ({}) has changed: {} holds {holds}; duet looked at {}; {locate}",
            take.name,
            take.take.digest,
            path.display(),
            looked.join(" and ")
        ),
        Looked::Unreadable { path, why } => bail!(
            "take {} ({}): {} does not read: {why}; {locate}",
            take.name,
            take.take.digest,
            path.display()
        ),
    }
}

/// The take a command was given (the project design's §5): the file to read, and the project's
/// take it is, when it is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// The file to read: the path given, or where [`find`] found the take named.
    pub file: PathBuf,
    /// The project's take the file is, when the command runs in a project and it is one.
    pub take: Option<InProject>,
}

/// One of the project's takes, as a command given it knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InProject {
    /// The project's root.
    pub root: PathBuf,
    /// The take's name.
    pub name: Name,
}

impl InProject {
    /// The take's directory, `takes/<take>/`, under the root.
    #[must_use]
    pub fn dir(&self) -> PathBuf {
        self.root.join(TAKES_DIR).join(self.name.as_str())
    }

    /// A reading's default stem: `takes/<take>/<take>.<label>`, where the label is its reader.
    #[must_use]
    pub fn stem(&self, label: &str) -> PathBuf {
        self.dir().join(format!("{}.{label}", self.name))
    }

    /// The labels of the take's readings: each `takes/<take>/<take>.<label>.capture.duet`,
    /// sorted.
    #[must_use]
    pub fn readings(&self) -> Vec<String> {
        let prefix = format!("{}.", self.name);
        let mut out: Vec<String> = fs::read_dir(self.dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let label = name.strip_prefix(&prefix)?.strip_suffix(".capture.duet")?;
                (!label.is_empty() && !label.contains('.')).then(|| label.to_owned())
            })
            .collect();
        out.sort();
        out
    }

    /// The stem of the take's one reading, for `compare` and `review` without `-o`.
    ///
    /// # Errors
    ///
    /// When the take has no reading in its directory, or more than one, naming each.
    pub fn reading_stem(&self) -> Result<PathBuf> {
        let readings = self.readings();
        let dir = Path::new(TAKES_DIR).join(self.name.as_str());
        match readings.as_slice() {
            [one] => Ok(self.stem(one)),
            [] => bail!(
                "the take {} has no reading in {}/: `duet capture melody {}` makes one",
                self.name,
                dir.display(),
                self.name
            ),
            many => bail!(
                "the take {} has {} readings in {}/, {}: -o names which, such as -o {}",
                self.name,
                many.len(),
                dir.display(),
                many.join(", "),
                self.stem(&many[0]).display()
            ),
        }
    }
}

/// What a command was given as a take, read in the project it runs in, if any. Outside a
/// project it is the path given, as always.
///
/// Inside one:
/// - **A take's name** (`duet capture melody wayfaring`) is found by [`find`], checked against
///   its digest, and where it was found is said on standard error. A name the project has no
///   take of, and no file has, is refused naming the takes it has.
/// - **A path** is read as given. With `digest`, its decode is digested, once, and the project's
///   take with that digest is the take it is, wherever the file lies.
///
/// # Errors
///
/// When the project's document does not read, the take named is not found, or the file given
/// does not decode.
pub fn input(ctx: &Context, given: &Path, digest: bool) -> Result<Input> {
    let outside = || Input {
        file: given.to_path_buf(),
        take: None,
    };
    let Some(project) = ctx.project() else {
        return Ok(outside());
    };
    let doc = read_project(&project.root)?;
    let in_project = |entry: &ProjectTake| InProject {
        root: project.root.clone(),
        name: entry.name.clone(),
    };
    if let Some(name) = given.to_str().and_then(|s| s.parse::<Name>().ok()) {
        if let Some(entry) = doc.body.take(&name) {
            let found = find(&project.root, entry)?;
            eprintln!("take {name}: {}", Looked::Found(found.clone()).describe());
            return Ok(Input {
                file: project.root.join(&found.path),
                take: Some(in_project(entry)),
            });
        }
        if !given.exists() {
            taken(&doc.body, &name)?;
        }
    }
    if !digest {
        return Ok(outside());
    }
    if !given.is_file() {
        bail!("{}: no such file", given.display());
    }
    let take = take_of(given)?;
    Ok(Input {
        file: given.to_path_buf(),
        take: doc
            .body
            .takes
            .iter()
            .find(|t| t.take.digest == take.digest)
            .map(in_project),
    })
}

/// The `duet take add` line for a file that is no take of the project, and the take's name it
/// would be given.
#[must_use]
pub fn add_line(file: &Path) -> (String, Option<Name>) {
    (
        format!("duet take add {}", file.display()),
        name_from(file).ok(),
    )
}

/// Looks for `take` as [`find`] does, saying what it found rather than refusing.
pub(crate) fn look(root: &Path, take: &ProjectTake, local: &toml::Table) -> Looked {
    let copied = copied_path(take);
    let named = local_path(local, &take.name);
    let mut looked = vec![copied.display().to_string()];
    if let Some(at) = &named {
        looked.push(format!("{}, named in {TAKES_LOCAL_FILE}", at.display()));
    }
    let candidates = [(root.join(&copied), copied.clone(), How::Copied)]
        .into_iter()
        .chain(named.map(|at| (at.clone(), at, How::Named)));
    for (full, shown, how) in candidates {
        if !full.is_file() {
            continue;
        }
        return match take_of(&full) {
            Ok(found) if found.digest == take.take.digest => {
                Looked::Found(Located { path: shown, how })
            }
            Ok(found) => Looked::Changed {
                path: shown,
                holds: found.digest.to_string(),
                looked,
            },
            Err(why) => Looked::Unreadable {
                path: shown,
                why: format!("{why:#}"),
            },
        };
    }
    Looked::Missing { looked }
}

/// `takes/<take>/<file>`, relative to the project's root.
fn copied_path(take: &ProjectTake) -> PathBuf {
    Path::new(TAKES_DIR)
        .join(take.name.as_str())
        .join(take.file.as_str())
}

/// The take `file` holds: its whole decode through the take digester.
///
/// # Errors
///
/// When the file does not decode, or holds no audio at all.
pub fn take_of(file: &Path) -> Result<Take> {
    let mut source = FileSource::open(file)?;
    let mut digester = Digester::new();
    while let Some(chunk) = source.next_chunk()? {
        digester
            .push(&chunk)
            .with_context(|| file.display().to_string())?;
    }
    let take = digester
        .finish()
        .with_context(|| format!("{}: holds no audio, so it is no take", file.display()))?;
    for said in gaps_said(file, take.rate, source.gaps()) {
        eprintln!("{said}");
    }
    Ok(take)
}

/// What a decode left out, one line a gap: it is read as silence, so what follows keeps its
/// time, and the digest is over the silence-filled audio. Nothing for a whole decode.
pub(crate) fn gaps_said(path: &Path, rate: u32, gaps: &[Gap]) -> Vec<String> {
    gaps.iter()
        .map(|gap| {
            format!(
                "{}: {} of audio at {} could not be decoded and is read as silence",
                path.display(),
                span(gap.frames, rate),
                at(gap.start, rate)
            )
        })
        .collect()
}

/// A position in a take, as minutes and seconds to a tenth.
pub(crate) fn at(frames: u64, rate: u32) -> String {
    let tenths = (frames as f64 / f64::from(rate) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// A length of audio, in milliseconds: under one is said as such, never as nothing.
pub(crate) fn span(frames: u64, rate: u32) -> String {
    let ms = frames as f64 * 1000.0 / f64::from(rate);
    if frames > 0 && ms < 1.0 {
        "under 1 ms".to_owned()
    } else {
        format!("{ms:.0} ms")
    }
}

/// A take's rate, channels and frames, for people.
pub(crate) fn format_of(take: &Take) -> String {
    format!(
        "{} Hz, {} channel{}, {} frames",
        take.rate,
        take.channels,
        if take.channels == 1 { "" } else { "s" },
        take.frames
    )
}

/// The take named `name`, or a refusal naming the project's takes.
fn taken<'a>(project: &'a ProjectDoc, name: &Name) -> Result<&'a ProjectTake> {
    project.take(name).with_context(|| {
        let names: Vec<&str> = project.takes.iter().map(|t| t.name.as_str()).collect();
        if names.is_empty() {
            format!("the project has no take {name}: it has no takes")
        } else {
            format!(
                "the project has no take {name}; its takes are {}",
                names.join(", ")
            )
        }
    })
}

/// Refuses a name the project has already given a take.
pub(crate) fn refuse_name_taken(project: &ProjectDoc, name: &Name) -> Result<()> {
    if let Some(other) = project.take(name) {
        bail!(
            "the project has a take named {name} already ({}, {}): a take's name is unique in \
             it; --name gives another",
            other.file,
            other.take.digest
        );
    }
    Ok(())
}

/// Refuses a recording the project has already, under any name: the digest is the take's
/// identity (#88's rule).
fn refuse_digest_taken(project: &ProjectDoc, take: &Take, file: &Path) -> Result<()> {
    if let Some(other) = project.takes.iter().find(|t| t.take.digest == take.digest) {
        bail!(
            "{} is the take {} already: both have the digest {}, and a take is in the project \
             once",
            file.display(),
            other.name,
            take.digest
        );
    }
    Ok(())
}

/// The name a take gets from its file when none is given: its stem, lowercased, every run of
/// other characters a single hyphen.
pub(crate) fn name_from(file: &Path) -> Result<Name> {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut spelled = String::new();
    for c in stem.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            spelled.push(c);
        } else if !spelled.is_empty() && !spelled.ends_with('-') {
            spelled.push('-');
        }
    }
    let spelled = spelled.trim_end_matches('-');
    spelled.parse().map_err(|why: String| {
        anyhow::anyhow!(
            "{}: no take name can be made from its file name ({why}); give one with --name",
            file.display()
        )
    })
}

/// Copies `from` to `to` byte for byte, making `to`'s directory. `to` must not exist.
pub(crate) fn copy_new(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    let mut source = fs::File::open(from).with_context(|| from.display().to_string())?;
    let mut dest = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .with_context(|| format!("{}: a take never replaces a file", to.display()))?;
    let copied = std::io::copy(&mut source, &mut dest)
        .and_then(|_| dest.sync_all())
        .with_context(|| format!("copying {} to {}", from.display(), to.display()));
    if copied.is_err() {
        drop(dest);
        let _ = fs::remove_file(to);
    }
    copied
}

/// The project's document.
pub(crate) fn read_project(root: &Path) -> Result<Document<ProjectDoc>> {
    let path = root.join(PROJECT_FILE);
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Document::read(&text).with_context(|| path.display().to_string())
}

/// Writes the project's document over itself, checked first, as this duet: written beside it,
/// then renamed over it, so a failed write leaves the old one whole. Recorded as a
/// `doc.written` that replaced it.
pub(crate) fn write_project(
    ctx: &Context,
    root: &Path,
    doc: &mut Document<ProjectDoc>,
) -> Result<()> {
    let path = root.join(PROJECT_FILE);
    doc.body
        .check()
        .map_err(anyhow::Error::msg)
        .with_context(|| path.display().to_string())?;
    doc.tool = tool();
    let text = doc.write()?;
    let beside = path.with_extension("duet.writing");
    fs::write(&beside, text.as_bytes()).with_context(|| beside.display().to_string())?;
    fs::rename(&beside, &path).with_context(|| path.display().to_string())?;
    ctx.wrote(&path, true)
}

/// `.duet/takes.local.toml`, or an empty table when there is none.
pub(crate) fn read_local(root: &Path) -> Result<toml::Table> {
    let path = root.join(TAKES_LOCAL_FILE);
    if !path.is_file() {
        return Ok(toml::Table::new());
    }
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    parse_local(&text).with_context(|| path.display().to_string())
}

fn parse_local(text: &str) -> Result<toml::Table> {
    Ok(toml::from_str(text)?)
}

/// The path the local map names for `name`.
fn local_path(local: &toml::Table, name: &Name) -> Option<PathBuf> {
    local
        .get(name.as_str())
        .and_then(|entry| entry.get("path"))
        .and_then(toml::Value::as_str)
        .map(PathBuf::from)
}

/// Sets `name`'s path in `.duet/takes.local.toml`, or with `None` takes it out, keeping every
/// other entry as it was. A map left empty is removed.
pub(crate) fn set_local(root: &Path, name: &Name, at: Option<&Path>) -> Result<()> {
    let path = root.join(TAKES_LOCAL_FILE);
    let mut local = read_local(root)?;
    match at {
        Some(at) => {
            let Some(text) = at.to_str() else {
                bail!(
                    "{}: the path is not UTF-8, so {TAKES_LOCAL_FILE} cannot hold it",
                    at.display()
                );
            };
            let mut entry = toml::Table::new();
            entry.insert("path".to_owned(), toml::Value::String(text.to_owned()));
            local.insert(name.as_str().to_owned(), toml::Value::Table(entry));
        }
        None => {
            if local.remove(name.as_str()).is_none() {
                return Ok(());
            }
        }
    }
    if local.is_empty() {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| path.display().to_string())
            }
            _ => Ok(()),
        };
    }
    let text = write_local(&local)?;
    let beside = path.with_extension("toml.writing");
    fs::File::create(&beside)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .with_context(|| beside.display().to_string())?;
    fs::rename(&beside, &path).with_context(|| path.display().to_string())
}

const LOCAL_HEADER: &str = "\
# Where this project's named takes are on this machine, written by `duet take add --named` and
# `duet take locate`. Machine-local: another machine keeps its own.
";

fn write_local(local: &toml::Table) -> Result<String> {
    Ok(format!("{LOCAL_HEADER}\n{}", toml::to_string(local)?))
}

/// What still names `take`, by its digest, one line each, sorted: the songs whose `takes` link it
/// (`songs/<song>/song.duet`), and the song parts comped from it (`songs/<song>/parts/<part>.duet`).
/// A song that does not read is not skipped in silence: it comes back as the second list, for the
/// caller to say, since it may link the take.
fn linked_by(root: &Path, take: &ProjectTake) -> Result<(Vec<String>, Vec<String>)> {
    let songs = root.join("songs");
    let mut out = Vec::new();
    let (linking, unreadable) =
        crate::placing::songs_linking_or_unreadable(root, &take.take.digest)?;
    for (name, _) in linking {
        out.push(format!("songs/{name}/song.duet"));
    }
    let unreadable = unreadable
        .into_iter()
        .map(|(name, why)| {
            format!(
                "songs/{name}/song.duet could not be read, so it may link the take {}: {why}",
                take.name
            )
        })
        .collect();
    let Ok(entries) = fs::read_dir(&songs) else {
        return Ok((out, unreadable));
    };
    for song in entries.flatten() {
        let parts = song.path().join("parts");
        let Ok(parts) = fs::read_dir(&parts) else {
            continue;
        };
        for part in parts.flatten() {
            let path = part.path();
            if path.extension().is_none_or(|e| e != "duet") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = Document::<PartBody>::read(&text) else {
                continue;
            };
            if let Source::Comped { takes, .. } = &doc.body.source
                && takes.iter().any(|t| t.digest == take.take.digest)
            {
                out.push(format!(
                    "songs/{}/parts/{}",
                    song.file_name().to_string_lossy(),
                    part.file_name().to_string_lossy()
                ));
            }
        }
    }
    out.sort();
    Ok((out, unreadable))
}

/// Adds a take recorded straight into `takes/<name>/<file>` to the project: `duet record` in a
/// project.
///
/// # Errors
///
/// When the file does not read, the take is in the project already, or the document cannot be
/// written.
pub fn add_recorded(ctx: &Context, project: &Project, name: &Name, file: &Path) -> Result<Take> {
    let mut doc = read_project(&project.root)?;
    refuse_name_taken(&doc.body, name)?;
    let take = take_of(file)?;
    refuse_digest_taken(&doc.body, &take, file)?;
    let file_name: FileName = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .parse()
        .map_err(anyhow::Error::msg)?;
    doc.body
        .takes
        .push(ProjectTake::new(name.clone(), take, file_name, today()?));
    write_project(ctx, &project.root, &mut doc)?;
    ctx.emit(TakeAdded::new(name.as_str(), take.digest));
    Ok(take)
}

/// Today's date, in UTC.
fn today() -> Result<Date> {
    crate::project::today()
}

#[cfg(test)]
mod tests {
    use super::{find, name_from, parse_local, take_of, write_local};
    use duet_schemas::project::ProjectTake;
    use std::path::Path;

    /// `frames` of a 16-bit mono WAV at 8 kHz, each sample `value`.
    fn wav(path: &Path, frames: u32, value: i16) {
        let data = frames * 2;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(8000u32.to_le_bytes());
        b.extend(16000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data.to_le_bytes());
        for _ in 0..frames {
            b.extend(value.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn finding_a_take_checks_its_digest_and_refuses_a_missing_or_changed_file_by_name() {
        let root = std::env::temp_dir().join(format!("duet-take-find-{}", std::process::id()));
        let dir = root.join("takes/idea");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.join(".duet")).unwrap();
        let file = dir.join("idea.wav");
        wav(&file, 800, 1000);
        let take = ProjectTake::new(
            "idea".parse().unwrap(),
            take_of(&file).unwrap(),
            "idea.wav".parse().unwrap(),
            "2026-11-19".parse().unwrap(),
        );
        let found = find(&root, &take).unwrap();
        assert_eq!(found.path, Path::new("takes/idea/idea.wav"));
        assert_eq!(found.how, super::How::Copied);

        wav(&file, 800, -1000);
        let changed = format!("{:#}", find(&root, &take).unwrap_err());
        let digest = take.take.digest.to_string();
        for said in [
            "take idea",
            digest.as_str(),
            "has changed",
            "takes/idea/idea.wav",
            "`duet take locate idea <path>`",
        ] {
            assert!(changed.contains(said), "{said:?} in {changed}");
        }

        std::fs::remove_file(&file).unwrap();
        let elsewhere = root.join("gone.wav");
        std::fs::write(
            root.join(super::TAKES_LOCAL_FILE),
            format!("[idea]\npath = {:?}\n", elsewhere.to_str().unwrap()),
        )
        .unwrap();
        let missing = format!("{:#}", find(&root, &take).unwrap_err());
        for said in [
            "is not found",
            digest.as_str(),
            "takes/idea/idea.wav and ",
            "gone.wav, named in .duet/takes.local.toml",
            "duet take locate",
        ] {
            assert!(missing.contains(said), "{said:?} in {missing}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_take_name_is_made_from_its_file_name() {
        let name = |f: &str| name_from(Path::new(f)).map(|n| n.to_string());
        assert_eq!(name("Blood Sport 3.m4a").unwrap(), "blood-sport-3");
        assert_eq!(name("/x/wayfaring.flac").unwrap(), "wayfaring");
        assert_eq!(name("__Idea--two__.wav").unwrap(), "idea-two");
        assert!(name("ÉÉ.wav").is_err(), "no letter duet names with");
    }

    #[test]
    fn the_local_map_round_trips_exactly() {
        let mut local = toml::Table::new();
        for (name, path) in [
            ("blood-sport", "/Volumes/Phone/Blood Sport 3.m4a"),
            ("wayfaring", r#"C:\takes\"quoted" é.flac"#),
        ] {
            let mut entry = toml::Table::new();
            entry.insert("path".into(), toml::Value::String(path.into()));
            local.insert(name.into(), toml::Value::Table(entry));
        }
        let text = write_local(&local).unwrap();
        assert!(
            text.contains("[blood-sport]\npath = \"/Volumes/Phone/Blood Sport 3.m4a\"\n"),
            "{text}"
        );
        let back = parse_local(&text).unwrap();
        assert_eq!(back, local);
        assert_eq!(
            write_local(&back).unwrap(),
            text,
            "written again, byte for byte"
        );
    }
}
