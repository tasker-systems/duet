//! `duet project adopt <dir>`: scratch work brought into a project (the project design's §7,
//! ruled in temper `01a1f1b0` to stand as written).
//!
//! **A plan, then an explicit apply. It copies; it never moves.** Without `--apply` it prints
//! where each file in the directory would go and writes nothing. With it, it copies what the plan
//! said, byte for byte, and refuses every destination that exists before it writes anything. The
//! scratch directory is never changed: deleting it is the owner's. No document is rewritten, so
//! every digest a comp or a view's header holds still matches.
//!
//! **How each file is sorted.** It reads every file directly in the directory, and each
//! `STEM.review/` directory there:
//! - **a recording** (WAV, FLAC, MP3, Ogg Vorbis, M4A) becomes a take, named from its stem as
//!   `take add` names one, copied into `takes/<take>/` or, with `--named`, left where it is. One
//!   the project has already, by its digest, is not added again;
//! - **a `.duet` or `.duet.jsonl`** is sorted by its envelope's kind, never its name:
//!   - `capture-analysis` and `performance` are derived, and go to the take whose digest they
//!     state. One from before the take digest (`capture-analysis` below 1.13, `performance` 1.0)
//!     states none, so it is matched as `duet capture compare` matches: to the take of its stem,
//!     its rate the same as its performance's, and its frames running no further than the
//!     take's length. The plan says so: such a reading is no variant;
//!   - a part or a drum grid beside a capture of its stem (`STEM.capture.duet`) is derived, and
//!     goes with that capture; any other goes to the song's `parts/`. A part from before the
//!     `part` rename (its kind `song`) states no `source`, and the plan says so;
//!   - `comp` goes to the song's `comps/`;
//! - **`.notes`, `.drums` and `.abc` text** goes where a part of its stem would;
//! - **a `.moments` file** goes to `review/`, and **a `STEM.review/` sheet** to its take's
//!   `takes/<take>/review/`;
//! - **anything else** is listed as left behind, with why, and never guessed at.
//!
//! Without `--song`, every part and comp is listed as needing a song, and left. `--song` names
//! the song, and `--apply` makes it, as `duet song new` does, when it does not exist.
//!
//! **The journal.** Each document copied in is a `doc.written`, as every document written in a
//! project is, and so is the project's document and a song made. Each take added is one
//! `take.added`, the act `duet take add` records: adopting a recording adds it to the project
//! exactly as `take add` does, so it is recorded the same way. A recording the project has
//! already records nothing.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use duet_capture::{Analysis, Event, PerformanceFrames, PerformanceHead, Take};
use duet_schemas::journal::TakeAdded;
use duet_schemas::project::{FileName, Name, ProjectTake};
use duet_schemas::{Document, PartBody, Song, Stream, read_envelope};
use schemars::JsonSchema;
use serde_json::Value;

use crate::capture::compare::LENGTH_SLACK_SECS;
use crate::io::{read_grid_document, stdout};
use crate::song::{NewInput, SongCommand};
use crate::take::{
    TAKES_DIR, TAKES_LOCAL_FILE, copy_new, format_of, name_from, read_project, set_local, take_of,
    write_project,
};
use crate::{Context, PROJECT_FILE};

/// The recordings adopt makes takes of, by extension.
const AUDIO: &[&str] = &["wav", "flac", "mp3", "ogg", "m4a"];

/// The text views adopt sorts as parts are sorted.
const TEXT: &[&str] = &["notes", "drums", "abc"];

/// What a reading from before the take digest is told.
const PRE_DIGEST: &str = "states no take digest: matched to {take} by its stem, rate and \
                          length, as compare matches; not a variant, so capture it again when \
                          a comp is wanted";

