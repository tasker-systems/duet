//! `duet song new | show | status | snapshot | restore`: a song's folder, `songs/<song>/`, under a
//! project's root.
//!
//! What a song is, and what it refuses, is duet-schemas' (`duet_schemas::song`): this reads the
//! folder, hands what it holds to it, and writes what comes back. A song is named by its folder
//! (the take-name rule: lowercase words joined by hyphens); its id is in `song.duet`, so renaming
//! the folder leaves it.
//!
//! **The status gates nothing** (decision 01a1ee66): every command works on a song in any state.
//! Only `duet song status` refuses, and only a move the state machine does not draw.
//!
//! **`song.duet` holds the status, and the journal each change** (the domain model's ruling 14):
//! inside a project, `duet song status` journals a `song.status-set` after the `doc.written` of
//! the song it rewrote. `duet journal check` reports a song whose document and journal disagree.
//!
//! **A snapshot is a folder of copies, made once and never written again** (the songs spec's
//! §3): `snapshots/<date>-<name>/` holds `song.duet` and every part the composition places, each
//! byte for byte, and `snapshot.duet`, which states each copy's digest. Taking one changes no
//! status (ruling 9): it journals a `doc.written` for each file and a `snapshot.taken`. A restore
//! takes a snapshot of the working song first, so nothing is lost by restoring.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use duet_core::{Key, Position, TimeSig};
use duet_schemas::journal::{DocDigest, Id, RelPath, SnapshotTaken, SongStatusSet};
use duet_schemas::project::{Defaults, Name, ProjectTake};
use duet_schemas::snapshot::{Copied, FileDigest, SNAPSHOT_FILE};
use duet_schemas::song::{Composition, Placed, Status, TakeLink, distinct_ids, part_names};
use duet_schemas::{AnyDocument, Document, Project, Snapshot, Song, Source};

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
    /// Link a take of the project to a song, by its name and digest, and say which parts it is
    /// for (--for, repeatable). A take already captured is placed on the song's bars at once,
    /// estimated from its latest capture, naming the analysis it used; one not captured is linked
    /// unplaced, and the next capture places it. A take already linked gains the --for parts
    /// given and changes in nothing else. A take not in the project is refused, naming `duet take
    /// add`, and the song is left as it was.
    Link(LinkInput),
    /// Place a take on a song's bars. --click --bar B --at S says it was played to the song's
    /// click; --bpm, --bar and --at, any of them without --click, set it, each value not given
    /// keeping the link's current one. A take not yet linked is linked first. A link with no
    /// current value for one not given is refused, naming each flag still needed, and the song
    /// is left as it was.
    Place(PlaceInput),
    /// Take a snapshot: copy song.duet and every part the composition places, byte for byte,
    /// into `snapshots/<date>-<name>/`, beside a snapshot.duet stating each copy's digest. A part
    /// placed nowhere is not copied. The song's status is left as it is, and a snapshot is never
    /// written again: one of the same name the same day is refused.
    Snapshot(SnapshotInput),
    /// Restore a snapshot: first take a snapshot of the working song, then copy the snapshot's
    /// song.duet and parts back over the working ones. The song keeps its status. A snapshot
    /// whose files differ from their stated digests is refused, naming them.
    Restore(RestoreInput),
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

