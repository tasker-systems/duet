//! What `duet journal check` reports.

use std::collections::HashMap;
use std::fmt;

use duet_schemas::journal::{Body, Closes, Id, SegmentStamp, Skip};
use duet_schemas::song::Status;
use duet_schemas::{IndexHead, Kind, Version};

use crate::{Orphan, Place, ProjectJournal, Refused, RefusedMove, Replay};

/// An entry kept verbatim and skipped, because this build does not read its event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unread {
    pub place: Place,
    pub id: Id,
    pub event: String,
    pub v: Version,
    pub why: Skip,
}

/// An id that appears twice: two lines claim to be one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Duplicate {
    pub place: Place,
    /// Where the first line with this id is.
    pub first: Place,
    pub id: Id,
}

/// A torn last line, which the next append cuts if it is the index's or the current segment's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Torn {
    pub file: String,
    pub bytes: usize,
}

/// An index line's closing summary that its segment's file no longer matches: the segment was
/// appended to after it was closed, by a merge. The file stays the truth.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CloseDiffers {
    pub segment: SegmentStamp,
    /// What the closing line says: entries.
    pub says: u64,
    /// What the file holds.
    pub holds: u64,
}

/// A song as its document has it: its name (its folder's), its id and its status, as
/// `songs/<song>/song.duet` states them. What [`Report::with_songs`] compares the journal with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SongOnDisk {
    pub name: String,
    pub id: Id,
    pub status: Status,
}

impl SongOnDisk {
    #[must_use]
    pub fn new(name: impl Into<String>, id: Id, status: Status) -> Self {
        Self {
            name: name.into(),
            id,
            status,
        }
    }
}

/// A song whose status in its `song.duet` is not where the journal's moves leave it (the
/// domain model's ruling 14). Both sides are named, and neither is changed: the check reports,
/// and never repairs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusDiffers {
    pub song: String,
    pub id: Id,
    /// What `song.duet` says.
    pub document: Status,
    /// Where the journal's moves leave it.
    pub journal: Status,
    /// The `song.status-set` that stands, the last that replay folded, or `None` when the
    /// journal has no move for the song and so leaves it where `duet song new` makes every
    /// song: at idea.
    pub set: Option<Place>,
}

/// A journal checked: what was kept unread, what replay skipped, and what is wrong.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Report {
    pub project: String,
    pub version: Version,
    pub segments: usize,
    /// Every whole entry, across every segment.
    pub entries: usize,
    /// Entries kept verbatim and skipped: stated, and not wrong.
    pub unread: Vec<Unread>,
    /// Torn last lines: stated, and not wrong.
    pub torn: Vec<Torn>,
    /// Segments closed by more than one index line: a merge of two clones that each rolled.
    /// Stated, and not wrong.
    pub closed_twice: Vec<SegmentStamp>,
    /// Closing summaries their segment no longer matches: stated, and not wrong.
    pub close_differs: Vec<CloseDiffers>,
    /// Segments no index line's summary closes, other than the current one: after a merge where
    /// both clones rolled, the one the other clone's line closed implicitly. They have no songs,
    /// parts or takes lists, so per-song queries read them whole. Stated, and not wrong.
    pub unsummarised: Vec<SegmentStamp>,
    /// Segment files with only a head and no index line: the next append adopts the newest.
    /// Stated, and not wrong.
    pub adoptable: Vec<SegmentStamp>,
    /// Transitions replay skipped: wrong, and never fatal.
    pub refused: Vec<Refused>,
    /// Status moves replay skipped: wrong, and never fatal.
    pub refused_moves: Vec<RefusedMove>,
    /// Songs whose `song.duet` disagrees with the journal on their status: wrong, and never
    /// repaired.
    pub status_differs: Vec<StatusDiffers>,
    /// Ids that appear twice: wrong.
    pub duplicates: Vec<Duplicate>,
    /// Segments the index names whose file is missing: wrong.
    pub missing: Vec<SegmentStamp>,
    /// Segment files holding entries that no index line names: wrong.
    pub unnamed: Vec<Orphan>,
    /// The asks still open.
    pub open: usize,
}

