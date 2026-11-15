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

mod envelope;
mod error;
mod frame;
mod kind;
mod version;

pub use envelope::{Envelope, From, Notice, SCHEMA_BASE, Tool, schema_url};
pub use error::{Error, Result};
pub use frame::{Document, Stream, read_envelope};
pub use kind::{Kind, StreamKind};
pub use version::Version;

#[cfg(test)]
mod tests;
