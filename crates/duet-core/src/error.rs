/// Everything that can go wrong reading or writing a song.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A line of the text format could not be parsed. `line` is 1-based.
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },
    /// A value in a [`Song`](crate::Song) is out of range or inconsistent.
    #[error("invalid song: {0}")]
    Invalid(String),
    /// A [`Tensor`](crate::Tensor)'s shape, number type or numbers do not agree.
    #[error("invalid tensor: {0}")]
    Tensor(String),
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    pub(crate) fn parse(line: usize, msg: impl Into<String>) -> Self {
        Error::Parse {
            line,
            msg: msg.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
