//! duet's documents: each data structure is a **kind** with a schema and an explicit version,
//! and a file says what it is in an **envelope** rather than by its name.
//!
//! # The two framings
//!
//! - **`.duet`** is one JSON document: `$schema`, the envelope (`duet`), then the `body`
//!   ([`Document`]).
//! - **`.duet.jsonl`** is a stream: the first line is `$schema`, the envelope and a `head`; every
//!   later line is one record of the kind, and nothing else ([`Stream`]).
//!
//! The envelope ([`Envelope`]) gives the kind's name and schema [`Version`], the [`Tool`] that
//! wrote the file, where the body came from if it was imported ([`From`]), and the statements
//! the writer must make about what it inferred or changed. The body is the kind's Rust type,
//! serialized exactly as the type serializes: the envelope adds nothing to it.
//!
//! `$schema` is the URL of the file's committed schema ([`schema_url`]). A reader refuses a file
//! whose `$schema` disagrees with its kind and version.
//!
//! # The version policy
//!
//! - A reader accepts its own version and every older minor of the same major, and reads an
//!   older body with [`Kind::read_minor`].
//! - It refuses, by name: a newer minor ([`Error::NewerMinor`]), since reading it would drop
//!   the fields it does not know and writing it back would lose them; another major
//!   ([`Error::OtherMajor`]), pointing at `duet convert`; a field it does not know
//!   ([`Error::UnknownField`] in the frame, [`Error::Body`] naming it in a body); and a
//!   `$schema` that disagrees with the kind and version ([`Error::SchemaMismatch`]).
//! - A writer writes the version it speaks: a [`Document`] takes its kind and version from its
//!   [`Kind`], never from the file it was read from.
//!
//! - **A change to the envelope's shape is a new major version of every kind.** The envelope has
//!   no version of its own in a file: every kind's schema names the one envelope schema
//!   ([`ENVELOPE_VERSION`]), and a reader refuses a field it does not know. So a field added to
//!   the envelope, to [`Tool`], [`From`] or [`Notice`] is one that every older reader refuses.
//!
//! # The schemas, and the gate
//!
//! Each kind's schema, and the envelope's, is generated from the Rust types with `schemars`
//! ([`schema`]) and committed under `crates/duet-schemas/schemas/<kind>/<major>.<minor>.json`,
//! with an example document beside it in `examples/<major>.<minor>.duet` (`.duet.jsonl` for a
//! stream's kind, whose every later line the gate validates on its own against the schema's
//! `$defs/Record`). The `$id` of each is its [`schema_url`]. The gate (`tests/schemas.rs`) fails when a schema's shape changes
//! without a version change, and asks only to regenerate when its descriptions alone change:
//! `DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas` writes them.
//!
//! The kinds this build knows are [`KINDS`]: `part` 1.0 ([`PartBody`]: a part, its id and its
//! source) and `capture-analysis` 1.0
//! (duet-capture's [`Analysis`](duet_capture::Analysis)), documents, and `performance` 1.0, a
//! stream of duet-dsp's [`PerformanceHead`](duet_dsp::PerformanceHead) and
//! [`FrameBlock`](duet_dsp::FrameBlock)s, and a project's journal ([`journal`]): `journal-index`
//! 1.0, the index over its segments, and `journal` 1.0, one segment, a stream of entries, each
//! one event; `project` 1.0 ([`Project`]), a project's document, `.duet/project.duet`; and
//! `song` 1.0 ([`Song`]), a song and its composition, `songs/<song>/song.duet`; and `snapshot`
//! 1.0 ([`Snapshot`]), what one of a song's snapshots holds,
//! `songs/<song>/snapshots/<snapshot>/snapshot.duet`.
//! [`AnyDocument`] reads a `.duet` of any document kind by its envelope.
//!
//! **The journal's events version on their own** ([`EVENTS`]), each schema committed under
//! `schemas/journal/events/<event>/<major>.<minor>.json` and gated like a kind's. A reader keeps
//! an entry of an event kind it does not know, or of a newer minor, verbatim, and states the
//! skip: the one departure from the policy below, since a journal is never written back
//! ([`journal`] gives the reason).
//!
//! # The rule
//!
//! **No crate below `duet-schemas` reads or writes an enveloped document.** The kind crates
//! (duet-core, duet-drums, duet-dsp, duet-capture) define their types and serialize them.
//! Putting a type in an envelope, and taking it out, happens only here, and the [`Kind`] impls
//! for their types live here. Everything that writes a document (duet-cli, later an MCP server)
//! does it through this crate.
//!
//! # Exact numbers
//!
//! serde_json is built with `float_roundtrip`, so every finite `f32` and `f64` a document holds
//! reads back to the same bits.

#![forbid(unsafe_code)]

mod capture;
mod envelope;
mod error;
mod frame;
pub mod journal;
mod kind;
pub mod part;
mod performance;
pub mod project;
mod registry;
pub mod schema;
pub mod snapshot;
pub mod song;
mod version;

pub use envelope::{Envelope, From, Notice, SCHEMA_BASE, Tool, schema_url};
pub use error::{Error, Result};
pub use frame::{Document, Stream, read_envelope};
pub use journal::{
    EVENTS, EventInfo, EventKind, IndexHead, Journal, JournalHead, JournalIndex, event_kind,
};
pub use kind::{Kind, StreamKind};
pub use part::{PartBody, Source};
pub use project::Project;
pub use registry::{AnyDocument, ENVELOPE_INFO, Framing, KINDS, KindInfo, kind};
pub use schema::{ENVELOPE, ENVELOPE_VERSION};
pub use snapshot::Snapshot;
pub use song::Song;
pub use version::Version;

#[cfg(test)]
mod tests;
