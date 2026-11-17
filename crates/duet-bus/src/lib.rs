//! duet's event bus, as far as build task 1 of the bus design takes it: the project's journal
//! (`journal.duet.jsonl`), appended under one lock and read from a cursor.
//!
//! The journal's kind, its entries and its events live in `duet-schemas`
//! ([`duet_schemas::journal`]); this crate writes and reads the file.
//!
//! - **[`Writer`]** appends one entry at a time under an exclusive lock on
//!   `.duet/run/journal.lock`. Under the lock it cuts a torn last line back to the last newline,
//!   folds in every line appended since it last read, checks the entry's transition, writes one
//!   whole line and calls `fdatasync`. The check and the append are one step, so two processes
//!   cannot both answer one ask (bus decision 13).
//! - **[`Tail`]** follows the journal from a [`Cursor`] `(seq, id)`, and says [`Polled::Resync`]
//!   when the line at the cursor is no longer the one it named.
//! - **[`Replay`]** folds the journal through each lifecycle's `transition`. Replay is lenient: a
//!   transition that fails is skipped, the first in file order stands, and it is listed (bus
//!   decision 14). Only the ask's lifecycle ([`ask`]) is in the journal.
//! - **[`Report`]** is what `duet journal check` prints: every entry kept unread, every
//!   transition replay skipped, and every id that appears twice.
//!
//! `seq` is an entry's position, never stored: the first entry (line 2) is `seq` 1.

#![forbid(unsafe_code)]

pub mod ask;
mod check;
mod error;
mod id;
mod replay;
mod tail;
mod writer;

pub use check::{Duplicate, Report, Unread};
pub use error::{Error, Result};
pub use id::{fresh_id, now};
pub use replay::{AskRecord, Refused, Replay};
pub use tail::{Cursor, Polled, Tail, after};
pub use writer::{Appended, Cut, Writer};

use std::path::{Path, PathBuf};

/// The journal's file name, at a project's root.
pub const JOURNAL: &str = "journal.duet.jsonl";

/// The lock every append takes, under a project's root.
pub const LOCK: &str = ".duet/run/journal.lock";

/// The journal of the project at `root`.
#[must_use]
pub fn journal_path(root: &Path) -> PathBuf {
    root.join(JOURNAL)
}

/// The lock of the project at `root`'s journal.
#[must_use]
pub fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK)
}

/// Reads the journal at `path`.
///
/// # Errors
///
/// [`Error::Io`] when it cannot be read, and [`Error::Journal`] when it does not read as a
/// journal, naming the line.
pub fn read(path: &Path) -> Result<duet_schemas::Journal> {
    let text = std::fs::read_to_string(path).map_err(Error::io(path))?;
    duet_schemas::Journal::read(&text).map_err(Error::journal(path))
}