/// Bring scratch work into the project. Every file directly in the directory is sorted, as the
/// project design's §7 lists: recordings become takes; captures and performances go to the take
/// whose digest they state; parts, grids and their text views go to the song, or with their
/// capture; comps to the song's comps/; moments to review/; STEM.review/ sheets to their take.
/// Anything else is left behind. Without --apply, the plan is printed and nothing is written.
#[derive(Debug, Args, JsonSchema)]
pub struct AdoptInput {
    /// The scratch directory. Nothing in it is changed, moved or deleted.
    pub dir: PathBuf,
    /// The song the parts and comps go to, made when it does not exist. Without it, each part
    /// and comp is listed as needing a song, and left.
    #[arg(long)]
    pub song: Option<Name>,
    /// Add each recording as a named take, left where it is, rather than copied in.
    #[arg(long)]
    pub named: bool,
    /// Copy what the plan says. Each destination that exists is refused, and nothing is written.
    #[arg(long)]
    pub apply: bool,
}

/// A recording the plan adds as a take.
#[derive(Debug)]
struct NewTake {
    from: PathBuf,
    file: FileName,
    name: Name,
    take: Take,
}

/// A take a document can belong to: one the plan adds, or one the project has, by a stem.
#[derive(Debug, Clone)]
struct Known {
    stem: String,
    name: Name,
    take: Take,
}

/// One file the plan copies, to a path under the project's root.
#[derive(Debug)]
struct Planned {
    from: PathBuf,
    to: PathBuf,
}

/// What a document read as.
#[derive(Debug)]
enum Read {
    Analysis {
        take: Option<Take>,
        rate: u32,
        last: u64,
    },
    Performance {
        take: Option<Take>,
        rate: u32,
        last: Option<u64>,
    },
    /// A part or a drum grid, with what the plan says of it.
    Part(Option<&'static str>),
    Comp,
}

/// A document in the directory.
#[derive(Debug)]
struct Doc {
    file: String,
    path: PathBuf,
    /// `kind version`, as the plan names it.
    kind: String,
    stem: String,
    read: Read,
}

/// The plan: what goes where, and what is left.
#[derive(Debug, Default)]
struct Plan {
    takes: Vec<NewTake>,
    copies: Vec<Planned>,
    /// What the plan says, section by section.
    take_lines: Vec<String>,
    copy_lines: Vec<String>,
    needs_song: Vec<String>,
    left: Vec<String>,
    /// Every destination, to refuse a second file to one.
    dests: BTreeSet<PathBuf>,
}

impl Plan {
    fn leave(&mut self, file: &str, why: impl std::fmt::Display) {
        self.left.push(format!("{file}: {why}"));
    }

