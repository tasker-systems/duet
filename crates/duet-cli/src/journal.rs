//! `duet journal show | check`: a project's journal, its index and segments, read as every duet
//! reads it.
//!
//! Both only read. Neither takes the journal's lock: a reader never sees a torn last line as an
//! entry, and nothing it does changes a file.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Subcommand;
use duet_bus::{INDEX, Replay, Report, SongOnDisk};
use duet_schemas::journal::Body;

#[derive(Subcommand)]
pub enum JournalCommand {
    /// Print every entry, one per line, in replay order: its segment and seq, when, who and
    /// through which face, its event and version, and its body. An entry of an event this duet
    /// does not read is shown as kept, and one that replay skips says so.
    Show {
        /// The project's root, or its journal.duet.jsonl. Default: the working directory.
        #[arg(default_value = ".")]
        project: PathBuf,
    },
    /// Check the journal: list every entry of an event this duet does not read, which is kept
    /// as it is; every transition replay skips, such as a second answer to one ask, the first
    /// in order standing; every song status move replay skips, such as two clones' moves of one
    /// song, merged; every song whose status in its song.duet is not where the journal's moves
    /// leave it, naming both, and changing neither; every id that appears twice; and every
    /// segment the index names that is missing, or that holds entries and is not named. Exits 1
    /// when anything is wrong. A torn last line, a merge's traces in the index, and a segment a
    /// crashed roll left are stated, not wrong.
    Check {
        /// The project's root, or its journal.duet.jsonl. Default: the working directory.
        #[arg(default_value = ".")]
        project: PathBuf,
    },
}

/// Every song under the project's `songs/` whose `song.duet` reads, with its id and status, for
/// the check to compare with the journal. One that does not read is left to `duet song show`,
/// which names why.
fn songs(root: &Path) -> Result<Vec<SongOnDisk>> {
    Ok(duet_actions::song::listed(root)?
        .into_iter()
        .filter_map(|listed| {
            let song = listed.song.ok()?;
            Some(SongOnDisk::new(listed.folder, song.id, song.status))
        })
        .collect())
}

/// The project's root: `path`, or the directory holding it when it names the index.
fn root_of(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|name| name == INDEX) {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    } else {
        path.to_path_buf()
    }
}

pub fn run(command: JournalCommand) -> Result<()> {
    match command {
        JournalCommand::Show { project } => {
            let read = duet_bus::read(&root_of(&project))?;
            let replay = Replay::of(&read);
            let mut refused: HashMap<_, _> = replay
                .refused
                .iter()
                .map(|r| (r.place, r.refusal.to_string()))
                .collect();
            refused.extend(
                replay
                    .refused_moves
                    .iter()
                    .map(|r| (r.place, r.refusal.to_string())),
            );
            let mut out = String::new();
            for (place, line) in read.entries() {
                let body = match &line.body {
                    Body::Read(event) => serde_json::to_string(event)?,
                    _ => serde_json::from_str::<serde_json::Value>(&line.text)?["body"].to_string(),
                };
                write!(
                    out,
                    "{} {:>4}  {}  {}/{}  {} {}  {body}",
                    place.segment,
                    place.seq,
                    line.at,
                    line.by.who,
                    line.by.face,
                    line.event,
                    line.v
                )?;
                if let Body::Skipped(why) = &line.body {
                    write!(out, "  (kept, not read: {why})")?;
                }
                if let Some(r) = refused.get(&place) {
                    write!(out, "  (skipped on replay: {r})")?;
                }
                out.push('\n');
            }
            duet_actions::io::stdout(&out)
        }
        JournalCommand::Check { project } => {
            let root = root_of(&project);
            let report = Report::with_songs(&duet_bus::read(&root)?, &songs(&root)?);
            duet_actions::io::stdout(&format!("{report}\n"))?;
            if report.is_wrong() {
                bail!("{}: the check found something wrong", project.display());
            }
            Ok(())
        }
    }
}
