//! Every kind this build knows, and a document of any of them.

use duet_capture::Analysis;
use duet_drums::DrumGrid;
use duet_dsp::PerformanceHead;
use serde_json::Value;

use crate::journal::{IndexHead, JournalHead};
use crate::part::PartBody;
use crate::project::Project;
use crate::schema::{document_schema, envelope_schema, stream_schema};
use crate::snapshot::Snapshot;
use crate::song::Song;
use crate::{Document, Error, Kind, Result, StreamKind, Version, read_envelope};

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

const fn stream<K: StreamKind>() -> KindInfo {
    KindInfo {
        name: K::NAME,
        version: K::VERSION,
        framing: Framing::Stream,
        schema: stream_schema::<K>,
    }
}

/// Every kind this build reads and writes.
pub const KINDS: &[KindInfo] = &[
    document::<PartBody>(),
    document::<DrumGrid>(),
    document::<Analysis>(),
    document::<Project>(),
    document::<Song>(),
    document::<Snapshot>(),
    stream::<PerformanceHead>(),
    stream::<JournalHead>(),
    stream::<IndexHead>(),
];

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
    Part(Document<PartBody>),
    DrumGrid(Document<DrumGrid>),
    /// Boxed: an analysis holds a take's every event and its reading, and is far the largest.
    CaptureAnalysis(Box<Document<Analysis>>),
    Project(Document<Project>),
    Song(Document<Song>),
    Snapshot(Document<Snapshot>),
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
            Some(info) if info.name == PartBody::NAME => Document::read(text).map(Self::Part),
            Some(info) if info.name == DrumGrid::NAME => Document::read(text).map(Self::DrumGrid),
            Some(info) if info.name == Analysis::NAME => {
                Document::read(text).map(|doc| Self::CaptureAnalysis(Box::new(doc)))
            }
            Some(info) if info.name == Project::NAME => Document::read(text).map(Self::Project),
            Some(info) if info.name == Song::NAME => Document::read(text).map(Self::Song),
            Some(info) if info.name == Snapshot::NAME => Document::read(text).map(Self::Snapshot),
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
            Self::Part(_) => PartBody::NAME,
            Self::DrumGrid(_) => DrumGrid::NAME,
            Self::CaptureAnalysis(_) => Analysis::NAME,
            Self::Project(_) => Project::NAME,
            Self::Song(_) => Song::NAME,
            Self::Snapshot(_) => Snapshot::NAME,
        }
    }

    /// Writes the document, as [`Document::write`] does.
    ///
    /// # Errors
    ///
    /// When the body does not serialize.
    pub fn write(&self) -> Result<String> {
        match self {
            Self::Part(doc) => doc.write(),
            Self::DrumGrid(doc) => doc.write(),
            Self::CaptureAnalysis(doc) => doc.write(),
            Self::Project(doc) => doc.write(),
            Self::Song(doc) => doc.write(),
            Self::Snapshot(doc) => doc.write(),
        }
    }
}
