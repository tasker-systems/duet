use std::path::PathBuf;

/// Everything that can go wrong capturing a take.
///
/// What the analysis itself refuses is a [`duet_dsp::Error`], carried whole as
/// [`Error::Analysis`] with its message unchanged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A source or a conversion met input it cannot work with.
    #[error("{0}")]
    Invalid(String),

    /// A file whose audio duet cannot decode: HE-AAC, a codec Symphonia lacks, or not audio.
    #[error("cannot decode {}: {codec}", path.display())]
    Undecodable {
        /// The file that was refused.
        path: PathBuf,
        /// What the file is, as far as it could be told.
        codec: String,
    },

    /// A take could not be written: the file could not be created or written, or the
    /// encoder refused a frame.
    /// Its message is the file's alone: the cause follows as the error's source, so a chain
    /// printed whole names it once.
    #[error("cannot write {}", path.display())]
    Write {
        /// The file being written.
        path: PathBuf,
        source: std::io::Error,
    },

    /// The take has nothing to assemble: a melody with no notes, or a rhythm with no hits.
    #[error("no {what} found")]
    NothingFound {
        /// What was looked for: `notes` or `hits`.
        what: &'static str,
    },

    /// The analysis refused: a stage met input it cannot work with, or the take has too few
    /// onsets to estimate a tempo from.
    #[error(transparent)]
    Analysis(#[from] duet_dsp::Error),
}

impl From<duet_core::Error> for Error {
    fn from(e: duet_core::Error) -> Self {
        match e {
            duet_core::Error::Invalid(msg) => Error::Invalid(msg),
            // duet_core::Error is non_exhaustive: a variant added there keeps its message, and
            // Parse, which capture has no variant for, keeps its line number in its message.
            other => Error::Invalid(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
