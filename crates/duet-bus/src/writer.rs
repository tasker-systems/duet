//! Appending to the journal: one lock, one line, one `fdatasync`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use duet_schemas::journal::{By, Entry, Event, Id, JournalHead, read_line};
use duet_schemas::{Journal, Tool};

use crate::{Error, Replay, Result, fresh_id, now};

/// What was cut from a torn last line before an append: a write that never finished, and was
/// never read by anyone as an entry. The only change ever made to bytes already in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cut {
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
    /// Its position: the first entry is `seq` 1.
    pub seq: u64,
    pub id: Id,
    /// A torn last line cut before it was written, if there was one.
    pub cut: Option<Cut>,
    /// Whether this append wrote the journal's head, the file being empty.
    pub started: bool,
}

/// How far a writer has read: every byte up to `offset`, whose last line is `last`.
#[derive(Debug, Default)]
struct Folded {
    offset: u64,
    /// The last whole line folded, with its newline: checked on the next append, so a journal
    /// rewritten or merged under the writer is folded again from the start.
    last: Vec<u8>,
    entries: u64,
    replay: Replay,
}

/// Appends entries to one journal, each as one step under the journal's lock: cut a torn last
/// line, fold in what was appended since, check the entry's transition, append it, sync.
#[derive(Debug)]
pub struct Writer {
    journal: PathBuf,
    lock: PathBuf,
    tool: Tool,
    head: JournalHead,
    by: By,
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

/// Cuts a last line with no newline back to just past the last newline, and syncs.
fn cut_torn_tail(file: &mut File) -> std::io::Result<Option<Cut>> {
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
    'search: while end > 0 {
        let start = end.saturating_sub(4096);
        let n = usize::try_from(end - start).unwrap_or(4096);
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk[..n])?;
        if let Some(i) = chunk[..n].iter().rposition(|b| *b == b'\n') {
            keep = start + i as u64 + 1;
            break 'search;
        }
        end = start;
    }
    let mut dropped = Vec::new();
    file.seek(SeekFrom::Start(keep))?;
    file.read_to_end(&mut dropped)?;
    file.set_len(keep)?;
    file.sync_data()?;
    Ok(Some(Cut {
        from: len,
        to: keep,
        dropped: String::from_utf8_lossy(&dropped).into_owned(),
    }))
}

impl Writer {
    /// A writer of the journal at `journal`, locking `lock` for each append, writing entries as
    /// `by`. When the journal is missing or empty, the first append starts it with `head`,
    /// written by `tool`; otherwise `head` is not used.
    #[must_use]
    pub fn new(
        journal: impl Into<PathBuf>,
        lock: impl Into<PathBuf>,
        tool: Tool,
        head: JournalHead,
        by: By,
    ) -> Self {
        Self {
            journal: journal.into(),
            lock: lock.into(),
            tool,
            head,
            by,
            folded: Folded::default(),
        }
    }

    /// A writer of the journal of the project at `root`: [`crate::journal_path`], locked by
    /// [`crate::lock_path`].
    #[must_use]
    pub fn of_project(root: &Path, tool: Tool, head: JournalHead, by: By) -> Self {
        Self::new(
            crate::journal_path(root),
            crate::lock_path(root),
            tool,
            head,
            by,
        )
    }

    /// The journal as this writer last folded it, through its last append.
    #[must_use]
    pub const fn replay(&self) -> &Replay {
        &self.folded.replay
    }

