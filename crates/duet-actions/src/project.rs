//! `duet init` and `duet project path | show`: making a project, and reading the one a command runs
//! in (the project design's §3 and §5, ruled in temper `01a1eb60`).
//!
//! A project is the nearest directory, from the working directory up, whose `.duet/` holds
//! `project.duet`, or the one `DUET_PROJECT` names ([`Project::find`]). A `.duet/` without one is
//! what it has always been: a config directory, and nothing more.
//!
//! **`init` journals nothing.** It writes `.duet/project.duet` and, when absent, a
//! `.duet/duet.toml` of comments, and nothing else (§3), so no journal appears beside them. The
//! project's account begins with the first action run in it. An `init --nested` inside another
//! project is not that project's work either: the nearest project wins, and the new one is it.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use duet_schemas::journal::At;
use duet_schemas::project::Date;
use duet_schemas::{Document, Project as ProjectDoc, Version};
use schemars::JsonSchema;

use crate::io::{stdout, tool};
use crate::{Context, PROJECT_FILE, PROJECT_VAR, Project, song};

/// A project's config, under its root.
const CONFIG_FILE: &str = ".duet/duet.toml";
/// Where a named take lives on this machine, under a project's root.
const TAKES_LOCAL_FILE: &str = ".duet/takes.local.toml";

/// What `init` writes as `.duet/duet.toml` when there is none: every line a comment, so it changes
/// nothing until edited.
const CONFIG_TEMPLATE: &str = "\
# This project's duet config. Every line is a comment, so nothing here is set until you
# uncomment it. It is read over your own duet.toml; .duet/duet.local.toml beside it is read
# last, for this machine only. `duet config show` prints every value and where it came from.

# [part]
# end = \"last-bar\"              # or { round-up = 4 }: how a part that names no end gets one

# [play]
# soundfont = \"GeneralUser.sf2\"  # for `duet play`; relative to this file's directory

# [journal]
# segment-size = \"1MiB\"         # the journal rolls to a new segment past this size

# [lanes.shaker-2]               # a drum lane added, or a gm lane changed
# key = 82
# aliases = [\"sh2\"]
";

/// Make a project: `.duet/project.duet`, and `.duet/duet.toml` when there is none. A
/// directory that holds a project already is refused, as is one inside another project unless
/// --nested is given, and the home directory itself. A `.duet/` that holds only config is
/// adopted, its config kept as it is. Nothing else is written: no song, no other directory, no
/// git file.
#[derive(Debug, Args, JsonSchema)]
pub struct InitInput {
    /// The directory to make a project, made when it is missing. Default: the working
    /// directory.
    #[arg(default_value = ".")]
    pub dir: PathBuf,
    /// The project's name for people. Default: the directory's name.
    #[arg(long)]
    pub title: Option<String>,
    /// Make a project inside another one. A nested project is legal, and the nearest wins, but
    /// it is nearly always a mistake.
    #[arg(long)]
    pub nested: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Print the project a command run here works in, and how it was found: named by
    /// DUET_PROJECT, or the nearest directory up whose .duet/ holds project.duet.
    Path(PathInput),
    /// Print the project's document as text, each take with where it was found or that it was
    /// not, and every song in songs/.
    Show(ShowInput),
}

/// Print the project a command run here works in, and how it was found.
#[derive(Debug, Args, JsonSchema)]
pub struct PathInput {}

/// Print the project's document as text, each take with where it was found, and every song.
#[derive(Debug, Args, JsonSchema)]
pub struct ShowInput {}

