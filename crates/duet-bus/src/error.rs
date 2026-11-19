//! Why a journal was not read or appended to.

use std::path::{Path, PathBuf};

use duet_schemas::journal::SegmentStamp;

use crate::ask::Refusal;

/// Why a journal was not read, or an entry not appended.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read, written, locked or synced. The message is the path: the
    /// source says why, as `{:#}` prints it.
    #[error("{}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A file of the journal did not read: the source names the line and why.
    #[error("{}", path.display())]
    Journal {
        path: PathBuf,
        #[source]
        source: duet_schemas::Error,
    },

    /// There is no index, so no journal, at the project's root.
    #[error("{}: no journal here (no {})", root.display(), crate::INDEX)]
    NoJournal { root: PathBuf },

    /// The index names the current segment, and its file does not exist.
    #[error("the index names the current segment {segment}, and its file {} does not exist", segment.file_name())]
    MissingSegment { segment: SegmentStamp },

    /// The entry's transition is refused by its lifecycle, so it was not appended.
    #[error("refused, {refusal}: {why}")]
    Refused { refusal: Refusal, why: String },

    /// A song's status move is refused by the song's lifecycle, as the journal has the song, so
    /// it was not appended.
    #[error("refused, {refusal}: {why}")]
    RefusedMove {
        refusal: crate::song::Refusal,
        why: String,
    },

    /// A path the journal writes through is a symbolic link: the journal is written only inside
    /// its project, so a link that could lead out of it is refused, and nothing is written.
    #[error(
        "{} is a symbolic link; the journal is written only to files in the project itself, so \
         it is refused and nothing was written",
        path.display()
    )]
    Link { path: PathBuf },

    /// Randomness for an id could not be had from the operating system.
    #[error("no randomness for an id: {0}")]
    Random(String),
}

impl Error {
    pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Self + '_ {
        move |source| Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn journal(path: &Path) -> impl FnOnce(duet_schemas::Error) -> Self + '_ {
        move |source| Self::Journal {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// A `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
