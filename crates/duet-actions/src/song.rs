//! `duet song new | show | status`: a song's folder, `songs/<song>/`, under a project's root.
//!
//! What a song is, and what it refuses, is duet-schemas' (`duet_schemas::song`): this reads the
//! folder, hands what it holds to it, and writes what comes back. A song is named by its folder
//! (the take-name rule: lowercase words joined by hyphens); its id is in `song.duet`, so renaming
//! the folder leaves it.
//!
//! **The status gates nothing** (decision 01a1ee66): every command works on a song in any state.
//! Only `duet song status` refuses, and only a move the state machine does not draw.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use duet_core::{Key, TimeSig};
use duet_schemas::journal::Id;
use duet_schemas::project::Name;
use duet_schemas::song::{Composition, Status, distinct_ids, part_names};
use duet_schemas::{AnyDocument, Document, Project, Song, Source};

use crate::Context;
use crate::io::{fresh_id, tool};
use clap::Args;
use schemars::JsonSchema;

/// A song's document in its folder.
const SONG_FILE: &str = "song.duet";
/// The project's document, under its root.
const PROJECT_FILE: &str = ".duet/project.duet";

#[derive(Debug, Subcommand)]
pub enum SongCommand {
    /// Make a song: `songs/<song>/song.duet`, at idea, with a new id, and an empty parts/. Its
    /// composition starts from --tempo, --meter and --key, or else the project's defaults, or
    /// else 120 bpm in 4/4 with no key.
    New(NewInput),
    /// Show a song: its id, title, status and the moves it allows, its composition, and each of
    /// its parts with its id, its source and where it is placed. A placement of a part the song
    /// does not have, two parts with one name or one id, and a file in parts/ that is no part,
    /// are refused by name.
    Show(ShowInput),
    /// Move a song's status: idea → sketch → developing → arranged, and arranged → developing.
    /// Any other move is refused, naming the song's state and the moves it allows, and the song
    /// is left as it was.
    Status(StatusInput),
}

