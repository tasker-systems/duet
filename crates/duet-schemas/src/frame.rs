//! The two framings: a `.duet` document, and a `.duet.jsonl` stream.

use std::fmt;

use serde::Serialize;
use serde::de::Error as _;
use serde_json::{Map, Value};

use crate::{Envelope, Error, From, Kind, Result, StreamKind, Tool, Version};

const SCHEMA: &str = "$schema";
const DUET: &str = "duet";
const ENVELOPE_FIELDS: [&str; 5] = ["kind", "version", "tool", "from", "statements"];

/// Which field holds a file's value: `body` in a document, `head` on a stream's first line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Body,
    Head,
}

impl Part {
    const fn key(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Head => "head",
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::Body => Self::Head,
            Self::Head => Self::Body,
        }
    }

    const fn frame(self) -> &'static str {
        match self {
            Self::Body => "document (.duet, with a `body`)",
            Self::Head => "stream (.duet.jsonl, whose first line has a `head`)",
        }
    }
}

/// Parses `text` as one JSON object, and returns it with its envelope read and checked against
/// its own `$schema`. Every field it holds is one the frame knows: `$schema`, `duet`, and `part`
/// if given, or either part if not.
fn open(text: &str, part: Option<Part>) -> Result<(Envelope, Map<String, Value>)> {
    let value: Value = serde_json::from_str(text).map_err(Error::Json)?;
    let Value::Object(mut map) = value else {
        return Err(Error::Json(serde_json::Error::custom(
            "a duet document is a JSON object",
        )));
    };
    for key in map.keys() {
        let known = [SCHEMA, DUET, Part::Body.key(), Part::Head.key()].contains(&key.as_str());
        if !known {
            return Err(Error::UnknownField {
                field: key.clone(),
                at: "the document",
            });
        }
        if let Some(part) = part
            && key == part.other().key()
        {
            return Err(Error::Framing {
                found: part.other().frame(),
                expected: part.frame(),
            });
        }
    }
    let duet = map.remove(DUET).ok_or(Error::Missing {
        field: DUET,
        at: "the document",
    })?;
    if let Value::Object(fields) = &duet
        && let Some(field) = fields
            .keys()
            .find(|k| !ENVELOPE_FIELDS.contains(&k.as_str()))
    {
        return Err(Error::UnknownField {
            field: field.clone(),
            at: "the envelope (`duet`)",
        });
    }
    let envelope: Envelope = serde_json::from_value(duet).map_err(Error::Envelope)?;
    if let Some(schema) = map.remove(SCHEMA) {
        let expected = envelope.schema_url();
        if schema.as_str() != Some(expected.as_str()) {
            return Err(Error::SchemaMismatch {
                found: match schema {
                    Value::String(s) => s,
                    other => other.to_string(),
                },
                kind: envelope.kind,
                version: envelope.version,
                expected,
            });
        }
    }
    Ok((envelope, map))
}

/// Reads the envelope of a `.duet` document, or of a `.duet.jsonl` stream's first line, without
/// reading its body: what a reader dispatches on.
///
/// `text` is one JSON value. It checks the frame and `$schema`, but not the version policy,
/// which belongs to whichever kind the envelope names.
///
/// # Errors
///
/// When `text` is not one JSON object holding a valid envelope, holds a field the frame does not
/// know, or has a `$schema` that disagrees with its kind and version.
pub fn read_envelope(text: &str) -> Result<Envelope> {
    open(text, None).map(|(envelope, _)| envelope)
}

/// The version policy: the minor to read `envelope`'s body at, if this build of `K` reads it.
fn policy<K: Kind>(envelope: &Envelope) -> Result<u16> {
    let found = envelope.version;
    if envelope.kind != K::NAME {
        return Err(Error::WrongKind {
            found: envelope.kind.clone(),
            expected: K::NAME,
        });
    }
    if found.major != K::VERSION.major {
        return Err(Error::OtherMajor {
            kind: envelope.kind.clone(),
            found,
            reads: K::VERSION.major,
        });
    }
    if found.minor > K::VERSION.minor {
        return Err(Error::NewerMinor {
            kind: envelope.kind.clone(),
            found,
            newest: K::VERSION,
        });
    }
    Ok(found.minor)
}