    /// Plans `from` copied to `to`, or leaves it when another file goes there already.
    fn copy(&mut self, file: &str, from: &Path, to: PathBuf, line: String) {
        if !self.dests.insert(to.clone()) {
            self.leave(
                file,
                format!("{} is another file's destination already", to.display()),
            );
            return;
        }
        self.copies.push(Planned {
            from: from.to_owned(),
            to,
        });
        self.copy_lines.push(line);
    }
}

/// Runs `duet project adopt`.
///
/// # Errors
///
/// Outside a project; when the directory does not read or is inside the project; with `--apply`,
/// when a destination exists or a file cannot be copied or written.
pub fn run(ctx: &Context, input: AdoptInput) -> Result<()> {
    let AdoptInput {
        dir,
        song,
        named,
        apply,
    } = input;
    let Some(project) = ctx.project() else {
        bail!(
            "no project here: adopt brings scratch work into a project, and there is no \
             {PROJECT_FILE} from the working directory up, nor a DUET_PROJECT; `duet init` makes \
             one"
        );
    };
    if !dir.is_dir() {
        bail!("{}: no such directory", dir.display());
    }
    let scratch = fs::canonicalize(&dir).with_context(|| dir.display().to_string())?;
    let root =
        fs::canonicalize(&project.root).with_context(|| project.root.display().to_string())?;
    if scratch.starts_with(&root) {
        bail!(
            "{} is inside the project {}: adopt copies scratch work in from outside it",
            dir.display(),
            root.display()
        );
    }
    let mut doc = read_project(&root)?;
    let song = song.map(|name| {
        let made = root
            .join("songs")
            .join(name.as_str())
            .join("song.duet")
            .is_file();
        (name, made)
    });
    let plan = plan(&scratch, &doc.body, song.as_ref().map(|(n, _)| n), named)?;

    // Every destination that exists already: --apply refuses them all before writing anything.
    let copied_takes = plan.takes.iter().filter(|_| !named).map(copied_to);
    let exists: Vec<String> = copied_takes
        .chain(plan.copies.iter().map(|c| c.to.clone()))
        .filter(|to| root.join(to).exists())
        .map(|to| to.display().to_string())
        .collect();
    let said = shown(&scratch, &root, song.as_ref(), named, &plan, &exists, apply);
    if !apply {
        return stdout(&said);
    }
    if !exists.is_empty() {
        let which = if exists.len() == 1 {
            format!("{} exists already", exists[0])
        } else {
            format!("each of these exists already: {}", exists.join(", "))
        };
        bail!("{which}; adopt never replaces a file, so nothing is written");
    }

    if let Some((name, false)) = &song {
        crate::song::run(
            ctx,
            SongCommand::New(NewInput {
                song: name.clone(),
                title: None,
                intent: None,
                tempo: None,
                meter: None,
                key: None,
                project: root.clone(),
            }),
        )?;
    }
    let mut made = Made::default();
    if let Err(e) = copy_all(&root, &plan, named, &mut made) {
        made.undo();
        return Err(e);
    }
    let added = crate::project::today()?;
    let mut local = Vec::new();
    for take in &plan.takes {
        if named {
            let at = fs::canonicalize(&take.from).with_context(|| take.from.display().to_string());
            if let Err(e) = at.and_then(|at| set_local(&root, &take.name, Some(&at))) {
                undo_local(&root, &local);
                made.undo();
                return Err(e);
            }
            local.push(take.name.clone());
        }
        doc.body.takes.push(ProjectTake::new(
            take.name.clone(),
            take.take,
            take.file.clone(),
            added,
        ));
    }
    if let Err(e) = write_project(ctx, &root, &mut doc) {
        undo_local(&root, &local);
        made.undo();
        return Err(e);
    }
    for copy in &plan.copies {
        ctx.wrote(&root.join(&copy.to), false)?;
    }
    for take in &plan.takes {
        ctx.emit(TakeAdded::new(take.name.as_str(), take.take.digest));
    }
    stdout(&format!(
        "{said}adopted {} take{} and {} file{}; {} is as it was\n",
        plan.takes.len(),
        if plan.takes.len() == 1 { "" } else { "s" },
        made.files.len(),
        if made.files.len() == 1 { "" } else { "s" },
        scratch.display()
    ))
}

/// `takes/<take>/<file>`, where a copied take goes.
fn copied_to(take: &NewTake) -> PathBuf {
    Path::new(TAKES_DIR)
        .join(take.name.as_str())
        .join(take.file.as_str())
}

/// Sorts every file in `scratch`.
fn plan(
    scratch: &Path,
    project: &duet_schemas::Project,
    song: Option<&Name>,
    named: bool,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut entries: Vec<(String, PathBuf, fs::FileType)> = Vec::new();
    for entry in fs::read_dir(scratch).with_context(|| scratch.display().to_string())? {
        let entry = entry.with_context(|| scratch.display().to_string())?;
        let kind = entry
            .file_type()
            .with_context(|| entry.path().display().to_string())?;
        entries.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
            kind,
        ));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // What a take can be found by: the project's takes by name and by their file's stem, and the
    // recordings here by theirs.
    let mut known: Vec<Known> = Vec::new();
    for take in &project.takes {
        for stem in [take.name.as_str(), stem(take.file.as_str())] {
            known.push(Known {
                stem: stem.to_owned(),
                name: take.name.clone(),
                take: take.take,
            });
        }
    }

