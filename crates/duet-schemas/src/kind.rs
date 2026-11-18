//! The `Kind` trait: a data structure with a name and a schema version.

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Version;

/// A data structure that is a kind of duet document: a body type with a name and a version.
///
/// For a document kind (a `.duet` file), `Self` is the body. For a stream kind (a `.duet.jsonl`
/// file), `Self` is the head of its first line, and [`StreamKind::Record`] is each later line.
///
/// The body serializes exactly as the type does: the envelope adds nothing to it. A body type
/// denies unknown fields (`#[serde(deny_unknown_fields)]`), so that a misspelled or newer field
/// is refused by name rather than dropped. Its `JsonSchema` is the committed schema's body, so it
/// must describe what the type writes: derived, or written by hand for a type serialized by hand.
pub trait Kind: Serialize + DeserializeOwned + JsonSchema {
    /// The kind's name, lowercase with hyphens: `part`, `drum-grid`.
    const NAME: &'static str;
    /// The version this build reads up to and writes.
    const VERSION: Version;

    /// Reads a body written at `minor`, an older or the current minor of [`Self::VERSION`]'s
    /// major. The version policy has refused every other version before this is called.
    ///
    /// The default parses it as the current type, which is right while every minor is additive:
    /// a field a newer minor added is optional, and absent in an older body.
    ///
    /// # Errors
    ///
    /// When the body does not read as `Self`.
    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        serde_json::from_value(body)
    }
}

/// A kind written as a stream: the envelope and `Self` (the head) on the first line, then one
/// [`Record`](StreamKind::Record) per line, each of which reads on its own.
pub trait StreamKind: Kind {
    /// One line after the first.
    type Record: Serialize + DeserializeOwned + JsonSchema;

    /// Reads a record written at `minor`, as [`Kind::read_minor`] reads a head.
    ///
    /// # Errors
    ///
    /// When the record does not read as [`Self::Record`].
    fn read_minor_record(
        minor: u16,
        record: serde_json::Value,
    ) -> serde_json::Result<Self::Record> {
        let _ = minor;
        serde_json::from_value(record)
    }
}
