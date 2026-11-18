//! `part`: a [`PartBody`], one voice, line or groove, with its id and its source; and
//! `drum-grid`: a [`DrumGrid`], as duet-drums serializes it.

use std::borrow::Cow;

use duet_core::Part;
use duet_drums::DrumGrid;
use duet_dsp::TakeDigest;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::journal::{DocDigest, Id};
use crate::project::Name;
use crate::{Kind, Version};

/// The body of `part` 1.0: the part's stable `id`, where its notes came from (`source`), and
/// then every field of the [`Part`] itself, at the same level, exactly as `Part` serializes.
///
/// - **`id`** is minted once, when the document is made, and never changes (the owner's
///   ruling 13 in decision 01a1eb48). Renaming the file or changing the title leaves it.
/// - **`source`** says how the part was made: by hand, comped from takes, or generated
///   (ruled 2026-11-18, decision 01a1ee66).
///
/// Both came into 1.0 in place, with no minor, as did the spelling below: before a first
/// release a format changes in place, since no file exists to keep (ruling 12). A part
/// document without either is refused, naming the field.
///
/// Every pitch keeps its spelling: a letter, up to two sharps or two flats, and the letter's
/// octave, so `Cb5` (MIDI 71), `E#4`, `F##4` and `Bbb3` are written and read as spelled. The
/// pitch pattern refuses what no spelling keeps (`C###4`, `C#b4`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct PartBody {
    /// The part's stable id: a UUIDv7, minted when the document was made.
    pub id: Id,
    /// How the part was made.
    pub source: Source,
    /// The part's notes and conductor.
    #[serde(flatten)]
    pub part: Part,
}

impl PartBody {
    #[must_use]
    pub fn new(id: Id, source: Source, part: Part) -> Self {
        Self { id, source, part }
    }
}

/// Reads `id` and `source`, then the rest as a [`Part`], which refuses any field it does not
/// know, by name. Serde's `flatten` would drop an unknown field instead, so this is written out.
impl<'de> Deserialize<'de> for PartBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let mut take = |name: &'static str, why: &str| {
            fields
                .remove(name)
                .ok_or_else(|| D::Error::custom(format!("missing field `{name}`: {why}")))
        };
        let id = take(
            "id",
            "a part has a stable id, a UUIDv7 minted when its document was made",
        )?;
        let source = take(
            "source",
            "a part states its source, {\"by\": \"hand\"}, {\"by\": \"comped\", ...} or \
             {\"by\": \"generated\", ...}",
        )?;
        let id: Id = serde_json::from_value(id).map_err(D::Error::custom)?;
        let source: Source =
            serde_json::from_value(source).map_err(|e| D::Error::custom(format!("source: {e}")))?;
        source.check().map_err(D::Error::custom)?;
        let part: Part = serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(Self { id, source, part })
    }
}

/// The part's own schema, with `id` and `source` added as required properties.
impl JsonSchema for PartBody {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PartBody")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::part::PartBody")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = Part::json_schema(generator);
        let id = generator.subschema_for::<Id>().to_value();
        let source = generator.subschema_for::<Source>().to_value();
        let fields = schema
            .as_object_mut()
            .expect("a struct's schema is an object");
        fields.insert("title".into(), Value::from("PartBody"));
        fields.insert(
            "description".into(),
            Value::from(
                "A part: its stable id, how it was made, and its notes and conductor, every \
                 field at one level.",
            ),
        );
        if let Some(Value::Object(properties)) = fields.get_mut("properties") {
            properties.insert("id".into(), id);
            properties.insert("source".into(), source);
        }
        if let Some(Value::Array(required)) = fields.get_mut("required") {
            required.insert(0, Value::from("source"));
            required.insert(0, Value::from("id"));
        }
        schema
    }
}

/// How a part was made (decision 01a1ee66, the songs spec's §2). Written as an object whose `by`
/// says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "lowercase", deny_unknown_fields)]
#[non_exhaustive]
pub enum Source {
    /// Written by hand, by the owner or by Claude: in duet, or in another program and imported
    /// (the envelope's `from` says from what).
    Hand {},
    /// Comped from takes: a reading of each take it drew from, and the comp when there is one.
    /// A take's reading kept as a part is a one-take comp with no comp: the simple case.
    Comped {
        /// The takes the notes came from, at least one.
        #[schemars(length(min = 1))]
        takes: Vec<CompedTake>,
        /// The comp in the song's `comps/` that wrote the part, by its name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        comp: Option<Name>,
    },
    /// Generated by an operation, which names itself and what it read, so it can be made again.
    Generated {
        /// The operation, as its command names it, such as `drums render`.
        #[schemars(length(min = 1))]
        operation: String,
        /// The digest of each document the operation read, in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<DocDigest>,
    },
}

impl Source {
    /// A part written by hand.
    #[must_use]
    pub const fn hand() -> Self {
        Self::Hand {}
    }

    /// A part comped from one take: its reading, as capture writes it.
    #[must_use]
    pub fn reading(digest: TakeDigest, reading: Option<String>) -> Self {
        Self::Comped {
            takes: vec![CompedTake {
                digest,
                take: None,
                reading,
            }],
            comp: None,
        }
    }

    /// Checks what a schema cannot say: a comped part names at least one take, and a generated
    /// part names its operation.
    ///
    /// # Errors
    ///
    /// A message naming what is missing.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Self::Comped { takes, .. } if takes.is_empty() => {
                Err("source: a comped part names the takes it was comped from, at least one".into())
            }
            Self::Generated { operation, .. } if operation.trim().is_empty() => {
                Err("source: a generated part names the operation that generated it".into())
            }
            _ => Ok(()),
        }
    }

    /// The word `by` holds: `hand`, `comped` or `generated`.
    #[must_use]
    pub const fn by(&self) -> &'static str {
        match self {
            Self::Hand {} => "hand",
            Self::Comped { .. } => "comped",
            Self::Generated { .. } => "generated",
        }
    }
}

/// One take a comped part drew from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CompedTake {
    /// The take's identity: the digest of its decoded samples.
    pub digest: TakeDigest,
    /// The take's name in the project, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<Name>,
    /// The reader whose reading the notes are, such as `rules`, `hmm` or `basic-pitch`.
    /// Absent for a rhythm take, which has one reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<String>,
}

impl CompedTake {
    #[must_use]
    pub fn new(digest: TakeDigest) -> Self {
        Self {
            digest,
            take: None,
            reading: None,
        }
    }
}

/// `part` 1.0's body is a [`PartBody`]. Its source is checked as it is read
/// ([`Source::check`]).
impl Kind for PartBody {
    const NAME: &'static str = "part";
    const VERSION: Version = Version::new(1, 0);
}

/// `drum-grid` 1.0's body is what `DrumGrid` serializes to: the conductor, the grid, channel,
/// levels and lanes, then `bar_steps` and `vel`, one lanes × steps tensor for the whole grid.
///
/// A body is checked as it is read ([`DrumGrid::check`]): a `vel` whose shape disagrees with
/// the lanes or the bars, or bar steps the meter does not give, are refused by name.
impl Kind for DrumGrid {
    const NAME: &'static str = "drum-grid";
    const VERSION: Version = Version::new(1, 0);

    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        let grid: DrumGrid = serde_json::from_value(body)?;
        grid.check().map_err(serde::de::Error::custom)?;
        Ok(grid)
    }
}