/// Link a take of the project to a song, by its name and digest, and say which parts it is for
/// (--for, repeatable). A take already captured is placed on the song's bars at once, estimated
/// from its latest capture, naming the analysis it used; one not captured is linked unplaced,
/// and the next capture places it. A take already linked gains the --for parts given and
/// changes in nothing else. A take not in the project is refused, naming `duet take add`, and
/// the song is left as it was.
#[derive(Debug, Args, JsonSchema)]
pub struct LinkInput {
    /// The song's name.
    pub song: Name,
    /// The take's name in the project.
    pub take: Name,
    /// A part the take is for, by name. The part need not be written yet. Repeatable.
    #[arg(long = "for")]
    pub r#for: Vec<Name>,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

/// Place a take on a song's bars. --click --bar B --at S says it was played to the song's
/// click; --bpm, --bar and --at, any of them without --click, set it, each value not given
/// keeping the link's current one. A take not yet linked is linked first. A link with no
/// current value for one not given is refused, naming each flag still needed, and the song is
/// left as it was.
#[derive(Debug, Args, JsonSchema)]
pub struct PlaceInput {
    /// The song's name.
    pub song: Name,
    /// The take's name in the project.
    pub take: Name,
    /// The take was played to the song's click: its tempo is the composition's. Needs --bar
    /// and --at, and refuses --bpm.
    #[arg(long)]
    pub click: bool,
    /// The place on the song's bars where the take's own start falls, such as 1.1.0.
    #[arg(long)]
    pub bar: Option<Position>,
    /// The seconds into the take at which --bar falls.
    #[arg(long)]
    pub at: Option<f64>,
    /// The take's tempo, in quarter notes per minute. Refused with --click.
    #[arg(long, conflicts_with = "click")]
    pub bpm: Option<f64>,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

/// Take a snapshot: copy song.duet and every part the composition places, byte for byte,
/// into `snapshots/<date>-<name>/`, beside a snapshot.duet stating each copy's digest. A part
/// placed nowhere is not copied. The song's status is left as it is, and a snapshot is never
/// written again: one of the same name the same day is refused.
#[derive(Debug, Args, JsonSchema)]
pub struct SnapshotInput {
    /// The song's name.
    pub song: Name,
    /// The snapshot's name: lowercase words joined by hyphens, such as first-verse. Its
    /// folder is `<date>-<name>`.
    pub name: Name,
    /// What you say of it, in your words. duet never reads it.
    #[arg(long)]
    pub said: Option<String>,
    /// The project's root. Default: the working directory.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

/// Restore a snapshot: first take a snapshot of the working song, then copy the snapshot's
/// song.duet and parts back over the working ones. The song keeps its status. A snapshot
/// whose files differ from their stated digests is refused, naming them.
#[derive(Debug, Args, JsonSchema)]
pub struct RestoreInput {
    /// The song's name.
    pub song: Name,
    /// The snapshot, by its folder under the song's snapshots/, such as 2026-11-20-first-verse.
    pub snapshot: String,
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
            let in_project = read_project(&project)?;
            let defaults = in_project.as_ref().and_then(|p| p.defaults.as_ref());
            let composition = Composition::starting(tempo, meter, key, defaults);
            // In a project, each value says where it came from (the project design's §5).
            if in_project.is_some() {
                for line in starting_from(&composition, tempo, meter, key, defaults) {
                    eprintln!("{line}");
                }
            }
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
            print!("{}", shown_snapshots(&dir)?);
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
            ctx.emit(SongStatusSet::new(
                doc.body.id.clone(),
                song.clone(),
                was,
                doc.body.status,
            ));
            println!("{}: {was} → {}", path.display(), doc.body.status);
            Ok(())
        }
        SongCommand::Link(input) => link(ctx, input),
        SongCommand::Place(input) => place(ctx, input),
        SongCommand::Snapshot(SnapshotInput {
            song,
            name,
            said,
            project,
        }) => {
            let taken = take_snapshot(ctx, &folder(&project, &song), &song, name, said)?;
            println!(
                "{}: {} files, {}",
                taken.dir.display(),
                taken.files,
                taken.status
            );
            Ok(())
        }
        SongCommand::Restore(RestoreInput {
            song,
            snapshot,
            project,
        }) => restore(ctx, &folder(&project, &song), &song, &snapshot),
    }
}

/// The project's take of this name, or a refusal naming the `duet take add` that adds one.
fn project_take(project: &Path, take: &Name) -> Result<ProjectTake> {
    let doc = crate::take::read_project(project)?;
    doc.body.take(take).cloned().ok_or_else(|| {
        let (add, _) = crate::take::add_line(Path::new("<file>"));
        anyhow::anyhow!(
            "{take} is not a take in this project ({}): `{add}` adds one, under the name {take}",
            project.display()
        )
    })
}

/// What [`link_with`] found and did.
struct Linked {
    path: PathBuf,
    /// The link was made now.
    new: bool,
    /// The parts it gained, in the order given.
    gained: Vec<Name>,
    /// Whether `song.duet` was written: only when its content would change.
    wrote: bool,
}

/// Finds the song's link to `take`, or makes it; adds the `parts` it is not yet for; and, when
/// `placed` is given, replaces the link's placement with it. A link already there keeps its
/// placement when `placed` is `None`. The song is checked before it is written, and written only
/// when the bytes would change.
fn link_with(
    ctx: &Context,
    song_dir: &Path,
    take: &ProjectTake,
    parts: &[Name],
    placed: Option<Placed>,
) -> Result<Linked> {
    let path = song_dir.join(SONG_FILE);
    let mut doc = read_song(song_dir)?;
    let was = doc.body.clone();
    let new = doc.body.link(&take.name).is_none();
    if new {
        doc.body
            .takes
            .push(TakeLink::new(take.name.clone(), take.take.digest));
    }
    let Some(link) = doc.body.link_mut(&take.name) else {
        bail!("{}: the link to {} was not made", path.display(), take.name);
    };
    if link.digest != take.take.digest {
        bail!(
            "{}: the song links the take {} by the digest {}, and the project's {} has the digest \
             {}: they are not the same audio",
            path.display(),
            take.name,
            link.digest,
            take.name,
            take.take.digest
        );
    }
    let mut gained = Vec::new();
    for part in parts {
        if !link.r#for.contains(part) {
            link.r#for.push(part.clone());
            gained.push(part.clone());
        }
    }
    if let Some(placed) = placed {
        link.placed = Some(placed);
    }
    doc.body
        .check()
        .map_err(|why| anyhow::anyhow!("{}: {why}", path.display()))?;
    let wrote = doc.body != was;
    if wrote {
        doc.tool = tool();
        write_song(ctx, &path, &doc)?;
    }
    Ok(Linked {
        path,
        new,
        gained,
        wrote,
    })
}