    let mut docs = Vec::new();
    let mut texts = Vec::new();
    let mut reviews = Vec::new();
    for (file, path, kind) in &entries {
        let lower = file.to_ascii_lowercase();
        if kind.is_symlink() {
            plan.leave(
                file,
                "a symbolic link: adopt copies files, and never follows a link",
            );
        } else if kind.is_dir() {
            if let Some(stem) = strip(file, ".review") {
                reviews.push((file.clone(), path.clone(), stem.to_owned()));
            } else {
                plan.leave(
                    &format!("{file}/"),
                    "a directory: adopt sorts the files directly in the one given, and \
                     STEM.review/ sheets",
                );
            }
        } else if lower.ends_with(".duet") || lower.ends_with(".duet.jsonl") {
            match read_doc(file, path) {
                Ok(doc) => docs.push(doc),
                Err(why) => plan.leave(file, why),
            }
        } else if let Some(ext) = extension(&lower)
            && AUDIO.contains(&ext)
        {
            recording(&mut plan, &mut known, project, file, path, named);
        } else if let Some(ext) = extension(&lower)
            && TEXT.contains(&ext)
        {
            texts.push((file.clone(), path.clone(), stem(file).to_owned()));
        } else if extension(&lower) == Some("moments") {
            let to = Path::new("review").join(file);
            let line = format!("{file} to {}", to.display());
            plan.copy(file, path, to, line);
        } else {
            plan.leave(file, "duet has no use for it");
        }
    }

    // Captures and performances, each to its take; a capture's stem carries its siblings.
    let performances: Vec<(String, u32)> = docs
        .iter()
        .filter_map(|d| match d.read {
            Read::Performance { rate, .. } => Some((d.stem.clone(), rate)),
            _ => None,
        })
        .collect();
    let mut captures: Vec<(String, Result<Name, ()>)> = Vec::new();
    for doc in &docs {
        let owner = match &doc.read {
            Read::Analysis { take, rate, last } => {
                let paired = longest(
                    &doc.stem,
                    performances.iter().map(|(s, r)| (s.as_str(), *r)),
                );
                let rated = match paired {
                    Some(r) if r != *rate => Err(format!(
                        "counts frames at {rate} Hz, and the performance of its stem at {r} Hz: \
                         they are not of one performance"
                    )),
                    _ => Ok(()),
                };
                rated.and_then(|()| owner(&known, &doc.stem, take.as_ref(), *rate, Some(*last)))
            }
            Read::Performance { take, rate, last } => {
                owner(&known, &doc.stem, take.as_ref(), *rate, *last)
            }
            Read::Part(_) | Read::Comp => continue,
        };
        if matches!(doc.read, Read::Analysis { .. }) {
            captures.push((
                doc.stem.clone(),
                owner.as_ref().map(|(n, _)| n.clone()).map_err(|_| ()),
            ));
        }
        match owner {
            Ok((name, pre_digest)) => {
                let to = Path::new(TAKES_DIR).join(name.as_str()).join(&doc.file);
                let mut line = format!("{} ({}) to {}", doc.file, doc.kind, to.display());
                if pre_digest {
                    let _ = write!(line, "; {}", PRE_DIGEST.replace("{take}", name.as_str()));
                }
                plan.copy(&doc.file, &doc.path, to, line);
            }
            Err(why) => plan.leave(&format!("{} ({})", doc.file, doc.kind), why),
        }
    }
    let beside = |stem: &str| {
        captures
            .iter()
            .find(|(s, _)| s == stem)
            .map(|(_, n)| n.clone())
    };

    // Parts, grids and their text: with their capture, else to the song.
    for doc in &docs {
        let (note, comp) = match doc.read {
            Read::Part(note) => (note, false),
            Read::Comp => (None, true),
            Read::Analysis { .. } | Read::Performance { .. } => continue,
        };
        let named = format!("{} ({})", doc.file, doc.kind);
        if !comp && let Some(owner) = beside(&doc.stem) {
            derived(
                &mut plan, &named, &doc.file, &doc.path, &doc.stem, owner, note,
            );
            continue;
        }
        let into = if comp { "comps" } else { "parts" };
        if !comp && doc.stem.parse::<Name>().is_err() {
            plan.leave(
                &named,
                "its stem is no part's name, and a song's part is named by its stem (lowercase \
                 words joined by hyphens)",
            );
            continue;
        }
        to_song(&mut plan, song, &named, &doc.file, &doc.path, into, note);
    }
    for (file, path, stem) in &texts {
        if let Some(owner) = beside(stem) {
            derived(&mut plan, file, file, path, stem, owner, None);
        } else {
            to_song(&mut plan, song, file, file, path, "parts", None);
        }
    }