/// Opens `text` as `part` of kind `K`, applying the version policy, and returns the envelope, the
/// minor it was written at and the part's value.
fn open_kind<K: Kind>(text: &str, part: Part) -> Result<(Envelope, u16, Value)> {
    let (envelope, mut map) = open(text, Some(part))?;
    let minor = policy::<K>(&envelope)?;
    let value = map.remove(part.key()).ok_or(Error::Missing {
        field: part.key(),
        at: "the document",
    })?;
    Ok((envelope, minor, value))
}

const fn body_error(
    kind: &'static str,
    version: Version,
    part: &'static str,
) -> impl FnOnce(serde_json::Error) -> Error {
    move |source| Error::Body {
        kind,
        version,
        part,
        source,
    }
}

/// The whole of a `.duet` file or of a stream's first line, in the order it is written:
/// `$schema`, then the envelope, then the body or head.
#[derive(Serialize)]
struct Framed<'a, T> {
    #[serde(rename = "$schema")]
    schema: String,
    duet: &'a Envelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<&'a T>,
}

impl<'a, T: Serialize> Framed<'a, T> {
    fn new(envelope: &'a Envelope, part: Part, value: &'a T) -> Self {
        Self {
            schema: envelope.schema_url(),
            duet: envelope,
            body: (part == Part::Body).then_some(value),
            head: (part == Part::Head).then_some(value),
        }
    }
}

fn envelope_for<K: Kind>(tool: &Tool, from: Option<&From>, statements: &[String]) -> Envelope {
    Envelope {
        kind: K::NAME.to_string(),
        version: K::VERSION,
        tool: tool.clone(),
        from: from.cloned(),
        statements: statements.to_vec(),
    }
}

/// A `.duet` file: a JSON document of kind `K`.
///
/// ```json
/// {
///   "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/part/1.0.json",
///   "duet": { "kind": "part", "version": "1.0", "tool": { "name": "duet", "version": "0.3.0" } },
///   "body": { "ticks_per_quarter": 480, "tempo_bpm": 92.0, "time_sig": "4/4", "tracks": [] }
/// }
/// ```
///
/// It holds the envelope's parts that are not fixed by `K`. The kind and version are `K`'s, so
/// a document read at an older minor writes back at the version this build speaks, with its body
/// unchanged.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Document<K> {
    /// What wrote it. A tool writing a document it read sets this to itself.
    pub tool: Tool,
    /// Where the body came from, if it was imported. Kept on write-back.
    pub from: Option<From>,
    /// What the writer inferred or changed and must say.
    pub statements: Vec<String>,
    pub body: K,
}

impl<K: Kind> Document<K> {
    /// `body`, written by `tool`, with no `from` and no statements.
    #[must_use]
    pub const fn new(tool: Tool, body: K) -> Self {
        Self {
            tool,
            from: None,
            statements: Vec::new(),
            body,
        }
    }

    /// The envelope this document is written with: `K`'s name and version.
    #[must_use]
    pub fn envelope(&self) -> Envelope {
        envelope_for::<K>(&self.tool, self.from.as_ref(), &self.statements)
    }

    /// Reads a `.duet` document of kind `K`, by the version policy: this build's version and
    /// every older minor of its major read; anything else is refused by name.
    ///
    /// # Errors
    ///
    /// [`Error::WrongKind`], [`Error::NewerMinor`], [`Error::OtherMajor`],
    /// [`Error::UnknownField`], [`Error::SchemaMismatch`], or [`Error::Body`] (naming the field
    /// when the body holds one its type does not know), and any error of the frame.
    pub fn read(text: &str) -> Result<Self> {
        let (envelope, minor, body) = open_kind::<K>(text, Part::Body)?;
        let body = K::read_minor(minor, body).map_err(body_error(
            K::NAME,
            envelope.version,
            "the body",
        ))?;
        Ok(Self {
            tool: envelope.tool,
            from: envelope.from,
            statements: envelope.statements,
            body,
        })
    }

    /// Writes the document as pretty-printed JSON with a final newline.
    ///
    /// # Errors
    ///
    /// When the body does not serialize.
    pub fn write(&self) -> Result<String> {
        let envelope = self.envelope();
        let mut text =
            serde_json::to_string_pretty(&Framed::new(&envelope, Part::Body, &self.body)).map_err(
                |source| Error::Write {
                    kind: K::NAME,
                    source,
                },
            )?;
        text.push('\n');
        Ok(text)
    }
}

