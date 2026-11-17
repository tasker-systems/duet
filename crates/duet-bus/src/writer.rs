//! Appending to the journal: one lock over the index and every segment, one line per write, one
//! `fdatasync` per line.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use duet_schemas::journal::{
    At, By, Closes, Concerns, Entry, Event, Id, IndexHead, IndexLine, JournalHead, Opens,
    SegmentStamp, read_line,
};
use duet_schemas::{Journal, JournalIndex, Tool};

use crate::project::{read_segment, segment_files};
use crate::{Error, Place, Replay, Result, fresh_id, index_path, lock_path, now, segment_path};

/// The size at which a segment rolls when none is given: `[journal] segment-size`'s default
/// (bus design decision 16).
pub const DEFAULT_SEGMENT_SIZE: u64 = 1 << 20;

/// What was cut from a torn last line before an append: a write that never finished, and was
/// never read by anyone as a line. The only change ever made to bytes already in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cut {
    /// The file's name: the index's, or a segment's.
    pub file: String,
    /// The file's length before the cut.
    pub from: u64,
    /// Its length after: just past the last newline.
    pub to: u64,
    /// The bytes cut, as text (lossily, if they were cut inside a character).
    pub dropped: String,
}

/// An entry appended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Appended {
    /// Where it is.
    pub place: Place,
    pub id: Id,
    /// Torn last lines cut before it was written: the index's, the current segment's, or both.
    pub cuts: Vec<Cut>,
    /// Whether this append started the journal: wrote the index's head and the first segment.
    pub started: bool,
    /// A segment a crashed roll left with no index line, which this append adopted.
    pub adopted: Option<SegmentStamp>,
    /// The segment this append rolled to, the last being full.
    pub rolled: Option<SegmentStamp>,
}

/// How far a writer has read one segment.
#[derive(Debug, Clone)]
struct SegmentFold {
    stamp: SegmentStamp,
    /// Every byte up to here, whose last line is `last`, with its newline.
    offset: u64,
    last: Vec<u8>,
    entries: u64,
    last_entry: Option<(Id, At)>,
    concerns: Concerns,
}

/// How far a writer has read the journal: the index lines, and each segment they open.
#[derive(Debug, Default)]
struct Folded {
    index: Vec<IndexLine>,
    segments: Vec<SegmentFold>,
    replay: Replay,
}

/// Appends entries to a project's journal, each as one step under the journal's lock.
#[derive(Debug)]
pub struct Writer {
    root: PathBuf,
    tool: Tool,
    project: String,
    by: By,
    segment_size: u64,
    folded: Folded,
}

/// The lock, held until dropped. `File::lock` is an advisory exclusive lock, `flock` on Unix,
/// released when the file is closed, by drop or by the process ending.
struct Held {
    _file: File,
}

fn take_lock(path: &Path) -> Result<Held> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(Error::io(dir))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(Error::io(path))?;
    file.lock().map_err(Error::io(path))?;
    Ok(Held { _file: file })
}

/// Writes `bytes` with one `write(2)`, as a line of a journal's size always is on a regular
/// file; the rest, if a short write ever left any, follows at once, still under the lock.
fn write_line(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    let n = file.write(bytes)?;
    if n < bytes.len() {
        file.write_all(&bytes[n..])?;
    }
    Ok(())
}

/// Opens `path` to append to, and to read.
fn open_append(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(Error::io(path))
}

/// Cuts a last line with no newline back to just past the last newline, and syncs.
fn cut_torn_tail(file: &mut File, name: &str) -> std::io::Result<Option<Cut>> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(None);
    }
    let mut last = [0u8; 1];
    file.seek(SeekFrom::Start(len - 1))?;
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(None);
    }
    let mut end = len;
    let mut keep = 0;
    let mut chunk = vec![0u8; 4096];
    while end > 0 {
        let start = end.saturating_sub(4096);
        let n = usize::try_from(end - start).unwrap_or(4096);
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk[..n])?;
        if let Some(i) = chunk[..n].iter().rposition(|b| *b == b'\n') {
            keep = start + i as u64 + 1;
            break;
        }
        end = start;
    }
    let mut dropped = Vec::new();
    file.seek(SeekFrom::Start(keep))?;
    file.read_to_end(&mut dropped)?;
    file.set_len(keep)?;
    file.sync_data()?;
    Ok(Some(Cut {
        file: name.to_string(),
        from: len,
        to: keep,
        dropped: String::from_utf8_lossy(&dropped).into_owned(),
    }))
}

/// Syncs the directory `dir`, so a file just created in it is there after a crash.
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(Error::io(dir))
}

