//! A project's whole journal, read: the index, and every segment it names.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use duet_schemas::journal::{Closes, Id, Line, SegmentStamp};
use duet_schemas::{Journal, JournalIndex};

use crate::{Error, Result, index_path, segment_path};

/// Where an entry is: its segment, and its `seq` within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Place {
    pub segment: SegmentStamp,
    pub seq: u64,
}

impl Place {
    #[must_use]
    pub const fn new(segment: SegmentStamp, seq: u64) -> Self {
        Self { segment, seq }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} seq {}", self.segment, self.seq)
    }
}

/// One segment the index names, read.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Segment {
    pub stamp: SegmentStamp,
    pub journal: Journal,
}

/// A segment file no index line names.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Orphan {
    pub stamp: SegmentStamp,
    /// How many entries it holds. With none, a crashed roll left it, and the next append adopts
    /// it. With some, it is wrong: no writer appends to a segment before its index line exists.
    pub entries: usize,
}

/// A project's whole journal.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ProjectJournal {
    pub index: JournalIndex,
    /// Every segment the index names whose file exists, in index order, each once.
    pub segments: Vec<Segment>,
    /// Segments the index names whose file does not exist: wrong.
    pub missing: Vec<SegmentStamp>,
    /// Segment files the index does not name, oldest first.
    pub orphans: Vec<Orphan>,
}

impl ProjectJournal {
    /// Every entry, in replay order: the segments in index order, each in file order.
    pub fn entries(&self) -> impl Iterator<Item = (Place, &Line)> {
        self.segments.iter().flat_map(|segment| {
            (1..)
                .zip(&segment.journal.lines)
                .map(move |(seq, line)| (Place::new(segment.stamp, seq), line))
        })
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.iter().map(|s| s.journal.lines.len()).sum()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The segments that may concern the song `id`, in index order: each whose closing summary
    /// lists the song by that id, under whatever name, and each with no summary to read, which
    /// is read whole: the current segment, and one a merge left unsummarised (bus design
    /// decisions 17 and 21). A song renamed since a segment closed is still found in it.
    #[must_use]
    pub fn segments_for_song(&self, id: &Id) -> Vec<SegmentStamp> {
        self.segments_where(|closes| closes.concerns_song(id))
    }

    /// The segments that may concern the part `id`, as [`Self::segments_for_song`] finds a
    /// song's.
    #[must_use]
    pub fn segments_for_part(&self, id: &Id) -> Vec<SegmentStamp> {
        self.segments_where(|closes| closes.concerns_part(id))
    }

    fn segments_where(&self, concerns: impl Fn(&Closes) -> bool) -> Vec<SegmentStamp> {
        let closes: Vec<&Closes> = self
            .index
            .lines
            .iter()
            .filter_map(|entry| entry.line.closes.as_ref())
            .collect();
        self.segments
            .iter()
            .map(|segment| segment.stamp)
            .filter(|stamp| {
                let mut summaries = closes.iter().filter(|c| c.segment == *stamp).peekable();
                summaries.peek().is_none() || summaries.any(|c| concerns(c))
            })
            .collect()
    }

    /// Whether any entry concerns the song `id`, read from the entries themselves.
    #[must_use]
    pub fn concerns_song(&self, id: &Id) -> bool {
        self.entries().any(|(_, line)| {
            line.read().is_some_and(|event| {
                event
                    .concerns()
                    .songs
                    .iter()
                    .any(|s| s.id.as_ref() == Some(id))
            })
        })
    }
}

/// The stamps of every segment file under `root`, oldest first.
pub(crate) fn segment_files(root: &Path) -> Result<Vec<SegmentStamp>> {
    let mut stamps: Vec<SegmentStamp> = std::fs::read_dir(root)
        .map_err(Error::io(root))?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            SegmentStamp::of_file_name(&name)
        })
        .collect();
    stamps.sort();
    Ok(stamps)
}

/// Reads the segment `stamp` under `root`, if its file exists.
pub(crate) fn read_segment(root: &Path, stamp: SegmentStamp) -> Result<Option<Journal>> {
    let path = segment_path(root, stamp);
    match std::fs::read_to_string(&path) {
        Ok(text) => Journal::read(&text)
            .map(Some)
            .map_err(Error::journal(&path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&path)(e)),
    }
}

/// Reads the journal of the project at `root`.
///
/// # Errors
///
/// [`Error::NoJournal`] when there is no index, [`Error::Io`] when a file cannot be read, and
/// [`Error::Journal`] when the index or a segment does not read, naming the file and the line.
pub fn read(root: &Path) -> Result<ProjectJournal> {
    let path = index_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NoJournal {
                root: root.to_path_buf(),
            });
        }
        Err(e) => return Err(Error::io(&path)(e)),
    };
    let index = JournalIndex::read(&text).map_err(Error::journal(&path))?;
    let mut named = HashSet::new();
    let mut segments = Vec::new();
    let mut missing = Vec::new();
    for entry in &index.lines {
        let stamp = entry.line.opens.segment;
        if !named.insert(stamp) {
            continue;
        }
        match read_segment(root, stamp)? {
            Some(journal) => segments.push(Segment { stamp, journal }),
            None => missing.push(stamp),
        }
    }
    let mut orphans = Vec::new();
    for stamp in segment_files(root)? {
        if !named.contains(&stamp)
            && let Some(journal) = read_segment(root, stamp)?
        {
            orphans.push(Orphan {
                stamp,
                entries: journal.lines.len(),
            });
        }
    }
    Ok(ProjectJournal {
        index,
        segments,
        missing,
        orphans,
    })
}
