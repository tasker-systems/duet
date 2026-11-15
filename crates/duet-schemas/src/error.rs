//! Why a document was refused.

use crate::Version;

/// Why a document or a stream was not read or written.
///
/// Every refusal of the version policy is its own variant, so a caller can tell them apart and a
/// person reads what to do: [`Error::NewerMinor`], [`Error::OtherMajor`],
/// [`Error::UnknownField`] (and, inside a body, [`Error::Body`], whose message names the field)
/// and [`Error::SchemaMismatch`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The text is not JSON, or not the JSON a document's frame needs.
    #[error("not a duet document: {0}")]
    Json(#[source] serde_json::Error),

    /// The `duet` object, the envelope, did not read.
    #[error("the envelope (`duet`) did not read: {0}")]
    Envelope(#[source] serde_json::Error),

    /// A version is not `<major>.<minor>`.
    #[error("{found:?} is not a version: a version is <major>.<minor>, such as 1.0")]
    BadVersion { found: String },

    /// A field the frame needs is missing.
    #[error("{at} has no `{field}`")]
    Missing {
        field: &'static str,
        at: &'static str,
    },

    /// A field no version of the frame holds. Refused rather than dropped, so that writing the
    /// file back never loses it.
    #[error("{at} has a field `{field}`, which no version of duet's documents holds")]
    UnknownField { field: String, at: &'static str },

    /// A document read as a stream, or a stream read as a document.
    #[error("this is a {found}, not a {expected}")]
    Framing {
        found: &'static str,
        expected: &'static str,
    },

    /// `$schema` disagrees with the envelope's `kind` and `version`.
    #[error(
        "`$schema` is {found}, but the envelope says {kind} {version}, whose schema is {expected}"
    )]
    SchemaMismatch {
        found: String,
        kind: String,
        version: Version,
        expected: String,
    },

    /// The document is of another kind than the one asked for.
    #[error("this is a {found} document, not a {expected} one")]
    WrongKind {
        found: String,
        expected: &'static str,
    },

    /// A newer minor of a major this reader knows. It is refused rather than read without the
    /// fields this reader does not know, since writing it back would drop them silently.
    #[error("this is {kind} {found}; this duet reads {kind} up to {newest}; update duet")]
    NewerMinor {
        kind: String,
        found: Version,
        newest: Version,
    },

    /// Another major than the one this reader knows.
    #[error(
        "this is {kind} {found}; this duet reads {kind} {reads}.x; `duet convert` reads it from a duet that knows {found}"
    )]
    OtherMajor {
        kind: String,
        found: Version,
        reads: u16,
    },

    /// The body (or a stream's head or record) did not read as the kind's type. serde's message
    /// names the field: an unknown one is refused by name here.
    #[error("{kind} {version}: {part} did not read: {source}")]
    Body {
        kind: &'static str,
        version: Version,
        part: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// A stream has no first line, so no envelope.
    #[error("the stream is empty: its first line must be the envelope")]
    EmptyStream,

    /// A line of a stream was refused. `line` is 1-based.
    #[error("line {line}: {source}")]
    Line {
        line: usize,
        #[source]
        source: Box<Error>,
    },

    /// A body, head or record did not serialize.
    #[error("{kind}: did not write: {source}")]
    Write {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

/// A `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
