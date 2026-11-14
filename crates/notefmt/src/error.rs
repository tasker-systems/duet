use std::fmt;

/// Everything that can go wrong reading or writing a song.
#[derive(Debug)]
pub enum Error {
    /// The MIDI file could not be parsed, or uses a feature this crate does not support.
    Midi(String),
    /// A line of the text format could not be parsed. `line` is 1-based.
    Parse { line: usize, msg: String },
    /// A value in a [`Song`](crate::Song) is out of range or inconsistent.
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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Midi(msg) => write!(f, "MIDI error: {msg}"),
            Error::Parse { line, msg } => write!(f, "line {line}: {msg}"),
            Error::Invalid(msg) => write!(f, "invalid song: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