    // Review sheets, to the take of their stem.
    for (file, path, stem) in &reviews {
        let owner = beside(stem).and_then(Result::ok).or_else(|| {
            known
                .iter()
                .find(|k| k.stem == *stem)
                .map(|k| k.name.clone())
        });
        let Some(take) = owner else {
            plan.leave(
                &format!("{file}/"),
                format!(
                    "no recording here, capture here or take in the project has the stem {stem}"
                ),
            );
            continue;
        };
        let to = Path::new(TAKES_DIR).join(take.as_str()).join("review");
        let mut files = Vec::new();
        walk(path, Path::new(""), &mut files)?;
        if files.is_empty() {
            plan.leave(&format!("{file}/"), "holds no file");
            continue;
        }
        if !plan.dests.insert(to.clone()) {
            plan.leave(
                &format!("{file}/"),
                format!("{} is another sheet's destination already", to.display()),
            );
            continue;
        }
        for rel in files {
            plan.copies.push(Planned {
                from: path.join(&rel),
                to: to.join(&rel),
            });
        }
        plan.copy_lines
            .push(format!("{file}/ to {}/", to.display()));
    }
    Ok(plan)
}

/// A part, grid or text beside the capture of `stem`: it goes with that capture.
fn derived(
    plan: &mut Plan,
    named: &str,
    file: &str,
    path: &Path,
    stem: &str,
    owner: Result<Name, ()>,
    note: Option<&str>,
) {
    match owner {
        Ok(take) => {
            let to = Path::new(TAKES_DIR).join(take.as_str()).join(file);
            let mut line = format!("{named} to {}", to.display());
            if let Some(note) = note {
                let _ = write!(line, "; {note}");
            }
            plan.copy(file, path, to, line);
        }
        Err(()) => plan.leave(
            named,
            format!("it is beside {stem}.capture.duet, which is left behind"),
        ),
    }
}

/// A part, grid, comp or text to the song's `into`, or listed as needing a song.
fn to_song(
    plan: &mut Plan,
    song: Option<&Name>,
    named: &str,
    file: &str,
    path: &Path,
    into: &str,
    note: Option<&str>,
) {
    let Some(song) = song else {
        plan.needs_song
            .push(format!("{named} needs a song: --song names one"));
        return;
    };
    let to = Path::new("songs").join(song.as_str()).join(into).join(file);
    let mut line = format!("{named} to {}", to.display());
    if let Some(note) = note {
        let _ = write!(line, "; {note}");
    }
    plan.copy(file, path, to, line);
}

/// A recording: a take the plan adds, one the project has already, or why it is left.
fn recording(
    plan: &mut Plan,
    known: &mut Vec<Known>,
    project: &duet_schemas::Project,
    file: &str,
    path: &Path,
    named: bool,
) {
    let take = match take_of(path) {
        Ok(take) => take,
        Err(why) => return plan.leave(file, format!("does not read as a recording: {why:#}")),
    };
    let stem = stem(file).to_owned();
    if let Some(had) = project.takes.iter().find(|t| t.take.digest == take.digest) {
        plan.take_lines.push(format!(
            "{file} is the take {} already ({}); not added again",
            had.name, take.digest
        ));
        known.push(Known {
            stem,
            name: had.name.clone(),
            take,
        });
        return;
    }
    if let Some(same) = plan.takes.iter().find(|t| t.take.digest == take.digest) {
        return plan.leave(
            file,
            format!(
                "the same recording as {}: both have the digest {}",
                same.file, take.digest
            ),
        );
    }
    let name = match name_from(path) {
        Ok(name) => name,
        Err(why) => return plan.leave(file, format!("{why:#}")),
    };
    if let Some(other) = project.take(&name) {
        return plan.leave(
            file,
            format!(
                "the project has a take named {name} already, another recording ({}); `duet take \
                 add --name` adds it under another name",
                other.take.digest
            ),
        );
    }
    if let Some(other) = plan.takes.iter().find(|t| t.name == name) {
        return plan.leave(
            file,
            format!("its take name, {name}, is {}'s already", other.file),
        );
    }
    let file_name: FileName = match file.parse() {
        Ok(f) => f,
        Err(why) => return plan.leave(file, why),
    };
    let new = NewTake {
        from: path.to_owned(),
        file: file_name,
        name: name.clone(),
        take,
    };
    let kept = if named {
        format!(
            "named at {}, in {TAKES_LOCAL_FILE}; nothing copied",
            path.display()
        )
    } else {
        format!("copied to {}", copied_to(&new).display())
    };
    plan.take_lines.push(format!(
        "take {name}: {file}, {kept}; {}, {}",
        take.digest,
        format_of(&take)
    ));
    known.push(Known { stem, name, take });
    plan.takes.push(new);
}