impl Report {
    /// Checks `journal`, and compares each song in `songs` with it: a song whose status in its
    /// document is not where the journal's moves leave it is listed ([`StatusDiffers`]). A song
    /// the journal concerns nowhere, by its id, is not compared: the journal has nothing to say
    /// of it. One it concerns with no move stands at idea, where `duet song new` makes it.
    #[must_use]
    pub fn with_songs(journal: &ProjectJournal, songs: &[SongOnDisk]) -> Self {
        let mut report = Self::of(journal);
        let replay = Replay::of(journal);
        for song in songs {
            let (journal_status, set) = match replay.song(&song.id) {
                Some(record) => (record.status, Some(record.set)),
                None if journal.concerns_song(&song.id) => (Status::Idea, None),
                None => continue,
            };
            if journal_status != song.status {
                report.status_differs.push(StatusDiffers {
                    song: song.name.clone(),
                    id: song.id.clone(),
                    document: song.status,
                    journal: journal_status,
                    set,
                });
            }
        }
        report
    }

    /// Checks `journal`, comparing no song with it.
    #[must_use]
    pub fn of(journal: &ProjectJournal) -> Self {
        let mut unread = Vec::new();
        let mut duplicates = Vec::new();
        let mut seen: HashMap<&Id, Place> = HashMap::new();
        for (place, line) in journal.entries() {
            if let Body::Skipped(why) = &line.body {
                unread.push(Unread {
                    place,
                    id: line.id.clone(),
                    event: line.event.clone(),
                    v: line.v,
                    why: why.clone(),
                });
            }
            if let Some(first) = seen.get(&line.id) {
                duplicates.push(Duplicate {
                    place,
                    first: *first,
                    id: line.id.clone(),
                });
            } else {
                seen.insert(&line.id, place);
            }
        }
        let mut torn = Vec::new();
        if let Some(t) = &journal.index.torn {
            torn.push(Torn {
                file: crate::INDEX.to_string(),
                bytes: t.len(),
            });
        }
        for segment in &journal.segments {
            if let Some(t) = &segment.journal.torn {
                torn.push(Torn {
                    file: segment.stamp.file_name(),
                    bytes: t.len(),
                });
            }
        }
        let mut closes: HashMap<SegmentStamp, Vec<&Closes>> = HashMap::new();
        for entry in &journal.index.lines {
            if let Some(c) = &entry.line.closes {
                closes.entry(c.segment).or_default().push(c);
            }
        }
        let mut closed_twice: Vec<SegmentStamp> = closes
            .iter()
            .filter(|(_, c)| c.len() > 1)
            .map(|(s, _)| *s)
            .collect();
        closed_twice.sort();
        let mut close_differs = Vec::new();
        for segment in &journal.segments {
            let holds = segment.journal.lines.len() as u64;
            for c in closes.get(&segment.stamp).into_iter().flatten() {
                if holds != c.entries {
                    close_differs.push(CloseDiffers {
                        segment: segment.stamp,
                        says: c.entries,
                        holds,
                    });
                }
            }
        }
        let current = journal.index.current().map(|o| o.segment);
        let unsummarised = journal
            .segments
            .iter()
            .map(|s| s.stamp)
            .filter(|s| Some(*s) != current && !closes.contains_key(s))
            .collect();
        let (adoptable, unnamed) = journal
            .orphans
            .iter()
            .cloned()
            .partition::<Vec<_>, _>(|o| o.entries == 0);
        let replay = Replay::of(journal);
        Self {
            project: journal.index.head.project.clone(),
            version: IndexHead::VERSION,
            segments: journal.segments.len(),
            entries: journal.len(),
            unread,
            torn,
            closed_twice,
            close_differs,
            unsummarised,
            adoptable: adoptable.into_iter().map(|o| o.stamp).collect(),
            refused: replay.refused.clone(),
            refused_moves: replay.refused_moves.clone(),
            status_differs: Vec::new(),
            duplicates,
            missing: journal.missing.clone(),
            unnamed,
            open: replay.open().len(),
        }
    }