/// Links `take` to the song in `song_dir` (`songs/<song>`), or finds the link, and writes its
/// placement, then `song.duet`. With `placed` `None` an existing link keeps its placement; with
/// `Some` the link's placement is replaced. `song.duet` is written only when its bytes would
/// change. A refusal happens before anything is written.
///
/// # Errors
///
/// When the song does not read or hold, or cannot be written.
pub(crate) fn link_take(
    ctx: &Context,
    song_dir: &Path,
    take: &ProjectTake,
    placed: Option<Placed>,
) -> Result<()> {
    link_with(ctx, song_dir, take, &[], placed).map(|_| ())
}

/// The estimate the take's latest capture gives, and the analysis it came from.
fn captured(project: &Path, take: &ProjectTake) -> Result<Option<(PathBuf, Placed)>> {
    let dir = project
        .join(crate::take::TAKES_DIR)
        .join(take.name.as_str());
    Ok(crate::placing::latest_analysis(&dir, &take.take.digest)?
        .map(|(path, analysis)| (path, crate::placing::estimate(&analysis))))
}

/// The line saying the next capture places a link that is not placed.
fn next_capture(song: &Name, take: &Name) -> String {
    format!(
        "the take {take} is linked to {song} and not placed: the next `duet capture rhythm {take}` \
         (or `melody`) places it on {song}'s bars, or `duet song place {song} {take}` places it"
    )
}

fn link(ctx: &Context, input: LinkInput) -> Result<()> {
    let LinkInput {
        song,
        take,
        r#for: parts,
        project,
    } = input;
    let dir = folder(&project, &song);
    let doc = read_song(&dir)?;
    let found = project_take(&project, &take)?;
    // A link with no placement is placed from the latest capture, if the take has one.
    let unplaced = doc.body.link(&take).is_none_or(|l| l.placed.is_none());
    let estimate = if unplaced {
        captured(&project, &found)?
    } else {
        None
    };
    let linked = link_with(
        ctx,
        &dir,
        &found,
        &parts,
        estimate.as_ref().map(|(_, placed)| *placed),
    )?;
    let path = linked.path.display();
    if !linked.wrote {
        println!("{path}: the take {take} is linked already; nothing changed");
    } else if linked.new {
        println!("{path}: linked the take {take}");
    } else if linked.gained.is_empty() {
        println!("{path}: placed the take {take}");
    } else {
        println!("{path}: the take {take} is now for more parts");
    }
    if let Some((analysis, placed)) = &estimate {
        eprintln!(
            "{} (from {})",
            crate::placing::placed_said(&song, &take, None, placed),
            analysis.display()
        );
    }
    let now = read_song(&dir)?;
    if now.body.link(&take).is_some_and(|l| l.placed.is_none()) {
        eprintln!("{}", next_capture(&song, &take));
    }
    let parts_in_song = read_parts(&dir)?;
    for line in now.body.check_for(parts_in_song.iter().map(|p| &p.name)) {
        eprintln!("{line}");
    }
    Ok(())
}

