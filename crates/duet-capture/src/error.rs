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
}

pub type Result<T> = std::result::Result<T, Error>;