    /// Whether anything is wrong: a transition or a status move replay skipped, a song whose
    /// document disagrees with the journal, an id twice, a segment the index names that is
    /// missing, or entries no index line names. Everything else is stated.
    #[must_use]
    pub const fn is_wrong(&self) -> bool {
        self.wrong() > 0
    }

    const fn wrong(&self) -> usize {
        self.refused.len()
            + self.refused_moves.len()
            + self.status_differs.len()
            + self.duplicates.len()
            + self.missing.len()
            + self.unnamed.len()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "journal of {}: {}, {}, {} open",
            self.project,
            plural(self.segments, "segment", "segments"),
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
                    "  {} ({}): {} {}, {}",
                    u.place, u.id, u.event, u.v, u.why
                )?;
            }
        }
        for t in &self.torn {
            writeln!(
                f,
                "{}: the last line is torn ({}, no newline): a write that never finished",
                t.file,
                plural(t.bytes, "byte", "bytes")
            )?;
        }
        for s in &self.closed_twice {
            writeln!(
                f,
                "segment {s} is closed by more than one index line: a merge of two clones that \
                 each rolled; every segment is still read"
            )?;
        }
        for c in &self.close_differs {
            writeln!(
                f,
                "segment {} holds {} where a closing line says {}: it was appended to after it \
                 closed, by a merge; the file stands",
                c.segment,
                plural(
                    usize::try_from(c.holds).unwrap_or(usize::MAX),
                    "entry",
                    "entries"
                ),
                c.says
            )?;
        }
        for s in &self.unsummarised {
            writeln!(
                f,
                "segment {s} is unsummarised: no index line closes it by name, after a merge of \
                 two clones that each rolled; it is read whole"
            )?;
        }
        for s in &self.adoptable {
            writeln!(
                f,
                "segment {s} has a head and no index line: a roll that never finished; the next \
                 append adopts it"
            )?;
        }
        if !self.refused.is_empty() {
            writeln!(
                f,
                "{} skipped on replay, the first in order standing:",
                plural(self.refused.len(), "transition", "transitions")
            )?;
            for r in &self.refused {
                let stands = r
                    .stands
                    .map(|s| format!("; {s} stands"))
                    .unwrap_or_default();
                writeln!(
                    f,
                    "  {} ({}): {} of ask {}, refused {}{stands}",
                    r.place, r.id, r.event, r.ask, r.refusal
                )?;
            }
        }
        if !self.refused_moves.is_empty() {
            writeln!(
                f,
                "{} skipped on replay, the move before standing:",
                plural(self.refused_moves.len(), "status move", "status moves")
            )?;
            for r in &self.refused_moves {
                let stands = r
                    .stands
                    .map(|s| format!("; {s} stands"))
                    .unwrap_or_default();
                writeln!(
                    f,
                    "  {} ({}): song.status-set of song {} ({}), {} → {}, refused {}{stands}",
                    r.place, r.id, r.name, r.song, r.from, r.to, r.refusal
                )?;
            }
        }
        for d in &self.status_differs {
            let journal = d.set.map_or_else(
                || "the journal has no move for it, so leaves it at idea".to_owned(),
                |set| format!("the journal's last move, {set}, leaves it at {}", d.journal),
            );
            writeln!(
                f,
                "song {} ({}): its song.duet says {}, and {journal}; neither is changed",
                d.song, d.id, d.document
            )?;
        }
        if !self.duplicates.is_empty() {
            writeln!(
                f,
                "{} that another line has too:",
                plural(self.duplicates.len(), "id", "ids")
            )?;
            for d in &self.duplicates {
                writeln!(f, "  {} has the id of {}: {}", d.place, d.first, d.id)?;
            }
        }
        for s in &self.missing {
            writeln!(
                f,
                "the index names segment {s}, and {} is missing",
                s.file_name()
            )?;
        }
        for o in &self.unnamed {
            writeln!(
                f,
                "{} holds {} and no index line names it",
                o.stamp.file_name(),
                plural(o.entries, "entry", "entries")
            )?;
        }
        if self.is_wrong() {
            write!(f, "{} wrong", plural(self.wrong(), "thing", "things"))
        } else {
            write!(f, "nothing wrong")
        }
    }
}