/// Make a song: `songs/<song>/song.duet`, at idea, with a new id, and an empty parts/. Its
/// composition starts from --tempo, --meter and --key, or else the project's defaults, or
/// else 120 bpm in 4/4 with no key.
#[derive(Debug, Args, JsonSchema)]
pub struct NewInput {
    /// The song's name: lowercase words joined by hyphens, such as wayfaring-stranger.
    pub song: Name,
    /// The song's name for people. Default: its name.
    #[arg(long)]
    pub title: Option<String>,
    /// What the song is meant to be, in your words. duet never reads it.
    #[arg(long)]
    pub intent: Option<String>,
    /// The tempo from bar 1, in quarter notes per minute.
    #[arg(long)]
    pub tempo: Option<f64>,
    /// The meter from bar 1, such as 3/4.
    #[arg(long)]
    pub meter: Option<TimeSig>,
    /// The key from bar 1, such as "G minor".
    #[arg(long)]
    pub key: Option<Key>,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

/// Show a song: its id, title, status and the moves it allows, its composition, and each of
/// its parts with its id, its source and where it is placed. A placement of a part the song
/// does not have, two parts with one name or one id, and a file in parts/ that is no part,
/// are refused by name.
#[derive(Debug, Args, JsonSchema)]
pub struct ShowInput {
    /// The song's name.
    pub song: Name,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

/// Move a song's status: idea → sketch → developing → arranged, and arranged → developing.
/// Any other move is refused, naming the song's state and the moves it allows, and the song
/// is left as it was.
#[derive(Debug, Args, JsonSchema)]
pub struct StatusInput {
    /// The song's name.
    pub song: Name,
    /// The state to move to: idea, sketch, developing or arranged.
    pub state: Status,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

pub fn run(ctx: &Context, command: SongCommand) -> Result<()> {
    match command {
        SongCommand::New(NewInput {
            song,
            title,
            intent,
            tempo,
            meter,
            key,
            project,
        }) => {
            let dir = folder(&project, &song);
            let path = dir.join(SONG_FILE);
            if path.exists() {
                bail!("{} exists: the song {song} is made already", path.display());
            }
            let defaults = read_project(&project)?.and_then(|p| p.defaults);
            let composition = Composition::starting(tempo, meter, key, defaults.as_ref());
            let mut body = Song::new(
                fresh_id()?,
                title.unwrap_or_else(|| song.to_string()),
                composition,
            );
            body.intent = intent;
            body.check().map_err(anyhow::Error::msg)?;
            fs::create_dir_all(dir.join("parts")).with_context(|| dir.display().to_string())?;
            write_song(ctx, &path, &Document::new(tool(), body.clone()))?;
            println!(
                "{}: {} ({}), at {}",
                path.display(),
                body.title,
                body.id,
                body.status
            );
            Ok(())
        }
        SongCommand::Show(ShowInput { song, project }) => {
            let dir = folder(&project, &song);
            let doc = read_song(&dir)?;
            let parts = read_parts(&dir)?;
            doc.body
                .check_parts(parts.iter().map(|p| &p.name))
                .map_err(|why| anyhow::anyhow!("{}: {why}", dir.join(SONG_FILE).display()))?;
            distinct_ids(
                parts
                    .iter()
                    .filter_map(|p| p.id.as_ref().map(|id| (&p.name, id))),
            )
            .map_err(|why| anyhow::anyhow!("{}: {why}", dir.join("parts").display()))?;
            print!("{}", shown(&song, &doc.body, &parts));
            Ok(())
        }
        SongCommand::Status(StatusInput {
            song,
            state,
            project,
        }) => {
            let dir = folder(&project, &song);
            let path = dir.join(SONG_FILE);
            let mut doc = read_song(&dir)?;
            let was = doc.body.status;
            doc.body
                .set_status(state)
                .map_err(|why| anyhow::anyhow!("{}: {why}", path.display()))?;
            doc.tool = tool();
            write_song(ctx, &path, &doc)?;
            println!("{}: {was} → {}", path.display(), doc.body.status);
            Ok(())
        }
    }
}

/// `songs/<song>/` under the project's root.
fn folder(project: &Path, song: &Name) -> PathBuf {
    project.join("songs").join(song.as_str())
}

/// The project's document, when the root has one.
fn read_project(project: &Path) -> Result<Option<Project>> {
    let path = project.join(PROJECT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let doc = Document::<Project>::read(&text).with_context(|| path.display().to_string())?;
    Ok(Some(doc.body))
}

fn read_song(dir: &Path) -> Result<Document<Song>> {
    let path = dir.join(SONG_FILE);
    if !path.exists() {
        bail!(
            "{} does not exist: there is no song {} here; `duet song new` makes one",
            path.display(),
            dir.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        );
    }
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Document::read(&text).with_context(|| path.display().to_string())
}

/// One folder under a project's `songs/`, as `duet project show` lists it: its name, and its song,
/// or why it holds none.
pub(crate) struct Listed {
    pub folder: String,
    pub song: std::result::Result<Song, String>,
}

/// Every folder under `root`'s `songs/`, sorted by name, each with the song its `song.duet` holds
/// or why it holds none: no `song.duet`, a name that is no song's name, or a document that does
/// not read. A file directly in `songs/` is no song, and is left out.
pub(crate) fn listed(root: &Path) -> Result<Vec<Listed>> {
    let songs = root.join("songs");
    if !songs.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&songs).with_context(|| songs.display().to_string())? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        let song = match folder.parse::<Name>() {
            Err(why) => Err(format!("is no song's name: {why}")),
            Ok(_) if !entry.path().join(SONG_FILE).is_file() => {
                Err(format!("holds no {SONG_FILE}"))
            }
            Ok(_) => read_song(&entry.path())
                .map(|doc| doc.body)
                .map_err(|e| format!("{e:#}")),
        };
        out.push(Listed { folder, song });
    }
    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    Ok(out)
}

/// Writes the song through a file beside it, renamed over it, so a song is never left half
/// written.
fn write_song(ctx: &Context, path: &Path, doc: &Document<Song>) -> Result<()> {
    let text = doc.write()?;
    let replaced = path.exists();
    let next = path.with_extension("duet.next");
    fs::write(&next, text).with_context(|| next.display().to_string())?;
    fs::rename(&next, path).with_context(|| path.display().to_string())?;
    ctx.wrote(path, replaced)
}

/// One of a song's parts, as `parts/` holds it.
struct SongPart {
    name: Name,
    kind: &'static str,
    /// A part's id; a drum grid states none yet.
    id: Option<Id>,
    source: Option<Source>,
}

/// Every part in the song's `parts/`, by name. A file there that is no part or drum grid is
/// refused, naming it and its kind.
fn read_parts(dir: &Path) -> Result<Vec<SongPart>> {
    let parts_dir = dir.join("parts");
    if !parts_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&parts_dir).with_context(|| parts_dir.display().to_string())? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let mut parts = Vec::new();
    for (name, file) in part_names(files.iter().map(String::as_str))
        .map_err(|why| anyhow::anyhow!("{}: {why}", dir.display()))?
    {
        let path = parts_dir.join(file);
        let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
        let doc = AnyDocument::read(&text).with_context(|| path.display().to_string())?;
        let part = match doc {
            AnyDocument::Part(doc) => SongPart {
                name,
                kind: "part",
                id: Some(doc.body.id),
                source: Some(doc.body.source),
            },
            AnyDocument::DrumGrid(_) => SongPart {
                name,
                kind: "drum-grid",
                id: None,
                source: None,
            },
            other => bail!(
                "{} is a {} document, and a song's parts/ holds parts and drum grids",
                path.display(),
                other.kind()
            ),
        };
        parts.push(part);
    }
    Ok(parts)
}

/// What `duet song show` prints.
fn shown(name: &Name, song: &Song, parts: &[SongPart]) -> String {
    let mut out = String::new();
    let c = &song.composition;
    let _ = writeln!(out, "song {name}: {}", song.title);
    let _ = writeln!(out, "id {}", song.id);
    let moves: Vec<&str> = song.status.moves().iter().map(|s| s.as_str()).collect();
    let _ = writeln!(
        out,
        "status {} (moves to {})",
        song.status,
        moves.join(" or ")
    );
    if let Some(intent) = &song.intent {
        let _ = writeln!(out, "intent {intent}");
    }
    let _ = writeln!(
        out,
        "composition, at {} ticks per quarter",
        c.ticks_per_quarter
    );
    let list = |items: Vec<String>| {
        if items.is_empty() {
            "none".to_owned()
        } else {
            items.join(", ")
        }
    };
    let _ = writeln!(
        out,
        "  tempo {}",
        list(
            c.tempo
                .iter()
                .map(|t| format!("{} bpm at {}", t.bpm, t.at))
                .collect()
        )
    );
    let _ = writeln!(
        out,
        "  meter {}",
        list(
            c.meter
                .iter()
                .map(|m| format!("{} at {}", m.sig, m.at))
                .collect()
        )
    );
    let _ = writeln!(
        out,
        "  key {}",
        list(
            c.key
                .iter()
                .map(|k| format!("{} at {}", k.key, k.at))
                .collect()
        )
    );
    let _ = writeln!(
        out,
        "  sections {}",
        list(
            c.sections
                .iter()
                .map(|s| format!("{} at {}", s.name, s.at))
                .collect()
        )
    );
    let _ = writeln!(
        out,
        "  placements {}",
        list(
            c.placements
                .iter()
                .map(|p| {
                    let mut text = format!("{} at {}", p.part, p.at);
                    if let Some(repeat) = p.repeat {
                        let _ = write!(text, " ×{repeat}");
                    }
                    if p.from.is_some() || p.to.is_some() {
                        let _ = write!(
                            text,
                            " ({} to {})",
                            p.from.map_or_else(|| "its start".into(), |f| f.to_string()),
                            p.to.map_or_else(|| "its end".into(), |t| t.to_string())
                        );
                    }
                    text
                })
                .collect()
        )
    );
    if let Some(end) = c.end {
        let _ = writeln!(out, "  end {end}");
    }
    if parts.is_empty() {
        let _ = writeln!(out, "parts none");
    } else {
        let _ = writeln!(out, "parts");
    }
    for part in parts {
        let placed = c.placements.iter().filter(|p| p.part == part.name).count();
        let placed = match placed {
            0 => "placed nowhere".to_owned(),
            1 => "placed once".to_owned(),
            n => format!("placed {n} times"),
        };
        let id = part
            .id
            .as_ref()
            .map_or_else(|| "states no id".to_owned(), |id| format!("id {id}"));
        let source = part
            .source
            .as_ref()
            .map_or_else(|| "states no source".to_owned(), described);
        let _ = writeln!(
            out,
            "  {} ({}), {id}, {source}, {placed}",
            part.name, part.kind
        );
    }
    out
}

/// A part's source, in words.
fn described(source: &Source) -> String {
    let value = serde_json::to_value(source).unwrap_or_default();
    match source.by() {
        "comped" => {
            let takes: Vec<String> = value["takes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|take| {
                    let digest = take["digest"].as_str().unwrap_or_default();
                    let mut text = take["take"]
                        .as_str()
                        .map_or_else(|| digest.to_owned(), String::from);
                    if let Some(reading) = take["reading"].as_str() {
                        let _ = write!(text, " ({reading})");
                    }
                    text
                })
                .collect();
            let comp = value["comp"]
                .as_str()
                .map_or_else(String::new, |comp| format!(" by the comp {comp}"));
            format!("comped from {}{comp}", takes.join(" and "))
        }
        "generated" => format!(
            "generated by {}",
            value["operation"].as_str().unwrap_or_default()
        ),
        by => format!("by {by}"),
    }
}