/// The take a capture or performance goes to, and whether it was matched without a digest; or
/// why it is left.
fn owner(
    known: &[Known],
    stem: &str,
    take: Option<&Take>,
    rate: u32,
    last: Option<u64>,
) -> Result<(Name, bool), String> {
    if let Some(take) = take {
        return known
            .iter()
            .find(|k| k.take.digest == take.digest)
            .map(|k| (k.name.clone(), false))
            .ok_or_else(|| {
                format!(
                    "states the take {}, which no recording here and no take in the project is",
                    take.digest
                )
            });
    }
    let Some(found) = longest(stem, known.iter().map(|k| (k.stem.as_str(), k))) else {
        return Err(
            "states no take digest, and no recording here nor take in the project has its stem"
                .to_owned(),
        );
    };
    let take_secs = found.take.frames as f64 / f64::from(found.take.rate);
    if let Some(last) = last {
        let runs = last as f64 / f64::from(rate);
        if runs > take_secs + LENGTH_SLACK_SECS {
            return Err(format!(
                "states no take digest, and its frames run to {runs:.2} s, past the {take_secs:.2} \
                 s of {}, the take of its stem: it is not that take's",
                found.name
            ));
        }
    }
    Ok((found.name.clone(), true))
}

/// The value of the key that is `stem`, or the longest one `stem` starts with followed by a dot:
/// `wayfaring.hmm` is of `wayfaring`.
fn longest<'a, T>(stem: &str, keys: impl Iterator<Item = (&'a str, T)>) -> Option<T> {
    keys.filter(|(key, _)| {
        !key.is_empty()
            && (stem == *key
                || stem
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.starts_with('.')))
    })
    .max_by_key(|(key, _)| key.len())
    .map(|(_, value)| value)
}

