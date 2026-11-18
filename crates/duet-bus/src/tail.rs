//! Reading the journal from a cursor, across segments, and following it as it grows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use duet_schemas::journal::{Id, Line, SegmentStamp};

use crate::{Place, ProjectJournal, Result, read, segment_path};

/// Where a reader is: the segment, the `seq` within it of the last entry it has, and that
/// entry's `id` (bus design decision 19). `seq` 0, with the segment's own id, is before the
/// segment's first entry. It names durable things only, so it survives a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cursor {
    pub segment: SegmentStamp,
    pub seq: u64,
    pub id: Id,
}

impl Cursor {
    #[must_use]
    pub const fn new(segment: SegmentStamp, seq: u64, id: Id) -> Self {
        Self { segment, seq, id }
    }

    /// At the entry at `place`, whose id is `id`.
    #[must_use]
    pub fn at(place: Place, id: Id) -> Self {
        Self::new(place.segment, place.seq, id)
    }
}

/// What a reader at a cursor reads next.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Polled {
    /// The entries after the cursor, in replay order. Empty when there are none yet.
    After(Vec<(Place, Line)>),
    /// The entry at the cursor is no longer the one it named: the journal was merged or rewritten
    /// under it. The reader starts over from the start of the segment where the check failed,
    /// since earlier segments did not change (bus design decision 19), or from the first when
    /// the cursor's segment is gone. These are every entry from there, the first item naming
    /// where.
    Resync(Vec<(Place, Line)>),
}

fn all(journal: &ProjectJournal) -> Vec<(Place, Line)> {
    from_segment(journal, 0)
}

/// Every entry from the start of the `i`th segment on, in replay order.
fn from_segment(journal: &ProjectJournal, i: usize) -> Vec<(Place, Line)> {
    journal
        .segments
        .iter()
        .skip(i)
        .flat_map(|segment| {
            (1..)
                .zip(&segment.journal.lines)
                .map(move |(seq, line)| (Place::new(segment.stamp, seq), line.clone()))
        })
        .collect()
}

/// The entries of `journal` after `cursor`: the rest of its segment, then every later segment in
/// index order, when the line at `cursor` is still `cursor.id`. When it is not,
/// [`Polled::Resync`] with every entry from the start of the cursor's segment, or from the first
/// when that segment is gone. A reader is never told it is up to date when it is not.
#[must_use]
pub fn after(journal: &ProjectJournal, cursor: &Cursor) -> Polled {
    let Some(i) = journal
        .segments
        .iter()
        .position(|s| s.stamp == cursor.segment)
    else {
        return Polled::Resync(all(journal));
    };
    let segment = &journal.segments[i];
    let holds = match usize::try_from(cursor.seq) {
        Ok(0) => segment.journal.head.id == cursor.id,
        Ok(n) => segment
            .journal
            .lines
            .get(n - 1)
            .is_some_and(|l| l.id == cursor.id),
        Err(_) => false,
    };
    if !holds {
        return Polled::Resync(from_segment(journal, i));
    }
    let mut out = Vec::new();
    for (j, segment) in journal.segments.iter().enumerate().skip(i) {
        let from = if j == i { cursor.seq } else { 0 };
        for (seq, line) in (1..).zip(&segment.journal.lines) {
            if seq > from {
                out.push((Place::new(segment.stamp, seq), line.clone()));
            }
        }
    }
    Polled::After(out)
}

/// Follows a project's journal from a cursor. Each [`Tail::poll`] reads the whole entries
/// appended since the last, across rolls, and never a torn line.
///
/// It reads the index on each poll and a segment only when its length changed. When a segment
/// before the cursor's changes, a merge added entries the reader has passed, so it is sent
/// [`Polled::Resync`] from the start of the earliest segment that grew: nothing is lost. A change that happened while no tail ran is caught only
/// when it moves the line at the cursor.
#[derive(Debug)]
pub struct Tail {
    root: PathBuf,
    cursor: Option<Cursor>,
    lengths: HashMap<SegmentStamp, u64>,
}

impl Tail {
    /// Opens the journal of the project at `root` and reads it from `cursor` (`None`: from the
    /// start). Returns the tail, and what is after the cursor now.
    ///
    /// # Errors
    ///
    /// When the journal cannot be read, or does not read.
    pub fn open(root: &Path, cursor: Option<Cursor>) -> Result<(Self, Polled)> {
        let mut tail = Self {
            root: root.to_path_buf(),
            cursor,
            lengths: HashMap::new(),
        };
        let polled = tail.poll()?;
        Ok((tail, polled))
    }

    /// The cursor after the last entry read, if any was.
    #[must_use]
    pub const fn cursor(&self) -> Option<&Cursor> {
        self.cursor.as_ref()
    }

    /// The whole entries appended since the last poll, or [`Polled::Resync`] with every entry
    /// when the journal changed under the tail.
    ///
    /// # Errors
    ///
    /// When the journal cannot be read, or a line does not read.
    pub fn poll(&mut self) -> Result<Polled> {
        let journal = read(&self.root)?;
        let lengths: HashMap<SegmentStamp, u64> = journal
            .segments
            .iter()
            .filter_map(|s| {
                std::fs::metadata(segment_path(&self.root, s.stamp))
                    .ok()
                    .map(|m| (s.stamp, m.len()))
            })
            .collect();
        let polled = match &self.cursor {
            None => Polled::After(all(&journal)),
            Some(cursor) => {
                let grew = journal
                    .segments
                    .iter()
                    .take_while(|s| s.stamp != cursor.segment)
                    .position(|s| {
                        self.lengths
                            .get(&s.stamp)
                            .is_some_and(|l| lengths.get(&s.stamp) != Some(l))
                    });
                match grew {
                    Some(i) => Polled::Resync(from_segment(&journal, i)),
                    None => after(&journal, cursor),
                }
            }
        };
        self.lengths = lengths;
        let last = match &polled {
            Polled::After(v) | Polled::Resync(v) => v.last(),
        };
        if let Some((place, line)) = last {
            self.cursor = Some(Cursor::at(*place, line.id.clone()));
        } else if let Polled::Resync(_) = polled {
            self.cursor = None;
        }
        Ok(polled)
    }
}
