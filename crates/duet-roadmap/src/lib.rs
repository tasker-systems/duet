//! Draws duet's roadmap snapshot from the Temper records.
//!
//! A snapshot is a new `research` resource in `+making-music/duet-dev`, tagged
//! `roadmap-snapshot`. It places each open goal in one lane (Now, Next or Later) by the work that
//! advances it, counts that work, groups goals by their workstream, and says what it leaves out.
//! The rules are the baseline's ROAD-1 to ROAD-5, read through the term mapping in `CLAUDE.md`.
//!
//! # The recipe
//!
//! [`RECIPE`] is the whole rule, and every snapshot carries it, so a reader holding the records
//! can draw the same lanes and counts by hand. In short:
//!
//! 1. Read every goal in `+making-music/duet-dev` and every task in
//!    `+making-music/duet-dev-workflows`, each listing whole (`--all`). A listing that says it is
//!    truncated, or returns fewer rows than its total, stops the draw.
//! 2. Read each task's edges. Its goal is the peer of its outgoing `advances` edge. Every task is
//!    read the same way, so a task without the edge is counted, not dropped.
//! 3. Place and count each open goal (`active` or `paused`) from the tasks that advance it.
//!
//! # What the tool never does
//!
//! - **It never rewrites a snapshot.** Its one write is [`Call::CreateSnapshot`]; there is no
//!   call that updates, moves or deletes, so no code path here can.
//! - **It never reads an unreachable source as an empty roadmap.** Any failed, unparseable or
//!   truncated read is an [`Error`], raised before anything is written.

#![forbid(unsafe_code)]

mod draw;
mod read;
mod render;
mod temper;
mod time;

use std::time::SystemTime;

pub use draw::{Counts, GoalLine, Group, Lane, Snapshot, TaskRef};
pub use read::{Goal, Records, Task};
pub use render::RECIPE;
pub use temper::{Call, Temper, TemperCli};
pub use time::{parse_utc, utc_date, utc_time};

/// Where the goals live.
pub const GOALS_CONTEXT: &str = "+making-music/duet-dev";
/// Where the work items live.
pub const TASKS_CONTEXT: &str = "+making-music/duet-dev-workflows";
/// Where snapshots are written.
pub const SNAPSHOT_CONTEXT: &str = GOALS_CONTEXT;
/// The tag every snapshot carries.
pub const SNAPSHOT_TAG: &str = "roadmap-snapshot";

/// The values of `open_meta.workstream`, in the order `CLAUDE.md` writes them (ROAD-3). The
/// snapshot's groups follow this order.
pub const WORKSTREAMS: [&str; 6] = [
    "songs-and-comps",
    "bus-and-project",
    "workbench",
    "capture-and-readings",
    "models-and-polyphony",
    "ci-and-release",
];

/// Why no snapshot was drawn. Every variant is raised before anything is written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `temper` could not be run, or exited non-zero.
    #[error("could not run `{command}`: {detail}")]
    Unavailable {
        /// The command, as it would be typed.
        command: String,
        /// What went wrong.
        detail: String,
    },
    /// A listing came back partial.
    #[error(
        "the {what} listing (`{command}`) was truncated: {detail}; refusing to draw from a partial page"
    )]
    Truncated {
        /// Which listing.
        what: &'static str,
        /// The command, as it would be typed.
        command: String,
        /// How it was partial.
        detail: String,
    },
    /// An answer did not have the shape the tool reads.
    #[error("`{command}` answered in a shape this tool does not read: {detail}")]
    Shape {
        /// The command, as it would be typed.
        command: String,
        /// What was missing or unexpected.
        detail: String,
    },
    /// A record holds a value outside the rule, such as a goal status the baseline does not name.
    #[error("{detail}")]
    OutOfRule {
        /// What was found, and where.
        detail: String,
    },
    /// The create failed or answered oddly, so whether the snapshot landed is not known. This is
    /// the one error raised after a write was attempted.
    #[error(
        "the create (`{command}`) did not report a new resource: {detail}; it may still have landed, \
         so look for a new `roadmap-snapshot` in +making-music/duet-dev before drawing again"
    )]
    WriteUncertain {
        /// The command, as it would be typed.
        command: String,
        /// What went wrong.
        detail: String,
    },
}

impl Error {
    /// Whether a write may have happened. Only [`Error::WriteUncertain`] says so; every other
    /// error is raised before the create is attempted.
    pub fn may_have_written(&self) -> bool {
        matches!(self, Error::WriteUncertain { .. })
    }
}

/// Whether to write the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Draw and return it; write nothing.
    DryRun,
    /// Draw it and create it as a new resource.
    Write,
}

/// A drawn snapshot.
#[derive(Debug, Clone)]
pub struct Drawn {
    /// What was computed.
    pub snapshot: Snapshot,
    /// The resource's title.
    pub title: String,
    /// The resource's body, markdown.
    pub body: String,
    /// The resource's open metadata.
    pub open_meta: serde_json::Value,
    /// The id of the resource created, in [`Mode::Write`].
    pub created: Option<String>,
}

/// Read the records, draw the snapshot, and in [`Mode::Write`] create it.
///
/// `now` is read once before the first read and once after the last, to state when the records
/// were read. Nothing else in the output depends on it.
pub fn draw_snapshot(
    temper: &mut dyn Temper,
    now: &mut dyn FnMut() -> SystemTime,
    mode: Mode,
) -> Result<Drawn, Error> {
    let started = now();
    let records = read::read(temper)?;
    let finished = now();
    let snapshot = draw::draw(&records);
    let rendered = render::render(&snapshot, &records, started, finished);
    let created = match mode {
        Mode::DryRun => None,
        Mode::Write => Some(temper::create(
            temper,
            Call::CreateSnapshot {
                title: rendered.title.clone(),
                body: rendered.body.clone(),
                open_meta: rendered.open_meta.clone(),
            },
        )?),
    };
    Ok(Drawn {
        snapshot,
        title: rendered.title,
        body: rendered.body,
        open_meta: rendered.open_meta,
        created,
    })
}