/// Reads a `.duet` or `.duet.jsonl` far enough to sort it, or says why it is left.
fn read_doc(file: &str, path: &Path) -> Result<Doc, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("does not read: {e}"))?;
    let stream = file.to_ascii_lowercase().ends_with(".duet.jsonl");
    let first = if stream {
        text.split_once('\n')
            .map_or(text.as_str(), |(first, _)| first)
    } else {
        text.as_str()
    };
    let envelope =
        read_envelope(first).map_err(|e| format!("does not read as a duet document: {e}"))?;
    let kind = format!("{} {}", envelope.kind, envelope.version);
    let refused = |e: duet_schemas::Error| format!("a {kind} that does not read: {e}");
    let (read, stem) = match envelope.kind.as_str() {
        "capture-analysis" => {
            let body: Analysis = Document::read(&text).map_err(refused)?.body;
            let last = body.events.iter().map(last_frame).max().unwrap_or(0);
            (
                Read::Analysis {
                    take: body.take,
                    rate: body.rate,
                    last,
                },
                strip(file, ".capture.duet").or_else(|| strip(file, ".duet")),
            )
        }
        "performance" => {
            let stream = Stream::<PerformanceHead>::read(&text).map_err(refused)?;
            let mut frames = PerformanceFrames::new(&stream.head);
            for block in &stream.records {
                frames
                    .push(block)
                    .map_err(|e| format!("a {kind} that does not read: {e}"))?;
            }
            let last = frames.len().checked_sub(1).map(|k| frames.frame(k));
            (
                Read::Performance {
                    take: stream.head.take,
                    rate: stream.head.rate,
                    last,
                },
                strip(file, ".performance.duet.jsonl").or_else(|| strip(file, ".duet.jsonl")),
            )
        }
        "part" => {
            let note = match Document::<PartBody>::read(&text) {
                Ok(_) => None,
                Err(_) if !states(&text, "source") => Some(
                    "a part from before parts stated a source: it states none, so duet song \
                         show refuses it until it is made again",
                ),
                Err(e) => return Err(refused(e)),
            };
            (Read::Part(note), strip(file, ".duet"))
        }
        "drum-grid" => {
            read_grid_document(path, "a drum grid").map_err(|e| format!("{e:#}"))?;
            (Read::Part(None), strip(file, ".duet"))
        }
        "song" => match Document::<Song>::read(&text) {
            Ok(_) => {
                return Err(format!(
                    "a {kind}: a song's own document is never adopted; --song names the song \
                     its parts go to"
                ));
            }
            Err(_) if !states(&text, "composition") => (
                Read::Part(Some(
                    "a part from before the part rename: it states no source, so duet song show \
                     refuses it until it is made again",
                )),
                strip(file, ".duet"),
            ),
            Err(e) => return Err(refused(e)),
        },
        "comp" => (Read::Comp, strip(file, ".duet")),
        other => {
            return Err(format!(
                "a {other} document: adopt sorts captures, performances, parts, grids and comps"
            ));
        }
    };
    let Some(stem) = stem else {
        return Err(format!(
            "a {kind} whose file name does not end as its kind's does"
        ));
    };
    Ok(Doc {
        file: file.to_owned(),
        path: path.to_owned(),
        kind,
        stem: stem.to_owned(),
        read,
    })
}

/// Whether a document's body names `field`.
fn states(text: &str, field: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|doc| doc.get("body").map(|body| body.get(field).is_some()))
        .unwrap_or(false)
}

/// The last frame an analysis event reaches.
fn last_frame(event: &Event) -> u64 {
    match *event {
        Event::Onset { frame, .. } | Event::Hit { frame, .. } => frame,
        Event::Note { end, .. } => end,
        // An event this build does not know reaches no frame it can count.
        _ => 0,
    }
}

/// `file` without `suffix`, matched in any case, when it ends so and something is left.
fn strip<'a>(file: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = file.len().checked_sub(suffix.len())?;
    (cut > 0 && file.is_char_boundary(cut) && file[cut..].eq_ignore_ascii_case(suffix))
        .then(|| &file[..cut])
}

/// A file name's last extension.
fn extension(lower: &str) -> Option<&str> {
    lower.rsplit_once('.').map(|(_, ext)| ext)
}

/// A file name without its last extension.
fn stem(file: &str) -> &str {
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

/// Every file under `dir`, by its path from it. A link inside is refused, never followed.
fn walk(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let here = dir.join(rel);
    let mut entries: Vec<_> = fs::read_dir(&here)
        .with_context(|| here.display().to_string())?
        .collect::<std::io::Result<_>>()
        .with_context(|| here.display().to_string())?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let kind = entry.file_type()?;
        let next = rel.join(entry.file_name());
        if kind.is_symlink() {
            bail!(
                "{}: a symbolic link; adopt copies files, and never follows a link",
                dir.join(&next).display()
            );
        } else if kind.is_dir() {
            walk(dir, &next, out)?;
        } else {
            out.push(next);
        }
    }
    Ok(())
}