/// Whether the bytes of `file` just before `offset` are `last`.
fn ends_with_at(file: &mut File, offset: u64, last: &[u8]) -> std::io::Result<bool> {
    let len = file.metadata()?.len();
    if offset > len || (last.len() as u64) > offset {
        return Ok(false);
    }
    let mut here = vec![0u8; last.len()];
    file.seek(SeekFrom::Start(offset - last.len() as u64))?;
    file.read_exact(&mut here)?;
    Ok(here == last)
}

/// Why a fold has to start again from nothing: the journal changed other than by appends.
struct Changed;

impl Writer {
    /// A writer of the journal of the project at `root`, named `project`, writing entries as
    /// `by` and heads as `tool`. Segments roll at [`DEFAULT_SEGMENT_SIZE`].
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, tool: Tool, project: impl Into<String>, by: By) -> Self {
        Self {
            root: root.into(),
            tool,
            project: project.into(),
            by,
            segment_size: DEFAULT_SEGMENT_SIZE,
            folded: Folded::default(),
        }
    }

    /// The same writer, rolling a segment once it holds at least `bytes`.
    #[must_use]
    pub const fn segment_size(mut self, bytes: u64) -> Self {
        self.segment_size = bytes;
        self
    }

    /// The journal as this writer last folded it, through its last append.
    #[must_use]
    pub const fn replay(&self) -> &Replay {
        &self.folded.replay
    }

    /// Appends `event`, caused by `cause` if given, as one step under the lock (bus design
    /// decisions 13 and 20):
    /// 1. take the lock;
    /// 2. cut a torn last line of the index; start the index if it is empty;
    /// 3. adopt a segment a crashed roll left with no index line;
    /// 4. start the first segment if there is none, and cut a torn last line of the current one;
    /// 5. fold in everything appended since this writer last read;
    /// 6. roll if the current segment is full: the new segment's head, synced with its
    ///    directory, then the index line that closes the old one and opens it;
    /// 7. check the event's transition, append the entry with one write, and `fdatasync`.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] when the event's lifecycle refuses it, which appends nothing;
    /// [`Error::Journal`] when a line does not read; [`Error::MissingSegment`],
    /// [`Error::Io`] and [`Error::Random`].
    pub fn append(&mut self, event: impl Into<Event>, cause: Option<Id>) -> Result<Appended> {
        let event = event.into();
        let root = self.root.clone();
        let _held = take_lock(&lock_path(&root))?;
        let index_path = index_path(&root);
        let io = |e| Error::io(&index_path)(e);
        let mut cuts = Vec::new();

        // 2. The index.
        let mut index = open_append(&index_path)?;
        cuts.extend(cut_torn_tail(&mut index, crate::INDEX).map_err(io)?);
        let mut started = false;
        if index.metadata().map_err(io)?.len() == 0 {
            let head = IndexHead::new(self.project.clone(), fresh_id()?);
            let line =
                JournalIndex::head_line(&self.tool, &head).map_err(Error::journal(&index_path))?;
            write_line(&mut index, line.as_bytes()).map_err(io)?;
            index.sync_data().map_err(io)?;
            started = true;
        }
        let mut lines = self.read_index(&mut index)?;

        // 3. Adopt.
        let mut adopted = None;
        if let Some(stamp) = self.adoptable(&lines)? {
            let journal =
                read_segment(&root, stamp)?.ok_or(Error::MissingSegment { segment: stamp })?;
            let closes = self.closes(&lines)?;
            if lines.is_empty() || closes.is_some() {
                let opens = Opens::new(stamp, journal.head.id.clone(), stamp.at());
                self.append_index(&mut index, &IndexLine::new(opens, closes))?;
                lines = self.read_index(&mut index)?;
                adopted = Some(stamp);
            }
        }

        // 4. The first segment, and the current one's torn tail.
        if lines.is_empty() {
            self.open_segment(&mut index, None)?;
            lines = self.read_index(&mut index)?;
            started = true;
        }
        let current = lines
            .last()
            .map(|l| l.opens.segment)
            .ok_or(Error::MissingSegment {
                segment: SegmentStamp::new(now()),
            })?;
        let current_path = segment_path(&root, current);
        if !current_path.exists() {
            return Err(Error::MissingSegment { segment: current });
        }
        let mut segment = open_append(&current_path)?;
        cuts.extend(
            cut_torn_tail(&mut segment, &current.file_name())
                .map_err(|e| Error::io(&current_path)(e))?,
        );

        // 5. Fold.
        self.fold(&lines)?;

        // 6. Roll.
        let mut rolled = None;
        let size = segment
            .metadata()
            .map_err(|e| Error::io(&current_path)(e))?
            .len();
        let has_entries = self.folded.segments.last().is_some_and(|s| s.entries > 0);
        if size >= self.segment_size && has_entries {
            let closes = self.closes(&lines)?;
            let stamp = self.open_segment(&mut index, closes)?;
            lines = self.read_index(&mut index)?;
            self.fold(&lines)?;
            rolled = Some(stamp);
        }
        let current = lines.last().map(|l| l.opens.segment).unwrap_or(current);
        let current_path = segment_path(&root, current);
        let mut segment = open_append(&current_path)?;

        // 7. Check, append, sync.
        let mut entry = Entry::new(fresh_id()?, now(), self.by.clone(), event);
        entry.cause = cause;
        if let Err((refusal, ask)) = self.folded.replay.check(&entry.id, &entry.event) {
            return Err(Error::Refused {
                refusal,
                why: format!("{} of ask {ask}", entry.event.name()),
            });
        }
        let line = entry.line().map_err(Error::journal(&current_path))?;
        let seg_io = |e| Error::io(&current_path)(e);
        write_line(&mut segment, line.as_bytes()).map_err(seg_io)?;
        segment.sync_data().map_err(seg_io)?;
        let fold = self
            .folded
            .segments
            .last_mut()
            .ok_or(Error::MissingSegment { segment: current })?;
        let seq = fold_line(
            fold,
            &mut self.folded.replay,
            line.into_bytes(),
            &current_path,
        )?;
        Ok(Appended {
            place: Place::new(current, seq),
            id: entry.id,
            cuts,
            started,
            adopted,
            rolled,
        })
    }

    /// Reads the index's lines, the head by the version policy.
    fn read_index(&self, index: &mut File) -> Result<Vec<IndexLine>> {
        let path = index_path(&self.root);
        let mut text = String::new();
        index
            .seek(SeekFrom::Start(0))
            .and_then(|_| index.read_to_string(&mut text))
            .map_err(Error::io(&path))?;
        let read = JournalIndex::read(&text).map_err(Error::journal(&path))?;
        Ok(read.lines.into_iter().map(|entry| entry.line).collect())
    }

    /// Appends `line` to the index, and syncs.
    fn append_index(&self, index: &mut File, line: &IndexLine) -> Result<()> {
        let path = index_path(&self.root);
        let text = line.line().map_err(Error::journal(&path))?;
        write_line(index, text.as_bytes()).map_err(Error::io(&path))?;
        index.sync_data().map_err(Error::io(&path))
    }

    /// The newest segment file with only its head, newer than every segment the index names, and
    /// not named by it: what a roll that crashed between its two writes left.
    fn adoptable(&self, lines: &[IndexLine]) -> Result<Option<SegmentStamp>> {
        let named: Vec<SegmentStamp> = lines.iter().map(|l| l.opens.segment).collect();
        let newest = named.iter().max().copied();
        let mut found = None;
        for stamp in segment_files(&self.root)? {
            if named.contains(&stamp) || newest.is_some_and(|n| stamp <= n) {
                continue;
            }
            if let Some(journal) = read_segment(&self.root, stamp)?
                && journal.lines.is_empty()
                && journal.torn.is_none()
            {
                found = Some(stamp);
            }
        }
        Ok(found)
    }

    /// The summary that closes the current segment, from what this writer folded of it: `None`
    /// when there is no current segment, or it holds no entry yet.
    fn closes(&mut self, lines: &[IndexLine]) -> Result<Option<Closes>> {
        if lines.is_empty() {
            return Ok(None);
        }
        self.fold(lines)?;
        let Some(current) = self.folded.segments.last() else {
            return Ok(None);
        };
        Ok(current.last_entry.as_ref().map(|(id, at)| {
            Closes::new(
                current.stamp,
                *at,
                current.entries,
                id.clone(),
                current.concerns.clone(),
            )
        }))
    }

    /// Starts a new segment: its head, synced with its directory, then the index line that opens
    /// it and closes the last one with `closes`. Returns its stamp.
    fn open_segment(&self, index: &mut File, closes: Option<Closes>) -> Result<SegmentStamp> {
        let floor = self.folded.segments.last().map(|s| s.stamp.next());
        let mut stamp = SegmentStamp::new(now());
        if let Some(floor) = floor
            && stamp < floor
        {
            stamp = floor;
        }
        let (mut file, path) = loop {
            let path = segment_path(&self.root, stamp);
            match OpenOptions::new()
                .read(true)
                .append(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => break (file, path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => stamp = stamp.next(),
                Err(e) => return Err(Error::io(&path)(e)),
            }
        };
        let id = fresh_id()?;
        let head = JournalHead::new(self.project.clone(), id.clone(), stamp);
        let line = Journal::head_line(&self.tool, &head).map_err(Error::journal(&path))?;
        write_line(&mut file, line.as_bytes()).map_err(Error::io(&path))?;
        file.sync_data().map_err(Error::io(&path))?;
        sync_dir(&self.root)?;
        self.append_index(
            index,
            &IndexLine::new(Opens::new(stamp, id, stamp.at()), closes),
        )?;
        Ok(stamp)
    }

    /// Folds in everything appended since this writer last read: new lines of the segment it
    /// was in, and every segment opened since. When the journal changed other than by appends
    /// (a merge, a rewrite), it folds all of it again from the start.
    fn fold(&mut self, lines: &[IndexLine]) -> Result<()> {
        if self.try_fold(lines)?.is_err() {
            self.folded = Folded::default();
            if self.try_fold(lines)?.is_err() {
                // Only a file changing under the lock could do this; nothing appends there.
                self.folded = Folded::default();
            }
        }
        Ok(())
    }

    fn try_fold(&mut self, lines: &[IndexLine]) -> Result<std::result::Result<(), Changed>> {
        let folded = &mut self.folded;
        if lines.len() < folded.index.len() || lines[..folded.index.len()] != folded.index[..] {
            return Ok(Err(Changed));
        }
        let was_open = folded.segments.len().checked_sub(1);
        let mut seen = Vec::new();
        let mut at = 0;
        for line in lines {
            let stamp = line.opens.segment;
            if seen.contains(&stamp) {
                continue;
            }
            seen.push(stamp);
            let path = segment_path(&self.root, stamp);
            let Some(mut file) = File::open(&path).ok() else {
                // A missing segment is `check`'s to report. Only the current one stops a writer,
                // in `append`.
                continue;
            };
            if at == folded.segments.len() {
                folded.segments.push(SegmentFold {
                    stamp,
                    offset: 0,
                    last: Vec::new(),
                    entries: 0,
                    last_entry: None,
                    concerns: Concerns::new(),
                });
            }
            let fold = &mut folded.segments[at];
            if fold.stamp != stamp {
                return Ok(Err(Changed));
            }
            let io = |e| Error::io(&path)(e);
            let len = file.metadata().map_err(io)?.len();
            let unchanged = ends_with_at(&mut file, fold.offset, &fold.last).map_err(io)?;
            let closed_before = was_open.is_some_and(|open| at < open);
            if !unchanged || (closed_before && len != fold.offset) {
                return Ok(Err(Changed));
            }
            if len > fold.offset {
                let mut text = String::new();
                file.seek(SeekFrom::Start(fold.offset))
                    .and_then(|_| file.read_to_string(&mut text))
                    .map_err(io)?;
                let whole = text.rfind('\n').map_or(0, |i| i + 1);
                let mut rest = &text[..whole];
                if fold.offset == 0 {
                    // The head, by the version policy: a newer journal is refused, not appended to.
                    let head_len = rest.find('\n').map_or(rest.len(), |i| i + 1);
                    Journal::read(&rest[..head_len]).map_err(Error::journal(&path))?;
                    fold.offset = head_len as u64;
                    fold.last = rest.as_bytes()[..head_len].to_vec();
                    rest = &rest[head_len..];
                }
                for line in rest.split_inclusive('\n') {
                    fold_line(fold, &mut folded.replay, line.as_bytes().to_vec(), &path)?;
                }
            }
            at += 1;
        }
        folded.index = lines.to_vec();
        Ok(Ok(()))
    }
}

/// Folds one whole line of `fold`'s segment, with its newline, and returns its `seq`.
fn fold_line(
    fold: &mut SegmentFold,
    replay: &mut Replay,
    bytes: Vec<u8>,
    path: &Path,
) -> Result<u64> {
    let text = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|t| t.strip_suffix('\n'))
        .unwrap_or_default();
    let line = read_line(text).map_err(|source| Error::Journal {
        path: path.to_path_buf(),
        source: duet_schemas::Error::Line {
            line: usize::try_from(fold.entries + 2).unwrap_or(usize::MAX),
            source: Box::new(source),
        },
    })?;
    fold.entries += 1;
    replay.fold(Place::new(fold.stamp, fold.entries), &line);
    if let Some(event) = line.read() {
        fold.concerns.extend(&event.concerns());
    }
    fold.last_entry = Some((line.id.clone(), line.at));
    fold.offset += bytes.len() as u64;
    fold.last = bytes;
    Ok(fold.entries)
}
