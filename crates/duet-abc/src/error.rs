use duet_core::Position;

/// Everything that can go wrong reading or writing ABC.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The text is not valid ABC, or breaks a rule a song needs, such as a bar of the
    /// wrong length. `line` and `col` are 1-based, counted in characters.
    #[error("line {line}, col {col}: {msg}")]
    Parse {
        line: usize,
        col: usize,
        msg: String,
    },
    /// Valid ABC that is outside the subset this version reads. `construct` is a stable
    /// name from the refusal table ([`crate::REFUSED`]), and `found` is the text that
    /// was refused.
    #[error(
        "line {line}, col {col}: {found} is refused: {construct} is not read in this version ({hint})"
    )]
    Unsupported {
        line: usize,
        col: usize,
        construct: &'static str,
        found: String,
        hint: String,
    },
    /// A song with no ABC form under the writer's rules. `track` is `None` when the
    /// cause is song-wide, such as a tempo that is not a whole number of bpm.
    #[error("{}at {at}: {why} ({hint})", track.as_ref().map(|t| format!("track '{t}' ")).unwrap_or_default())]
    Unwritable {
        at: Position,
        track: Option<String>,
        why: String,
        hint: String,
    },
    /// Anything else, such as a tune number the file does not hold.
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub(crate) fn parse(line: usize, col: usize, msg: impl Into<String>) -> Self {
        Error::Parse {
            line,
            col,
            msg: msg.into(),
        }
    }

    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}

impl From<duet_core::Error> for Error {
    fn from(e: duet_core::Error) -> Self {
        // duet_core::Error is non_exhaustive: every variant keeps its message.
        Error::Invalid(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