fn place(ctx: &Context, input: PlaceInput) -> Result<()> {
    let PlaceInput {
        song,
        take,
        click,
        bar,
        at,
        bpm,
        project,
    } = input;
    let dir = folder(&project, &song);
    let path = dir.join(SONG_FILE);
    let doc = read_song(&dir)?;
    let found = project_take(&project, &take)?;
    // What the link holds now, or what linking it would place from the latest capture.
    let current = match doc.body.link(&take) {
        Some(link) => link.placed,
        None => captured(&project, &found)?.map(|(_, placed)| placed),
    };
    let wants = |flags: &[(&str, bool)]| -> String {
        flags
            .iter()
            .filter(|(_, missing)| *missing)
            .map(|(flag, _)| *flag)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let placed = if click {
        let missing = wants(&[("--bar", bar.is_none()), ("--at", at.is_none())]);
        if !missing.is_empty() {
            bail!(
                "{}: --click says where the click's bar falls in the take, and needs {missing} \
                 too; the song is left as it was",
                path.display()
            );
        }
        let (Some(bar), Some(at)) = (bar, at) else {
            unreachable!("both are given: the missing list is empty");
        };
        // A drift measured for a click placement stays: it is the take's clock against the click's.
        let ppm = current.filter(|c| c.by == duet_schemas::song::By::Click);
        Placed::click(bar, at, ppm.and_then(|c| c.clock_ppm))
    } else {
        if bar.is_none() && at.is_none() && bpm.is_none() {
            bail!(
                "{}: nothing to place: give --click with --bar and --at, or any of --bpm, --bar \
                 and --at; the song is left as it was",
                path.display()
            );
        }
        let bar = bar.or_else(|| current.map(|c| c.bar));
        let at = at.or_else(|| current.map(|c| c.at_secs));
        let bpm = bpm.or_else(|| current.and_then(|c| c.bpm));
        let missing = wants(&[
            ("--bar", bar.is_none()),
            ("--at", at.is_none()),
            ("--bpm", bpm.is_none()),
        ]);
        let (Some(bar), Some(at), Some(bpm)) = (bar, at, bpm) else {
            bail!(
                "{}: the take {take} has no placement to keep a value from, so {missing} must \
                 be given too; the song is left as it was",
                path.display()
            );
        };
        Placed::set(bar, at, bpm)
    };
    let is_new = doc.body.link(&take).is_none();
    link_take(ctx, &dir, &found, Some(placed))?;
    if is_new {
        eprintln!("linked the take {take} to {song}");
    }
    println!(
        "{}: {}",
        path.display(),
        crate::placing::placed_said(&song, &take, current.as_ref(), &placed)
    );
    Ok(())
}

/// A snapshot just taken: its folder, how many files it copied, and the song's status, which
/// it left as it was.
struct Taken {
    dir: PathBuf,
    folder: String,
    files: usize,
    status: duet_schemas::song::Status,
}

/// Takes a snapshot of the song in `dir`, named `name`: copies `song.duet` and every part its
/// composition places into `snapshots/<date>-<name>/`, writes `snapshot.duet` beside them, and
/// journals a `doc.written` for each and a `snapshot.taken`. Each file is read once, and the
/// bytes read are the bytes checked, digested and copied.
///
/// The copies are written into a folder beside the snapshot's and renamed into place, so a
/// snapshot is there whole or not at all.
fn take_snapshot(
    ctx: &Context,
    dir: &Path,
    song: &Name,
    name: Name,
    said: Option<String>,
) -> Result<Taken> {
    let song_path = dir.join(SONG_FILE);
    let song_bytes = read_bytes(&song_path)?;
    let doc = std::str::from_utf8(&song_bytes)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(Document::<Song>::read(text)?))
        .with_context(|| song_path.display().to_string())?;
    let parts = read_parts(dir)?;
    doc.body
        .check_parts(parts.iter().map(|p| &p.name))
        .map_err(|why| anyhow::anyhow!("{}: {why}", song_path.display()))?;

    // Every part the composition places, once each, in the order first placed.
    let mut placed: Vec<&SongPart> = Vec::new();
    for placement in &doc.body.composition.placements {
        if placed.iter().all(|p| p.name != placement.part)
            && let Some(part) = parts.iter().find(|p| p.name == placement.part)
        {
            placed.push(part);
        }
    }
    let mut copies: Vec<(RelPath, Vec<u8>)> = vec![(rel(SONG_FILE)?, song_bytes)];
    for part in placed {
        let path = format!("parts/{}", part.file);
        let bytes = read_bytes(&dir.join(&path))?;
        copies.push((rel(&path)?, bytes));
    }

    let mut body = Snapshot::new(
        name,
        crate::project::today()?,
        copies
            .iter()
            .map(|(path, bytes)| Copied::new(path.clone(), FileDigest::of(bytes)))
            .collect(),
    );
    body.said = said;
    body.check().map_err(anyhow::Error::msg)?;
    let snapshots = dir.join("snapshots");
    no_link(project_of(dir), &snapshots)?;
    let name_of = body.folder();
    let target = snapshots.join(&name_of);
    if there(&target) {
        bail!(
            "{} exists: a snapshot is never written again; give this one another name",
            target.display()
        );
    }
    let staging = snapshots.join(format!(".{name_of}.partial"));
    if there(&staging) {
        bail!(
            "{} exists: a snapshot left half written by an earlier run. Nothing reads it; \
             remove it and take the snapshot again",
            staging.display()
        );
    }
    let text = Document::new(tool(), body.clone()).write()?;
    let staged = (|| -> Result<()> {
        fs::create_dir_all(staging.join("parts")).with_context(|| staging.display().to_string())?;
        for (path, bytes) in &copies {
            let to = staging.join(path.as_str());
            fs::write(&to, bytes).with_context(|| to.display().to_string())?;
        }
        let to = staging.join(SNAPSHOT_FILE);
        fs::write(&to, &text).with_context(|| to.display().to_string())?;
        fs::rename(&staging, &target).with_context(|| target.display().to_string())
    })();
    if let Err(e) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    for (path, _) in &copies {
        ctx.wrote(&target.join(path.as_str()), false)?;
    }
    ctx.wrote(&target.join(SNAPSHOT_FILE), false)?;
    let digest = DocDigest::of(serde_json::to_string(&body)?.as_bytes());
    ctx.emit(SnapshotTaken::new(
        doc.body.id.clone(),
        song.clone(),
        name_of.clone(),
        digest,
    ));
    Ok(Taken {
        dir: target,
        folder: name_of,
        files: copies.len(),
        status: doc.body.status,
    })
}