/// Runs `duet init`.
///
/// # Errors
///
/// When the directory is refused, or a file cannot be written.
pub fn init(_ctx: &Context, input: InitInput) -> Result<()> {
    let InitInput { dir, title, nested } = input;
    let cwd = std::env::current_dir().context("finding the working directory")?;
    let target = absolute(&cwd.join(&dir))?;
    let shown = |rel: &str| {
        if dir == Path::new(".") {
            PathBuf::from(rel)
        } else {
            dir.join(rel)
        }
    };
    let document = target.join(PROJECT_FILE);
    if document.exists() {
        bail!(
            "{} exists: {} is a project already",
            shown(PROJECT_FILE).display(),
            target.display()
        );
    }
    if let Some(home) = home()
        && target == home
    {
        bail!(
            "{} is the home directory: a .duet/ there would make every directory below it a \
             project for config, so init makes none there",
            target.display()
        );
    }
    if !nested && let Some(above) = target.parent().and_then(Project::above) {
        bail!(
            "{} is inside the project {} (its {PROJECT_FILE}): a project nested in another is \
             nearly always a mistake; --nested makes one anyway",
            target.display(),
            above.display()
        );
    }
    let title = title.unwrap_or_else(|| {
        target.file_name().map_or_else(
            || "project".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    let body = ProjectDoc::new(title, today()?, made_by());
    body.check().map_err(anyhow::Error::msg)?;
    let text = Document::new(tool(), body).write()?;

    let duet_dir = target.join(".duet");
    fs::create_dir_all(&duet_dir).with_context(|| duet_dir.display().to_string())?;
    // `create_new`: a project.duet made since the check above is refused, never written over.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&document)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .with_context(|| shown(PROJECT_FILE).display().to_string())?;
    let mut said = format!("wrote {}\n", shown(PROJECT_FILE).display());
    let config = target.join(CONFIG_FILE);
    if config.exists() {
        let _ = writeln!(said, "kept {}, as it was", shown(CONFIG_FILE).display());
    } else {
        fs::write(&config, CONFIG_TEMPLATE).with_context(|| config.display().to_string())?;
        let _ = writeln!(said, "wrote {}", shown(CONFIG_FILE).display());
    }
    stdout(&said)
}

/// Runs `duet project path | show`.
///
/// # Errors
///
/// For `show`, when no project is found, or its document does not read.
pub fn run(ctx: &Context, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::Path(PathInput {}) => stdout(&match ctx.project() {
            Some(project) => format!("project: {}\n{}\n", project.root.display(), project.found),
            None => {
                let cwd = std::env::current_dir().context("finding the working directory")?;
                format!(
                    "project: none\nno {PROJECT_VAR}, and no .duet/ holding project.duet from {} \
                     up; `duet init` makes one\n",
                    cwd.display()
                )
            }
        }),
        ProjectCommand::Show(ShowInput {}) => {
            let Some(project) = ctx.project() else {
                bail!(
                    "no project here: no {PROJECT_VAR}, and no .duet/ holding project.duet from \
                     the working directory up; `duet init` makes one"
                );
            };
            stdout(&shown(project)?)
        }
    }
}

/// What `duet project show` prints.
fn shown(project: &Project) -> Result<String> {
    let path = project.root.join(PROJECT_FILE);
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let doc = Document::<ProjectDoc>::read(&text).with_context(|| path.display().to_string())?;
    let body = &doc.body;
    let local = takes_local(&project.root)?;
    let mut out = String::new();
    let _ = writeln!(out, "project {}: {}", project.root.display(), body.title);
    let _ = writeln!(out, "created {}, by duet {}", body.created, body.duet);
    match &body.defaults {
        None => out.push_str("defaults none\n"),
        Some(d) => {
            let mut set = Vec::new();
            if let Some(tempo) = d.tempo_bpm {
                set.push(format!("tempo {tempo} bpm"));
            }
            if let Some(meter) = &d.meter {
                set.push(format!("meter {meter}"));
            }
            if let Some(key) = &d.key {
                set.push(format!("key {key}"));
            }
            if let Some(tuning) = &d.tuning {
                set.push(format!("tuning {tuning}"));
            }
            if let Some(grid) = &d.grid {
                set.push(format!("grid {grid}"));
            }
            if set.is_empty() {
                out.push_str("defaults none\n");
            } else {
                let _ = writeln!(out, "defaults {}", set.join(", "));
            }
        }
    }
    if body.takes.is_empty() {
        out.push_str("takes none\n");
    } else {
        out.push_str("takes\n");
    }
    for take in &body.takes {
        let copied = Path::new("takes")
            .join(take.name.as_str())
            .join(take.file.as_str());
        let named = local
            .as_ref()
            .and_then(|t| t.get(take.name.as_str()))
            .and_then(|entry| entry.get("path"))
            .and_then(toml::Value::as_str)
            .map(PathBuf::from);
        let found = if project.root.join(&copied).is_file() {
            format!("found at {}", copied.display())
        } else if let Some(at) = named.as_ref().filter(|p| p.is_file()) {
            format!("found at {}, named in {TAKES_LOCAL_FILE}", at.display())
        } else {
            let mut looked = vec![copied.display().to_string()];
            if let Some(at) = &named {
                looked.push(format!("{}, named in {TAKES_LOCAL_FILE}", at.display()));
            }
            format!("not found: looked at {}", looked.join(" and "))
        };
        let t = &take.take;
        let _ = write!(
            out,
            "  {} ({}), {}, {} Hz, {} channel{}, {} frames, added {}",
            take.name,
            take.file,
            t.digest,
            t.rate,
            t.channels,
            if t.channels == 1 { "" } else { "s" },
            t.frames,
            take.added
        );
        if let Some(given) = &take.given {
            let mut set = Vec::new();
            if let Some(tempo) = given.tempo_bpm {
                set.push(format!("tempo {tempo} bpm"));
            }
            if let Some(meter) = &given.meter {
                set.push(format!("meter {meter}"));
            }
            if let Some(key) = &given.key {
                set.push(format!("key {key}"));
            }
            if !set.is_empty() {
                let _ = write!(out, ", given {}", set.join(", "));
            }
        }
        let _ = writeln!(out, "; {found}");
    }
    let songs = song::listed(&project.root)?;
    if songs.is_empty() {
        out.push_str("songs none\n");
    } else {
        out.push_str("songs\n");
    }
    for listed in songs {
        match listed.song {
            Ok(song) => {
                let _ = writeln!(
                    out,
                    "  {}: {} ({}), at {}",
                    listed.folder, song.title, song.id, song.status
                );
            }
            Err(why) => {
                let _ = writeln!(out, "  songs/{}/ {why}", listed.folder);
            }
        }
    }
    Ok(out)
}

/// `.duet/takes.local.toml`, when the project has one.
fn takes_local(root: &Path) -> Result<Option<toml::Table>> {
    let path = root.join(TAKES_LOCAL_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let table = toml::from_str(&text).with_context(|| path.display().to_string())?;
    Ok(Some(table))
}

/// `path` made absolute, with symbolic links resolved as far as it exists: the part that exists
/// is resolved and the rest, not yet made, is laid on it.
fn absolute(path: &Path) -> Result<PathBuf> {
    let mut rest = Vec::new();
    let mut base = path;
    loop {
        if base.exists() {
            let mut out = fs::canonicalize(base).with_context(|| base.display().to_string())?;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        match (base.parent(), base.components().next_back()) {
            (Some(parent), Some(Component::Normal(last))) => {
                rest.push(last.to_owned());
                base = parent;
            }
            (Some(parent), Some(Component::CurDir)) => base = parent,
            _ => bail!("{} cannot be made absolute", path.display()),
        }
    }
}

/// The home directory, resolved, when there is one.
fn home() -> Option<PathBuf> {
    let home = etcetera::home_dir().ok()?;
    Some(fs::canonicalize(&home).unwrap_or(home))
}

/// Today's date, in UTC.
fn today() -> Result<Date> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let stamp = At::from_unix_ms(now).to_string();
    stamp[..10].parse().map_err(anyhow::Error::msg)
}

/// The `<major>.<minor>` of this duet.
fn made_by() -> Version {
    let part = |s: &str| s.parse().unwrap_or(0);
    Version::new(
        part(env!("CARGO_PKG_VERSION_MAJOR")),
        part(env!("CARGO_PKG_VERSION_MINOR")),
    )
}
