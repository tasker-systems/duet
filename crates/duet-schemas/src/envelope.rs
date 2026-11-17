//! The envelope: the `duet` object that says what a file is.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Version;

/// Where every kind's committed schema lives: `<SCHEMA_BASE><kind>/<major>.<minor>.json`.
pub const SCHEMA_BASE: &str =
    "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/";

/// The `$schema` (and the schema's `$id`) of `kind` at `version`.
///
/// ```
/// use duet_schemas::{schema_url, Version};
/// assert_eq!(
///     schema_url("part", Version::new(1, 0)),
///     "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/part/1.0.json",
/// );
/// ```
#[must_use]
pub fn schema_url(kind: &str, version: Version) -> String {
    format!("{SCHEMA_BASE}{kind}/{version}.json")
}

/// The name and version of what wrote a file. Information only: no reader checks it or
/// branches on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Tool {
    pub name: String,
    pub version: String,
}

impl Tool {
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// Where a body came from, when it was imported from an outside format (ABC, MIDI).
///
/// It is a fact about this file, not about the music, so it lives in the envelope: two
/// documents with the same body are the same part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct From {
    /// The outside format: `abc` or `midi`.
    pub format: String,
    /// Which tune of an ABC book, by its `X:` number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tune: Option<String>,
    /// What the outside format's reader read but did not carry into the body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Notice>,
}

impl From {
    /// Came from `format`, with no tune and no notices.
    #[must_use]
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            tune: None,
            notices: Vec::new(),
        }
    }
}

/// A construct an outside format's reader read but did not carry, such as an ABC slur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Notice {
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub col: u32,
    /// The kind of construct, such as `slur` or `annotation`.
    pub construct: String,
    /// The text that was found.
    pub found: String,
}

impl Notice {
    #[must_use]
    pub fn new(
        line: u32,
        col: u32,
        construct: impl Into<String>,
        found: impl Into<String>,
    ) -> Self {
        Self {
            line,
            col,
            construct: construct.into(),
            found: found.into(),
        }
    }
}

/// The `duet` object of a `.duet` file, or of a `.duet.jsonl` stream's first line.
///
/// It is read before the body, so a reader can tell what a file is before it parses the rest
/// ([`read_envelope`](crate::read_envelope)). A [`Document`](crate::Document) or
/// [`Stream`](crate::Stream) builds it on write from its kind, so a writer always writes the
/// version it speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Envelope {
    /// The kind's name, lowercase with hyphens: `part`, `drum-grid`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9]*(-[a-z0-9]+)*$"))]
    pub kind: String,
    /// The kind's schema version the body is written at.
    pub version: Version,
    /// What wrote the file.
    pub tool: Tool,
    /// Where the body came from, if it was imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<From>,
    /// What the tool that wrote the body inferred or changed and must say, one sentence each:
    /// "tempo 92, estimated (confidence 0.81)". Every view shows them as comments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statements: Vec<String>,
}

impl Envelope {
    /// `kind` at `version`, written by `tool`, with no `from` and no statements.
    #[must_use]
    pub fn new(kind: impl Into<String>, version: Version, tool: Tool) -> Self {
        Self {
            kind: kind.into(),
            version,
            tool,
            from: None,
            statements: Vec::new(),
        }
    }

    /// The `$schema` this envelope's kind and version name.
    #[must_use]
    pub fn schema_url(&self) -> String {
        schema_url(&self.kind, self.version)
    }
}