/// Restores the snapshot `snapshot` of the song in `dir`: checks every file it holds against
/// its stated digest, takes a snapshot of the working song, then copies the snapshot's parts and
/// `song.duet` back over the working ones. The song keeps its working status: a restore moves
/// no status, which only `duet song status` does.
fn restore(ctx: &Context, dir: &Path, song: &Name, snapshot: &str) -> Result<()> {
    let snap_dir = dir.join("snapshots").join(snapshot);
    if snapshot.starts_with('.') || snapshot.contains(['/', '\\']) || !snap_dir.is_dir() {
        bail!(
            "{} is no snapshot of {song}: `duet song show {song}` lists its snapshots",
            snap_dir.display()
        );
    }
    no_link(project_of(dir), &snap_dir)?;
    let held = read_snapshot(&snap_dir)?;
    let problems = check_snapshot(&snap_dir, &held)?;
    if !problems.is_empty() {
        bail!(
            "{}: {}; a snapshot is never changed, so this one is not restored",
            snap_dir.display(),
            problems.join("; ")
        );
    }
    let snap_song_path = snap_dir.join(SONG_FILE);
    let snap_song_bytes = read_bytes(&snap_song_path)?;
    let mut snap_song = std::str::from_utf8(&snap_song_bytes)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(Document::<Song>::read(text)?))
        .with_context(|| snap_song_path.display().to_string())?;
    let working = read_song(dir)?;
    for file in &held.files {
        no_link(project_of(dir), &dir.join(file.path.as_str()))?;
    }
    if snap_song.body.id != working.body.id {
        bail!(
            "{} is the song {}, and the working song is {}: a snapshot restores only the song it \
             was taken of",
            snap_song_path.display(),
            snap_song.body.id,
            working.body.id
        );
    }

    // The working song, kept first, under a name that is free.
    let base = format!("before-restoring-{snapshot}");
    let mut kept_name: Name = base
        .parse()
        .map_err(|e| anyhow::anyhow!("{snapshot} is not named as a snapshot is: {e}"))?;
    let today = crate::project::today()?;
    for n in 2.. {
        if !dir
            .join("snapshots")
            .join(duet_schemas::snapshot::folder(today, &kept_name))
            .exists()
        {
            break;
        }
        kept_name = format!("{base}-{n}").parse().map_err(anyhow::Error::msg)?;
    }
    let kept = take_snapshot(
        ctx,
        dir,
        song,
        kept_name,
        Some(format!(
            "the working song, kept before restoring {snapshot}"
        )),
    )?;

    // The snapshot's song, with the working status, then its parts.
    let status = working.body.status;
    let song_path = dir.join(SONG_FILE);
    if snap_song.body.status == status {
        replace(ctx, &song_path, &snap_song_bytes)?;
    } else {
        snap_song.body.status = status;
        snap_song.tool = tool();
        write_song(ctx, &song_path, &snap_song)?;
    }
    let mut restored = vec![SONG_FILE.to_owned()];
    for file in held.files.iter().skip(1) {
        let bytes = read_bytes(&snap_dir.join(file.path.as_str()))?;
        replace(ctx, &dir.join(file.path.as_str()), &bytes)?;
        restored.push(file.path.to_string());
    }
    println!(
        "{}: restored {} over the working song ({}); the song kept its status, {status}. The \
         working song is kept as the snapshot {}",
        dir.display(),
        snapshot,
        restored.join(", "),
        kept.folder
    );
    Ok(())
}

