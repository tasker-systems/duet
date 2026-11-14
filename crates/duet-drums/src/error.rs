/// Everything that can go wrong reading, writing or converting a drum grid.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A line of the text form could not be read. `line` is 1-based.
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },
    /// A grid, a lane map or a conversion is out of range or inconsistent.
    #[error("{0}")]
    Invalid(String),
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

impl From<duet_core::Error> for Error {
    fn from(e: duet_core::Error) -> Self {
        match e {
            duet_core::Error::Parse { line, msg } => Error::Parse { line, msg },
            duet_core::Error::Invalid(msg) => Error::Invalid(msg),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
