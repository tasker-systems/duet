use std::path::PathBuf;

/// Everything that can go wrong capturing a take.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A source, a stage or a conversion met input it cannot work with.
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

    /// The take has too few onsets to estimate a tempo from.
    #[error(
        "found {found} onset{}, too few to estimate a tempo; give one with --tempo",
        if *found == 1 { "" } else { "s" }
    )]
    TooFewOnsets {
        /// How many onsets the take has.
        found: usize,
    },

    /// The take has nothing to assemble: a melody with no notes, or a rhythm with no hits.
    #[error("no {what} found")]
    NothingFound {
        /// What was looked for: `notes` or `hits`.
        what: &'static str,
    },
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