/// A `.duet.jsonl` file: one JSON value per line. The first line is the envelope with a `head`;
/// every later line is one record, and nothing else, so a stream can be appended to, cut at any
/// line, or read as it is written.
#[non_exhaustive]
pub struct Stream<K: StreamKind> {
    /// What wrote it.
    pub tool: Tool,
    /// Where it came from, if it was imported.
    pub from: Option<From>,
    /// What the writer inferred or changed and must say.
    pub statements: Vec<String>,
    /// The stream's own header.
    pub head: K,
    /// Every line after the first, in order.
    pub records: Vec<K::Record>,
}

impl<K: StreamKind + fmt::Debug> fmt::Debug for Stream<K>
where
    K::Record: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("tool", &self.tool)
            .field("from", &self.from)
            .field("statements", &self.statements)
            .field("head", &self.head)
            .field("records", &self.records)
            .finish()
    }
}

impl<K: StreamKind + Clone> Clone for Stream<K>
where
    K::Record: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tool: self.tool.clone(),
            from: self.from.clone(),
            statements: self.statements.clone(),
            head: self.head.clone(),
            records: self.records.clone(),
        }
    }
}

impl<K: StreamKind + PartialEq> PartialEq for Stream<K>
where
    K::Record: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.tool == other.tool
            && self.from == other.from
            && self.statements == other.statements
            && self.head == other.head
            && self.records == other.records
    }
}

impl<K: StreamKind> Stream<K> {
    /// A stream with `head`, written by `tool`, and no records yet.
    #[must_use]
    pub const fn new(tool: Tool, head: K) -> Self {
        Self {
            tool,
            from: None,
            statements: Vec::new(),
            head,
            records: Vec::new(),
        }
    }

    /// The envelope this stream is written with: `K`'s name and version.
    #[must_use]
    pub fn envelope(&self) -> Envelope {
        envelope_for::<K>(&self.tool, self.from.as_ref(), &self.statements)
    }

    /// Reads a whole `.duet.jsonl` stream of kind `K`, by the version policy, as
    /// [`Document::read`] reads a document. Every record is read at the minor the first line
    /// names.
    ///
    /// # Errors
    ///
    /// [`Error::EmptyStream`], or [`Error::Line`] naming the 1-based line and wrapping why it
    /// was refused: on the first line, any error [`Document::read`] gives; on a later line, the
    /// JSON or the record not reading.
    pub fn read(text: &str) -> Result<Self> {
        let mut lines = text.lines().enumerate();
        let at = |line: usize| {
            move |source| Error::Line {
                line: line + 1,
                source: Box::new(source),
            }
        };
        let (n, first) = lines.next().ok_or(Error::EmptyStream)?;
        let (envelope, minor, head) = open_kind::<K>(first, Part::Head).map_err(at(n))?;
        let version = envelope.version;
        let head = K::read_minor(minor, head)
            .map_err(body_error(K::NAME, version, "the head"))
            .map_err(at(n))?;
        let records = lines
            .map(|(n, line)| {
                serde_json::from_str(line)
                    .map_err(Error::Json)
                    .and_then(|value| {
                        K::read_minor_record(minor, value).map_err(body_error(
                            K::NAME,
                            version,
                            "a record",
                        ))
                    })
                    .map_err(at(n))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            tool: envelope.tool,
            from: envelope.from,
            statements: envelope.statements,
            head,
            records,
        })
    }

    /// The first line: `$schema`, the envelope and the head, with its newline.
    ///
    /// # Errors
    ///
    /// When the head does not serialize.
    pub fn head_line(&self) -> Result<String> {
        let envelope = self.envelope();
        line(K::NAME, &Framed::new(&envelope, Part::Head, &self.head))
    }

    /// One later line: `record`, with its newline. A writer appends these to the head line as the
    /// records are made.
    ///
    /// # Errors
    ///
    /// When the record does not serialize.
    pub fn record_line(record: &K::Record) -> Result<String> {
        line(K::NAME, record)
    }

    /// Writes the whole stream: the head line, then one line per record.
    ///
    /// # Errors
    ///
    /// When the head or a record does not serialize.
    pub fn write(&self) -> Result<String> {
        let mut text = self.head_line()?;
        for record in &self.records {
            text.push_str(&Self::record_line(record)?);
        }
        Ok(text)
    }
}

/// `value` as one line of compact JSON with its newline.
fn line<T: Serialize>(kind: &'static str, value: &T) -> Result<String> {
    let mut text = serde_json::to_string(value).map_err(|source| Error::Write { kind, source })?;
    text.push('\n');
    Ok(text)
}