/// Writes `bytes` over `path` through a file beside it, renamed over it, and records it.
fn replace(ctx: &Context, path: &Path, bytes: &[u8]) -> Result<()> {
    let replaced = path.exists();
    let next = path.with_extension("duet.next");
    write_new(&next, bytes)?;
    fs::rename(&next, path).with_context(|| path.display().to_string())?;
    ctx.wrote(path, replaced)
}

/// Writes `bytes` to a new file at `path`. Whatever is there first, a file left by an earlier
/// run or a symbolic link, is removed, never written through: the file is created only if
/// nothing is at `path`, so a link put there between the two is refused, not followed.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    if there(path) {
        fs::remove_file(path).with_context(|| path.display().to_string())?;
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .with_context(|| path.display().to_string())
}

/// The project's root, for a song's folder `songs/<song>/` under it.
fn project_of(dir: &Path) -> &Path {
    dir.parent().and_then(Path::parent).unwrap_or(dir)
}

/// Whether anything is at `path`, a symbolic link included, even one that points nowhere.
fn there(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Refuses `path` when it, or any folder between `dir` and it, is a symbolic link, naming the
/// link: duet writes and restores no file through one, so a link committed to a project cannot
/// send a write outside it. `dir` itself and the folders above it are the owner's, and are not
/// checked; a part of `path` that is not there yet is no link.
fn no_link(dir: &Path, path: &Path) -> Result<()> {
    let Ok(rest) = path.strip_prefix(dir) else {
        bail!("{} is not under {}", path.display(), dir.display());
    };
    let mut at = dir.to_path_buf();
    for part in rest.components() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => bail!(
                "{} is a symbolic link: duet writes and restores no file through one",
                at.display()
            ),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| at.display().to_string()),
        }
    }
    Ok(())
}

/// A path in a song's or a snapshot's folder.
fn rel(path: &str) -> Result<RelPath> {
    path.parse().map_err(anyhow::Error::msg)
}

/// A file's bytes, naming it by its path when it cannot be read.
fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| path.display().to_string())
}

/// A snapshot's `snapshot.duet`.
fn read_snapshot(snap_dir: &Path) -> Result<Snapshot> {
    let path = snap_dir.join(SNAPSHOT_FILE);
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Ok(Document::<Snapshot>::read(&text)
        .with_context(|| path.display().to_string())?
        .body)
}

/// What differs in a snapshot from what its `snapshot.duet` states, file by file: a file that
/// differs from its stated digest, one that is missing, one it holds but does not state, and a
/// symbolic link, which a snapshot never holds. Empty when the snapshot is as taken.
///
/// No link is followed, so a link to a folder above it cannot make the walk endless.
fn check_snapshot(snap_dir: &Path, held: &Snapshot) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for file in &held.files {
        let path = snap_dir.join(file.path.as_str());
        if no_link(snap_dir, &path).is_err() {
            problems.push(format!("{} is a symbolic link", file.path));
            continue;
        }
        match fs::read(&path) {
            Ok(bytes) if FileDigest::of(&bytes) == file.digest => {}
            Ok(_) => problems.push(format!("{} differs from its stated digest", file.path)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                problems.push(format!("{} is missing", file.path));
            }
            Err(e) => return Err(e).with_context(|| path.display().to_string()),
        }
    }
    let mut on_disk = Vec::new();
    let mut stack = vec![snap_dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in fs::read_dir(&at).with_context(|| at.display().to_string())? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(path);
            } else if let Ok(rest) = path.strip_prefix(snap_dir) {
                let rest: Vec<String> = rest
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                on_disk.push((rest.join("/"), kind.is_symlink()));
            }
        }
    }
    on_disk.sort();
    for (path, link) in on_disk {
        let stated = held.files.iter().any(|f| f.path.as_str() == path);
        if link && !stated {
            problems.push(format!("{path} is a symbolic link"));
        } else if path != SNAPSHOT_FILE && !stated {
            problems.push(format!("{path} is in it, and it does not state it"));
        }
    }
    Ok(problems)
}

