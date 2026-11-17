//! Why a journal was not read or appended to.

use std::path::{Path, PathBuf};

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

    /// The journal did not read: the source names the line and why.
    #[error("{}", path.display())]
    Journal {
        path: PathBuf,
        #[source]
        source: duet_schemas::Error,
    },

    /// The entry's transition is refused by its lifecycle, so it was not appended.
    #[error("refused, {refusal}: {why}")]
    Refused { refusal: Refusal, why: String },

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
