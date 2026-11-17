//! What `duet journal check` reports.

use std::collections::HashMap;
use std::fmt;

use duet_schemas::Journal;
use duet_schemas::journal::{Body, Id, Skip};
use duet_schemas::{Kind, Version};

use crate::{Refused, Replay};

/// An entry kept verbatim and skipped, because this build does not read its event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unread {
    pub seq: u64,
    pub id: Id,
    pub event: String,
    pub v: Version,
    pub why: Skip,
}

/// An id that appears twice: two lines claim to be one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Duplicate {
    pub seq: u64,
    /// The `seq` of the first line with this id.
    pub first: u64,
    pub id: Id,
}

/// A journal checked: what was kept unread, what replay skipped, and what is wrong.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Report {
    pub project: String,
    pub version: Version,
    /// Every whole entry.
    pub entries: usize,
    /// Entries kept verbatim and skipped: stated, and not wrong.
    pub unread: Vec<Unread>,
    /// Transitions replay skipped: wrong, and never fatal.
    pub refused: Vec<Refused>,
    /// Ids that appear twice: wrong.
    pub duplicates: Vec<Duplicate>,
    /// The length of a torn last line, which the next append cuts: stated, and not wrong.
    pub torn: Option<usize>,
    /// The asks still open.
    pub open: usize,
}

impl Report {
    /// Checks `journal`.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        let mut unread = Vec::new();
        let mut duplicates = Vec::new();
        let mut seen: HashMap<&Id, u64> = HashMap::new();
        for (seq, line) in (1..).zip(&journal.lines) {
            if let Body::Skipped(why) = &line.body {
                unread.push(Unread {
                    seq,
                    id: line.id.clone(),
                    event: line.event.clone(),
                    v: line.v,
                    why: why.clone(),
                });
            }
            if let Some(first) = seen.get(&line.id) {
                duplicates.push(Duplicate {
                    seq,
                    first: *first,
                    id: line.id.clone(),
                });
            } else {
                seen.insert(&line.id, seq);
            }
        }
        let replay = Replay::of(&journal.lines);
        Self {
            project: journal.head.project.clone(),
            version: duet_schemas::JournalHead::VERSION,
            entries: journal.lines.len(),
            unread,
            refused: replay.refused.clone(),
            duplicates,
            torn: journal.torn.as_ref().map(String::len),
            open: replay.open().len(),
        }
    }

    /// Whether anything is wrong: a transition replay skipped, or an id twice. An entry kept
    /// unread, or a torn last line, is stated but not wrong.
    #[must_use]
    pub const fn is_wrong(&self) -> bool {
        !self.refused.is_empty() || !self.duplicates.is_empty()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "journal {} of {}: {}, {} open",
            self.version,
            self.project,
            plural(self.entries, "entry", "entries"),
            plural(self.open, "ask", "asks"),
        )?;
        if !self.unread.is_empty() {
            writeln!(
                f,
                "{} of a kind this duet does not read, kept as they are:",
                plural(self.unread.len(), "entry", "entries")
            )?;
            for u in &self.unread {
                writeln!(
                    f,
                    "  seq {} ({}): {} {}, {}",
                    u.seq, u.id, u.event, u.v, u.why
                )?;
            }
        }
        if let Some(len) = self.torn {
            writeln!(
                f,
                "the last line is torn ({}, no newline): a write that never finished; the next \
                 append cuts it",
                plural(len, "byte", "bytes")
            )?;
        }
        if !self.refused.is_empty() {
            writeln!(
                f,
                "{} skipped on replay, the first in file order standing:",
                plural(self.refused.len(), "transition", "transitions")
            )?;
            for r in &self.refused {
                let stands = r
                    .stands
                    .map(|s| format!("; seq {s} stands"))
                    .unwrap_or_default();
                writeln!(
                    f,
                    "  seq {} ({}): {} of ask {}, refused {}{stands}",
                    r.seq, r.id, r.event, r.ask, r.refusal
                )?;
            }
        }
        if !self.duplicates.is_empty() {
            writeln!(
                f,
                "{} that another line has too:",
                plural(self.duplicates.len(), "id", "ids")
            )?;
        }
        for d in &self.duplicates {
            writeln!(f, "  seq {} has the id of seq {}: {}", d.seq, d.first, d.id)?;
        }
        if self.is_wrong() {
            write!(
                f,
                "{} wrong",
                plural(
                    self.refused.len() + self.duplicates.len(),
                    "thing",
                    "things"
                )
            )
        } else {
            write!(f, "nothing wrong")
        }
    }
}