/// What `adopt` prints: the plan, section by section.
fn shown(
    scratch: &Path,
    root: &Path,
    song: Option<&(Name, bool)>,
    named: bool,
    plan: &Plan,
    exists: &[String],
    apply: bool,
) -> String {
    let mut out = format!("adopt {}\ninto {}\n", scratch.display(), root.display());
    if let Some((song, made)) = song {
        let dir = Path::new("songs").join(song.as_str());
        if *made {
            let _ = writeln!(out, "song {song}: {}/, as it is", dir.display());
        } else {
            let _ = writeln!(
                out,
                "song {song}: made, {}",
                dir.join("song.duet").display()
            );
        }
    }
    for line in &plan.take_lines {
        let _ = writeln!(out, "{line}");
    }
    if plan.takes.is_empty() && plan.take_lines.is_empty() {
        out.push_str("takes none\n");
    } else if named {
        let _ = writeln!(out, "  named takes stay where they are");
    }
    if !plan.copy_lines.is_empty() {
        out.push_str("copied\n");
    }
    for line in &plan.copy_lines {
        let _ = writeln!(out, "  {line}");
    }
    if !plan.needs_song.is_empty() {
        out.push_str("needs a song\n");
    }
    for line in &plan.needs_song {
        let _ = writeln!(out, "  {line}");
    }
    for line in &plan.left {
        let _ = writeln!(out, "left behind: {line}");
    }
    for to in exists {
        let _ = writeln!(out, "exists: {to}, so --apply refuses");
    }
    if !apply {
        out.push_str("a plan: nothing written; --apply copies it\n");
    }
    out
}

/// What an apply has made so far, to take back when it fails.
#[derive(Debug, Default)]
struct Made {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Made {
    /// Makes `dir` and every missing directory above it, keeping each it made.
    fn dir(&mut self, root: &Path, dir: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut at = root.join(dir);
        while !at.exists() {
            missing.push(at.clone());
            let Some(up) = at.parent() else { break };
            at = up.to_owned();
        }
        for dir in missing.into_iter().rev() {
            fs::create_dir(&dir).with_context(|| dir.display().to_string())?;
            self.dirs.push(dir);
        }
        Ok(())
    }

    /// Removes what this apply made, newest first.
    fn undo(&self) {
        for file in self.files.iter().rev() {
            let _ = fs::remove_file(file);
        }
        for dir in self.dirs.iter().rev() {
            let _ = fs::remove_dir(dir);
        }
    }
}

/// Copies every take's recording (unless named) and every file, each new, byte for byte.
fn copy_all(root: &Path, plan: &Plan, named: bool, made: &mut Made) -> Result<()> {
    let takes = plan
        .takes
        .iter()
        .filter(|_| !named)
        .map(|take| (&take.from, copied_to(take)));
    let copies = plan.copies.iter().map(|c| (&c.from, c.to.clone()));
    for (from, to) in takes.chain(copies) {
        if let Some(dir) = to.parent() {
            made.dir(root, dir)?;
        }
        let dest = root.join(&to);
        copy_new(from, &dest)?;
        made.files.push(dest);
    }
    Ok(())
}

/// Takes the named takes' paths back out of `.duet/takes.local.toml`.
fn undo_local(root: &Path, names: &[Name]) {
    for name in names {
        let _ = set_local(root, name, None);
    }
}

#[cfg(test)]
mod tests {
    use super::{longest, strip};

    #[test]
    fn a_reading_is_of_the_longest_stem_it_starts_with() {
        let keys = [("wayfaring", 1), ("wayfaring-two", 2), ("way", 3)];
        let of = |stem: &str| longest(stem, keys.iter().copied());
        assert_eq!(of("wayfaring"), Some(1));
        assert_eq!(of("wayfaring.hmm"), Some(1));
        assert_eq!(of("wayfaring-two.rules"), Some(2));
        assert_eq!(of("wayfarings"), None, "a stem continues only after a dot");
        assert_eq!(of("other"), None);
    }

    #[test]
    fn a_suffix_is_stripped_in_any_case_and_never_to_nothing() {
        assert_eq!(strip("take.capture.duet", ".capture.duet"), Some("take"));
        assert_eq!(strip("Take.CAPTURE.duet", ".capture.duet"), Some("Take"));
        assert_eq!(strip(".duet", ".duet"), None);
        assert_eq!(strip("x.abc", ".duet"), None);
    }
}
