//! Reading the journal from a cursor, and following it as it grows.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use duet_schemas::Journal;
use duet_schemas::journal::{Id, Line, read_line};

use crate::{Error, Result};

/// Where a reader is: the `seq` of the last entry it has, and that entry's `id`. `seq` 0, with
/// the journal's own id, is before the first entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cursor {
    pub seq: u64,
    pub id: Id,
}

impl Cursor {
    #[must_use]
    pub const fn new(seq: u64, id: Id) -> Self {
        Self { seq, id }
    }

    /// Before the first entry of `journal`.
    #[must_use]
    pub fn start(journal: &Journal) -> Self {
        Self::new(0, journal.head.id.clone())
    }
}

/// What a reader at a cursor reads next.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Polled {
    /// The entries after the cursor, each with its `seq`. Empty when there are none yet.
    After(Vec<(u64, Line)>),
    /// The entry at the cursor is no longer the one it named: the journal was merged or rewritten
    /// under it. The reader starts over from `seq` 0, and these are every entry from the first.
    Resync(Vec<(u64, Line)>),
}

/// The entries of `journal` after `cursor`: [`Polled::After`] when the line at `cursor.seq` is
/// still `cursor.id`, and [`Polled::Resync`] with every entry when it is not. A reader is never
/// told it is up to date when it is not.
#[must_use]
pub fn after(journal: &Journal, cursor: &Cursor) -> Polled {
    let numbered = |from: usize| -> Vec<(u64, Line)> {
        (1..)
            .zip(&journal.lines)
            .skip(from)
            .map(|(seq, line)| (seq, line.clone()))
            .collect()
    };
    let at = usize::try_from(cursor.seq).ok();
    let holds = match at {
        Some(0) => cursor.id == journal.head.id,
        Some(n) => journal.lines.get(n - 1).is_some_and(|l| l.id == cursor.id),
        None => false,
    };
    match (holds, at) {
        (true, Some(n)) => Polled::After(numbered(n)),
        _ => Polled::Resync(numbered(0)),
    }
}

/// Follows a journal file from a cursor: each [`Tail::poll`] reads the whole lines appended
/// since the last, and never a torn one.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    /// The byte just past the last whole line read.
    offset: u64,
    /// That last line, with its newline, checked on each poll.
    last: Vec<u8>,
    cursor: Cursor,
}

impl Tail {
    /// Opens the journal at `path` and reads it from `cursor` (`None`: from the start). Returns
    /// the tail, and what is after the cursor now.
    ///
    /// # Errors
    ///
    /// When the journal cannot be read, or does not read.
    pub fn open(path: &Path, cursor: Option<Cursor>) -> Result<(Self, Polled)> {
        let mut tail = Self {
            path: path.to_path_buf(),
            offset: 0,
            last: Vec::new(),
            cursor: Cursor::new(0, Id::from_parts(0, [0; 10])),
        };
        let polled = tail.reread(cursor.as_ref())?;
        Ok((tail, polled))
    }

    /// The cursor after the last entry read.
    #[must_use]
    pub const fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Reads the whole file again, and what is after `cursor` in it.
    fn reread(&mut self, cursor: Option<&Cursor>) -> Result<Polled> {
        let text = std::fs::read_to_string(&self.path).map_err(Error::io(&self.path))?;
        let journal = Journal::read(&text).map_err(Error::journal(&self.path))?;
        let whole = text.len() - journal.torn.as_ref().map_or(0, String::len);
        let start = Cursor::start(&journal);
        let polled = after(&journal, cursor.unwrap_or(&start));
        self.offset = whole as u64;
        let last_start = text[..whole.saturating_sub(1)]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.last = text.as_bytes()[last_start..whole].to_vec();
        self.cursor = journal.lines.last().map_or(start, |line| {
            Cursor::new(journal.lines.len() as u64, line.id.clone())
        });
        Ok(match (cursor, polled) {
            // Opening from the start is no resync: there was nothing to lose.
            (None, Polled::Resync(all)) => Polled::After(all),
            (_, polled) => polled,
        })
    }

    /// The whole entries appended since the last poll, or [`Polled::Resync`] with every entry
    /// when the file changed under the tail.
    ///
    /// # Errors
    ///
    /// When the journal cannot be read, or a new line does not read.
    pub fn poll(&mut self) -> Result<Polled> {
        let path = self.path.clone();
        let io = |e| Error::io(&path)(e);
        let mut file = File::open(&self.path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        let still = self.offset <= len && {
            let mut here = vec![0u8; self.last.len()];
            file.seek(SeekFrom::Start(self.offset - self.last.len() as u64))
                .map_err(io)?;
            file.read_exact(&mut here).map_err(io)?;
            here == self.last
        };
        if !still {
            let cursor = self.cursor.clone();
            return self.reread(Some(&cursor));
        }
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(io)?;
        let whole = text.rfind('\n').map_or(0, |i| i + 1);
        let mut out = Vec::new();
        for line in text[..whole].split_inclusive('\n') {
            let seq = self.cursor.seq + 1;
            let read = read_line(line.trim_end_matches('\n')).map_err(|source| Error::Journal {
                path: self.path.clone(),
                source: duet_schemas::Error::Line {
                    line: usize::try_from(seq + 1).unwrap_or(usize::MAX),
                    source: Box::new(source),
                },
            })?;
            self.cursor = Cursor::new(seq, read.id.clone());
            self.offset += line.len() as u64;
            self.last = line.as_bytes().to_vec();
            out.push((seq, read));
        }
        Ok(Polled::After(out))
    }
}
