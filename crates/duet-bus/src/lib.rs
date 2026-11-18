//! duet's event bus, as far as build task 1 of the bus design takes it: the project's journal,
//! an index over segments, appended under one lock and read from a cursor.
//!
//! The kinds, the entries and the events live in `duet-schemas` ([`duet_schemas::journal`]); this
//! crate writes and reads the files (bus design decisions 16–23):
//!
//! - `journal.duet.jsonl` is the **index**. Each line opens a segment and, after the first, closes
//!   the one before it. The last line is the one open entry, the current segment.
//! - `journal.<stamp>.duet.jsonl` is a **segment**: the entries, one per line.
//!
//! The parts:
//! - **[`Writer`]** appends one entry at a time under an exclusive lock on
//!   `.duet/run/journal.lock`, which covers the index and every segment. Under the lock it:
//!   1. cuts a torn last line of the index and of the current segment;
//!   2. adopts a segment a crashed roll left without its index line;
//!   3. folds in what was appended since it last read;
//!   4. rolls when the current segment is full: the new segment's head first, then its index
//!      line;
//!   5. checks the entry's transition, writes it with one write, and calls `fdatasync`.
//!
//!   The check and the append are one step, so two processes cannot both answer one ask.
//! - **[`read`]** reads the whole journal as a [`ProjectJournal`]: the index, every segment it
//!   names, and what is missing or not named.
//! - **[`Tail`]** follows it from a [`Cursor`] `(segment, seq, id)`, and says [`Polled::Resync`]
//!   when the line at the cursor is no longer the one it named.
//! - **[`Replay`]** folds the segments in index order through each lifecycle's `transition`,
//!   leniently: a transition that fails is skipped, the first in order stands, and it is listed.
//! - **[`Report`]** is what `duet journal check` prints.
//!
//! `seq` is an entry's position within its segment, never stored: a segment's line 2 is `seq` 1.

#![forbid(unsafe_code)]

pub mod ask;
mod check;
mod error;
mod id;
mod project;
mod replay;
mod tail;
mod writer;

pub use check::{CloseDiffers, Duplicate, Report, Torn, Unread};
pub use error::{Error, Result};
pub use id::{fresh_id, now};
pub use project::{Orphan, Place, ProjectJournal, Segment, read};
pub use replay::{AskRecord, Refused, Replay};
pub use tail::{Cursor, Polled, Tail, after};
pub use writer::{Appended, Cut, DEFAULT_SEGMENT_SIZE, Writer};

use std::path::{Path, PathBuf};

use duet_schemas::journal::SegmentStamp;

/// The index's file name, at a project's root.
pub const INDEX: &str = "journal.duet.jsonl";

/// The lock every append takes, under a project's root.
pub const LOCK: &str = ".duet/run/journal.lock";

/// The index of the project at `root`.
#[must_use]
pub fn index_path(root: &Path) -> PathBuf {
    root.join(INDEX)
}

/// The segment `stamp` of the project at `root`.
#[must_use]
pub fn segment_path(root: &Path, stamp: SegmentStamp) -> PathBuf {
    root.join(stamp.file_name())
}

/// The lock of the project at `root`'s journal.
#[must_use]
pub fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK)
}
