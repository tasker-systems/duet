//! Every kind this build knows, and a document of any of them.

use duet_core::Song;
use serde_json::Value;

use crate::schema::{document_schema, envelope_schema};
use crate::{Document, Error, Kind, Result, Version, read_envelope};

/// How a kind's files are framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Framing {
    /// A `.duet` document, with a `body`.
    Document,
    /// A `.duet.jsonl` stream, whose first line has a `head`.
    Stream,
}

/// One kind this build knows: its name, the version it reads up to and writes, its framing, and
/// its schema as committed under `schemas/`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct KindInfo {
    pub name: &'static str,
    pub version: Version,
    pub framing: Framing,
    /// Generates the committed schema at `version`.
    pub schema: fn() -> Value,
}

const fn document<K: Kind>() -> KindInfo {
    KindInfo {
        name: K::NAME,
        version: K::VERSION,
        framing: Framing::Document,
        schema: document_schema::<K>,
    }
}

/// Every kind this build reads and writes.
pub const KINDS: &[KindInfo] = &[document::<Song>()];

/// The envelope's schema, as a [`KindInfo`] whose name is [`ENVELOPE`](crate::ENVELOPE). It is
/// no kind, but the gate commits and checks it like one.
pub const ENVELOPE_INFO: KindInfo = KindInfo {
    name: crate::ENVELOPE,
    version: crate::ENVELOPE_VERSION,
    framing: Framing::Document,
    schema: envelope_schema,
};

/// The kind named `name`, if this build knows it.
#[must_use]
pub fn kind(name: &str) -> Option<&'static KindInfo> {
    KINDS.iter().find(|info| info.name == name)
}

/// A `.duet` document of any kind this build knows, read by its envelope.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AnyDocument {
    Song(Document<Song>),
}

impl AnyDocument {
    /// Reads a `.duet` document of whichever kind its envelope names, by that kind's version
    /// policy.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownKind`] for a kind this build does not know, [`Error::Framing`] for a
    /// stream's kind, and any error of [`Document::read`].
    pub fn read(text: &str) -> Result<Self> {
        let envelope = read_envelope(text)?;
        match kind(&envelope.kind) {
            Some(info) if info.name == Song::NAME => Document::read(text).map(Self::Song),
            Some(info) if info.framing == Framing::Stream => Err(Error::Framing {
                found: "document (.duet, with a `body`)",
                expected: "stream (.duet.jsonl), as its kind is",
            }),
            _ => Err(Error::UnknownKind {
                found: envelope.kind,
                known: KINDS
                    .iter()
                    .map(|info| info.name)
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }

    /// The kind's name.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Song(_) => Song::NAME,
        }
    }

    /// Writes the document, as [`Document::write`] does.
    ///
    /// # Errors
    ///
    /// When the body does not serialize.
    pub fn write(&self) -> Result<String> {
        match self {
            Self::Song(doc) => doc.write(),
        }
    }
}