/// What `duet song show` prints of the song's snapshots: each by its folder, with its name and
/// how many files it holds when it is as taken, or else everything in it that differs from
/// what its `snapshot.duet` states, naming each file.
fn shown_snapshots(dir: &Path) -> Result<String> {
    let snapshots = dir.join("snapshots");
    let mut folders = Vec::new();
    if snapshots.is_dir() {
        for entry in fs::read_dir(&snapshots).with_context(|| snapshots.display().to_string())? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() || kind.is_symlink() {
                folders.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    folders.sort();
    let mut out = String::new();
    if folders.is_empty() {
        let _ = writeln!(out, "snapshots none");
        return Ok(out);
    }
    let _ = writeln!(out, "snapshots");
    for folder in folders {
        let snap_dir = snapshots.join(&folder);
        if fs::symlink_metadata(&snap_dir).is_ok_and(|m| m.file_type().is_symlink()) {
            let _ = writeln!(
                out,
                "  {folder}: is a symbolic link, and a snapshot is a folder of its own"
            );
            continue;
        }
        if folder.starts_with('.') {
            let _ = writeln!(
                out,
                "  {folder}: a snapshot left half written; nothing reads it, and it can be removed"
            );
            continue;
        }
        let held = match read_snapshot(&snap_dir) {
            Ok(held) => held,
            Err(e) => {
                let _ = writeln!(out, "  {folder}: {SNAPSHOT_FILE} does not read: {e:#}");
                continue;
            }
        };
        let problems = check_snapshot(&snap_dir, &held)?;
        if problems.is_empty() {
            let _ = writeln!(
                out,
                "  {folder}: {}, {} files, as taken{}",
                held.name,
                held.files.len(),
                held.said
                    .as_ref()
                    .map_or_else(String::new, |said| format!(", said {said:?}"))
            );
        } else {
            for problem in problems {
                let _ = writeln!(out, "  {folder}: {problem}");
            }
        }
    }
    Ok(out)
}

/// `songs/<song>/` under the project's root.
fn folder(project: &Path, song: &Name) -> PathBuf {
    project.join("songs").join(song.as_str())
}

/// Where each value a new song's composition starts from came from: its flag, the project's
/// defaults, or duet's own default. One line for each of tempo, meter and key.
fn starting_from(
    composition: &Composition,
    tempo: Option<f64>,
    meter: Option<TimeSig>,
    key: Option<Key>,
    defaults: Option<&Defaults>,
) -> Vec<String> {
    let from = |given: bool, defaulted: bool| {
        if given {
            "given".to_owned()
        } else if defaulted {
            format!("from the project's defaults in {PROJECT_FILE}")
        } else {
            "duet's default".to_owned()
        }
    };
    let bpm = composition.tempo.first().map_or(0.0, |t| t.bpm);
    let bpm = if (bpm - bpm.round()).abs() < 1e-9 {
        format!("{bpm:.0}")
    } else {
        format!("{bpm}")
    };
    let mut out = vec![format!(
        "tempo: {bpm} bpm, {}",
        from(
            tempo.is_some(),
            defaults.is_some_and(|d| d.tempo_bpm.is_some())
        )
    )];
    if let Some(first) = composition.meter.first() {
        out.push(format!(
            "meter: {}, {}",
            first.sig,
            from(meter.is_some(), defaults.is_some_and(|d| d.meter.is_some()))
        ));
    }
    out.push(match composition.key.first() {
        Some(first) => format!(
            "key: {}, {}",
            first.key,
            from(key.is_some(), defaults.is_some_and(|d| d.key.is_some()))
        ),
        None => "key: none, given or in the project's defaults".to_owned(),
    });
    out
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

/// The song in `dir` (`songs/<song>`), from its `song.duet`; refused, naming `duet song new`, when
/// there is none.
pub(crate) fn read_song(dir: &Path) -> Result<Document<Song>> {
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
#[derive(Debug)]
pub struct Listed {
    pub folder: String,
    pub song: std::result::Result<Song, String>,
}

/// Every folder under `root`'s `songs/`, sorted by name, each with the song its `song.duet` holds
/// or why it holds none: no `song.duet`, a name that is no song's name, or a document that does
/// not read. A file directly in `songs/` is no song, and is left out.
///
/// # Errors
///
/// When `songs/` or a folder in it cannot be read.
pub fn listed(root: &Path) -> Result<Vec<Listed>> {
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
    write_new(&next, text.as_bytes())?;
    fs::rename(&next, path).with_context(|| path.display().to_string())?;
    ctx.wrote(path, replaced)
}

/// One of a song's parts, as `parts/` holds it.
struct SongPart {
    name: Name,
    /// Its file in `parts/`.
    file: String,
    kind: &'static str,
    /// A part's id; a drum grid states none yet.
    id: Option<Id>,
    source: Option<Source>,
}

/// Every part in the song's `parts/`, by name. A file there that is no part or drum grid is
/// refused, naming it and its kind.
/// The song in `dir` and every part its composition places, each read as a part (a drum grid
/// rendered as one, as `duet drums render` does), for `duet play`. A placement of a part the song
/// does not have, and two parts with one name or one id, are refused by name, as `duet song show`
/// refuses them.
pub fn read_placed(dir: &Path) -> Result<(Song, Vec<(Name, duet_core::Part)>)> {
    let doc = read_song(dir)?;
    let parts = read_parts(dir)?;
    doc.body
        .check_parts(parts.iter().map(|p| &p.name))
        .map_err(|why| anyhow::anyhow!("{}: {why}", dir.join(SONG_FILE).display()))?;
    distinct_ids(
        parts
            .iter()
            .filter_map(|p| p.id.as_ref().map(|id| (&p.name, id))),
    )
    .map_err(|why| anyhow::anyhow!("{}: {why}", dir.join("parts").display()))?;
    let mut placed = Vec::new();
    for placement in &doc.body.composition.placements {
        if placed.iter().any(|(name, _)| *name == placement.part) {
            continue;
        }
        let Some(part) = parts.iter().find(|p| p.name == placement.part) else {
            continue;
        };
        let path = dir.join("parts").join(&part.file);
        let read = match crate::io::read_any(&path, crate::io::Format::Duet, "duet play reads")? {
            crate::io::Loaded::Part(part, _) => part,
            crate::io::Loaded::Grid(grid, _) => {
                grid.to_part().with_context(|| path.display().to_string())?
            }
        };
        placed.push((part.name.clone(), read));
    }
    Ok((doc.body, placed))
}

/// The song folder `input` names for `duet play`, if it names one: a folder holding song.duet,
/// a song.duet itself, or, in a project, a song's name. A folder that holds no song.duet, and a
/// name the project has no song of, are refused by name.
pub fn folder_for_play(project: Option<&Path>, input: &Path) -> Result<Option<PathBuf>> {
    if input.is_dir() {
        if input.join(SONG_FILE).is_file() {
            return Ok(Some(input.to_path_buf()));
        }
        bail!(
            "{} is a folder with no {SONG_FILE}: duet play plays a part, or a song's folder",
            input.display()
        );
    }
    if input.file_name().is_some_and(|n| n == SONG_FILE) && input.is_file() {
        return Ok(Some(
            input
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        ));
    }
    if input.exists() {
        return Ok(None);
    }
    let (Some(root), Some(name)) = (project, input.to_str().and_then(|s| s.parse::<Name>().ok()))
    else {
        return Ok(None);
    };
    let dir = folder(root, &name);
    if dir.join(SONG_FILE).is_file() {
        return Ok(Some(dir));
    }
    let songs: Vec<String> = listed(root)?
        .into_iter()
        .filter(|l| l.song.is_ok())
        .map(|l| l.folder)
        .collect();
    bail!(
        "the project at {} has no song {name}: {} does not exist. {}",
        root.display(),
        dir.join(SONG_FILE).display(),
        if songs.is_empty() {
            "It has no songs; `duet song new` makes one".to_owned()
        } else {
            format!("Its songs are {}", songs.join(", "))
        }
    )
}

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
                file: file.to_owned(),
                kind: "part",
                id: Some(doc.body.id),
                source: Some(doc.body.source),
            },
            AnyDocument::DrumGrid(_) => SongPart {
                name,
                file: file.to_owned(),
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
    if song.takes.is_empty() {
        let _ = writeln!(out, "takes none");
    } else {
        let _ = writeln!(out, "takes");
    }
    for link in &song.takes {
        let parts = if link.r#for.is_empty() {
            "none".to_owned()
        } else {
            link.r#for
                .iter()
                .map(Name::as_str)
                .collect::<Vec<_>>()
                .join(" and ")
        };
        let placed = link
            .placed
            .as_ref()
            .map_or_else(|| "unplaced".to_owned(), crate::placing::describe);
        let _ = writeln!(out, "  {}, for {parts}, {placed}", link.take);
    }
    for line in song.check_for(parts.iter().map(|p| &p.name)) {
        let _ = writeln!(out, "  {line}");
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