    /// Appends `event`, caused by `cause` if given, as one step under the lock (bus decision
    /// 13):
    /// 1. take the lock;
    /// 2. cut a torn last line back to the last newline, and start an empty journal with its
    ///    head;
    /// 3. fold in every line appended since this writer last read;
    /// 4. check the event's transition against the result;
    /// 5. append the entry as one line, with one write, and `fdatasync`;
    /// 6. release the lock.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] when the event's lifecycle refuses it, which appends nothing;
    /// [`Error::Journal`] when a line in the file does not read; [`Error::Io`] and
    /// [`Error::Random`].
    pub fn append(&mut self, event: impl Into<Event>, cause: Option<Id>) -> Result<Appended> {
        let event = event.into();
        let path = self.journal.clone();
        let io = |e| Error::io(&path)(e);
        let _held = take_lock(&self.lock)?;
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&self.journal)
            .map_err(io)?;
        let cut = cut_torn_tail(&mut file).map_err(io)?;
        let started = file.metadata().map_err(io)?.len() == 0;
        if started {
            let head = Journal::head_line(&self.tool, &self.head).map_err(Error::journal(&path))?;
            write_line(&mut file, head.as_bytes()).map_err(io)?;
            file.sync_data().map_err(io)?;
        }
        self.fold(&mut file)?;
        let mut entry = Entry::new(fresh_id()?, now(), self.by.clone(), event);
        entry.cause = cause;
        if let Err((refusal, ask)) = self.folded.replay.check(&entry.id, &entry.event) {
            return Err(Error::Refused {
                refusal,
                why: format!("{} of ask {ask}", entry.event.name()),
            });
        }
        let line = entry.line().map_err(Error::journal(&path))?;
        write_line(&mut file, line.as_bytes()).map_err(io)?;
        file.sync_data().map_err(io)?;
        let seq = self.fold_line(line.into_bytes())?;
        Ok(Appended {
            seq,
            id: entry.id,
            cut,
            started,
        })
    }

    /// Folds in every whole line after what this writer has read. When the line it read last is
    /// no longer where it was, the journal was rewritten or merged under it, and it folds the
    /// whole file again.
    fn fold(&mut self, file: &mut File) -> Result<()> {
        let path = self.journal.clone();
        let io = |e| Error::io(&path)(e);
        let len = file.metadata().map_err(io)?.len();
        let f = &self.folded;
        let still = f.offset <= len && {
            let start = f.offset - f.last.len() as u64;
            let mut here = vec![0u8; f.last.len()];
            file.seek(SeekFrom::Start(start)).map_err(io)?;
            file.read_exact(&mut here).map_err(io)?;
            here == f.last
        };
        if !still {
            self.folded = Folded::default();
        }
        let mut text = String::new();
        file.seek(SeekFrom::Start(self.folded.offset)).map_err(io)?;
        file.read_to_string(&mut text).map_err(io)?;
        if self.folded.offset == 0 {
            // The whole file: the head is read by the version policy, so a newer journal is
            // refused rather than appended to.
            let journal = Journal::read(&text).map_err(Error::journal(&path))?;
            let head_len = text.find('\n').map_or(text.len(), |i| i + 1);
            self.folded.offset = head_len as u64;
            self.folded.last = text.as_bytes()[..head_len].to_vec();
            for line in journal.lines {
                let mut bytes = line.text.into_bytes();
                bytes.push(b'\n');
                self.fold_line(bytes)?;
            }
        } else {
            for line in text.split_inclusive('\n') {
                self.fold_line(line.as_bytes().to_vec())?;
            }
        }
        Ok(())
    }

    /// Folds one whole line, with its newline, and returns its `seq`.
    fn fold_line(&mut self, bytes: Vec<u8>) -> Result<u64> {
        let text = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|t| t.strip_suffix('\n'))
            .unwrap_or_default();
        let line = read_line(text).map_err(|source| Error::Journal {
            path: self.journal.clone(),
            source: duet_schemas::Error::Line {
                line: usize::try_from(self.folded.entries + 2).unwrap_or(usize::MAX),
                source: Box::new(source),
            },
        })?;
        let folded = &mut self.folded;
        folded.entries += 1;
        folded.replay.fold(folded.entries, &line);
        folded.offset += bytes.len() as u64;
        folded.last = bytes;
        Ok(folded.entries)
    }
}
