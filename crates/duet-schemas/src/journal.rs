//! A project's journal: the shared, ordered account of what the owner and Claude did and asked of
//! each other. It is an **index** over **segments** (bus design decisions 16–23):
//!
//! - **`journal-index`** ([`IndexHead`], [`JournalIndex`]) is `journal.duet.jsonl`. Every line
//!   after its head opens a segment, and, after the first, closes the one before it with a
//!   summary ([`Closes`]). No line is ever rewritten, so the last line is always the one open
//!   entry: the current segment.
//! - **`journal`** ([`JournalHead`], [`Journal`]) is one segment, `journal.<stamp>.duet.jsonl`
//!   ([`SegmentStamp`]). Its events are what follows.
//!
//! A segment's first line is the envelope with a [`JournalHead`]. Every later line is one **entry**: an
//! [`Id`], when it was written ([`At`]), who wrote it ([`By`]), what caused it (optional), and
//! one **event**: its name (`event`), its own version (`v`) and its `body`.
//!
//! # Event kinds version on their own
//!
//! An entry's frame (`id`, `at`, `by`, `cause`, `event`, `v`, `body`) is `journal`'s, and a field
//! added to it is a new minor of `journal`. Each event kind versions on its own, by the same
//! `major.minor` rule ([`EventKind`], [`EVENTS`]). A new event kind, or a new minor of one, does
//! not change the journal's version: a journal is appended for months by every duet the owner
//! runs, older ones included, and appending never rewrites the head.
//!
//! **The departure from the version policy.** A reader refuses a newer minor *of the journal*
//! itself, as for every kind. But an entry whose `event` it does not know, or whose `v` is newer
//! than it reads, is **kept verbatim and skipped**, and the skip is stated ([`Body::Skipped`]).
//! The policy refuses a newer minor because writing it back would lose the fields it does not
//! know; an append-only journal is never written back, so that reason does not hold. An unknown
//! *field* in a known event at a known version is still refused, by name.
//!
//! # Sequence
//!
//! An entry's `seq` is its position within its segment: the segment's first entry (line 2) is
//! `seq` 1. It is never stored, so a union merge of two clones cannot make two lines claim one
//! `seq`. What a line stores is its `id`, a UUIDv7, unique across machines.
//!
//! # A torn last line
//!
//! A last line with no newline is a write that never finished. [`Journal::read`] keeps it apart
//! ([`Journal::torn`]) and reads everything before it; the next writer cuts it.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use duet_dsp::TakeDigest;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::schema::event_schema;
use crate::{Error, From, Kind, Result, Stream, StreamKind, Tool, Version};

// --- The small types every entry uses ---

/// A UUIDv7, written in lowercase hyphenated form: what names an entry, an ask, and a journal.
///
/// It is unique across machines, so two clones that each appended can be merged and every line
/// keeps a name of its own. duet-schemas reads and writes one; the random bits come from the
/// writer ([`Id::from_parts`]).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(String);

impl Id {
    /// The UUIDv7 of `unix_ms` (its 48 low bits) and 74 bits of `random`, laid out as RFC 9562
    /// gives: the time, the version `7`, 12 random bits, the variant `10`, 62 random bits.
    #[must_use]
    pub fn from_parts(unix_ms: u64, random: [u8; 10]) -> Self {
        let ms = unix_ms.to_be_bytes();
        let mut b = [0u8; 16];
        b[..6].copy_from_slice(&ms[2..]);
        b[6] = 0x70 | (random[0] & 0x0f);
        b[7] = random[1];
        b[8] = 0x80 | (random[2] & 0x3f);
        b[9..].copy_from_slice(&random[3..]);
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        Self(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        ))
    }

    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

const ID_PATTERN: &str = "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";

impl FromStr for Id {
    type Err = String;

    /// Reads a UUIDv7 in lowercase hyphenated form, and nothing else, so an id has one spelling.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let b = s.as_bytes();
        let hex = |c: &u8| matches!(c, b'0'..=b'9' | b'a'..=b'f');
        let ok = b.len() == 36
            && b.iter().enumerate().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => *c == b'-',
                14 => *c == b'7',
                19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
                _ => hex(c),
            });
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "{s:?} is not an id: one is a UUIDv7, lowercase, such as \
                 0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6"
            ))
        }
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Id {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Id")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::journal::Id")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": ID_PATTERN,
            "description": "A UUIDv7, lowercase and hyphenated.",
        })
    }
}

/// When an entry was written: a UTC time to the millisecond, written
/// `2026-11-17T15:04:05.123Z`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct At {
    unix_ms: u64,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // Both fit: d is 1..=31 and m is 1..=12 by construction.
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

impl At {
    /// `unix_ms` milliseconds after 1970-01-01T00:00:00Z.
    #[must_use]
    pub const fn from_unix_ms(unix_ms: u64) -> Self {
        Self { unix_ms }
    }

    /// Milliseconds after 1970-01-01T00:00:00Z.
    #[must_use]
    pub const fn unix_ms(self) -> u64 {
        self.unix_ms
    }
}

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.unix_ms % 1000;
        let secs = self.unix_ms / 1000;
        let (days, rem) = (secs / 86_400, secs % 86_400);
        let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or(i64::MAX / 2));
        write!(
            f,
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }
}

impl fmt::Debug for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

const AT_PATTERN: &str = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z$";

impl FromStr for At {
    type Err = String;

    /// Reads exactly the form [`At`] writes, of a real time: `2026-02-30…` is refused.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let bad = || {
            format!(
                "{s:?} is not a time: one is UTC to the millisecond, such as \
                 2026-11-17T15:04:05.123Z"
            )
        };
        let b = s.as_bytes();
        let shape = b.len() == 24
            && b.iter().enumerate().all(|(i, c)| match i {
                4 | 7 => *c == b'-',
                10 => *c == b'T',
                13 | 16 => *c == b':',
                19 => *c == b'.',
                23 => *c == b'Z',
                _ => c.is_ascii_digit(),
            });
        if !shape {
            return Err(bad());
        }
        let n = |range: std::ops::Range<usize>| s[range].parse::<u32>().map_err(|_| bad());
        let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
        let (h, min, sec, ms) = (n(11..13)?, n(14..16)?, n(17..19)?, n(20..23)?);
        if y < 1970 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return Err(bad());
        }
        if h > 23 || min > 59 || sec > 59 {
            return Err(bad());
        }
        let days = days_from_civil(i64::from(y), m, d);
        let secs = days * 86_400 + i64::from(h * 3600 + min * 60 + sec);
        let unix_ms = u64::try_from(secs).map_err(|_| bad())? * 1000 + u64::from(ms);
        let at = Self { unix_ms };
        // A day past the month's end (02-30) lands in the next month: refused by the round trip.
        if at.to_string() == s {
            Ok(at)
        } else {
            Err(bad())
        }
    }
}

impl Serialize for At {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for At {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for At {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("At")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::journal::At")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": AT_PATTERN,
            "description": "A UTC time to the millisecond, such as 2026-11-17T15:04:05.123Z.",
        })
    }
}

/// Who wrote an entry, and through which face. An audit label, never authentication: nothing
/// is refused for who wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct By {
    /// Who acted: `owner`, `claude`, or what `DUET_ACTOR` names.
    pub who: String,
    /// The face they acted through: `cli`, later `server`, `app` or `mcp`.
    pub face: String,
}

impl By {
    #[must_use]
    pub fn new(who: impl Into<String>, face: impl Into<String>) -> Self {
        Self {
            who: who.into(),
            face: face.into(),
        }
    }
}

// --- The head ---

// --- Segments ---

/// A segment's stamp: its first moment, UTC to the millisecond, written in ISO 8601 basic form,
/// `20261117T150405123Z`. It names the segment's file, `journal.<stamp>.duet.jsonl`, sorts as
/// text in time order, and has no colon (bus design decision 18).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegmentStamp(At);

impl SegmentStamp {
    /// The stamp of `at`.
    #[must_use]
    pub const fn new(at: At) -> Self {
        Self(at)
    }

    /// The moment it names.
    #[must_use]
    pub const fn at(self) -> At {
        self.0
    }

    /// The stamp a millisecond later: what a writer steps to when a name is taken.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(At::from_unix_ms(self.0.unix_ms() + 1))
    }

    /// The segment's file name: `journal.<stamp>.duet.jsonl`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("journal.{self}.duet.jsonl")
    }

    /// The stamp a segment's file name gives, if it is one: `journal.duet.jsonl`, the index, is
    /// not.
    #[must_use]
    pub fn of_file_name(name: &str) -> Option<Self> {
        name.strip_prefix("journal.")?
            .strip_suffix(".duet.jsonl")?
            .parse()
            .ok()
    }
}

impl fmt::Display for SegmentStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self
            .0
            .to_string()
            .chars()
            .filter(|c| !matches!(c, '-' | ':' | '.'))
            .collect();
        f.write_str(&text)
    }
}

impl fmt::Debug for SegmentStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

const STAMP_PATTERN: &str = "^[0-9]{8}T[0-9]{9}Z$";

impl FromStr for SegmentStamp {
    type Err = String;

    /// Reads exactly the form a stamp is written in, of a real time.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let bad = || {
            format!(
                "{s:?} is not a segment stamp: one is UTC to the millisecond in ISO 8601 basic \
                 form, such as 20261117T150405123Z"
            )
        };
        let b = s.as_bytes();
        let shape = b.len() == 19
            && b.iter().enumerate().all(|(i, c)| match i {
                8 => *c == b'T',
                18 => *c == b'Z',
                _ => c.is_ascii_digit(),
            });
        if !shape {
            return Err(bad());
        }
        let at = format!(
            "{}-{}-{}T{}:{}:{}.{}Z",
            &s[0..4],
            &s[4..6],
            &s[6..8],
            &s[9..11],
            &s[11..13],
            &s[13..15],
            &s[15..18]
        );
        at.parse().map(Self).map_err(|_| bad())
    }
}

impl Serialize for SegmentStamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SegmentStamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SegmentStamp {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SegmentStamp")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::journal::SegmentStamp")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": STAMP_PATTERN,
            "description": "A segment's first moment, UTC to the millisecond in ISO 8601 basic \
                form, such as 20261117T150405123Z. Its file is journal.<stamp>.duet.jsonl.",
        })
    }
}

/// `journal` 1.0's first line holds the head: the project's name, the segment's own id, and its
/// stamp, so a segment read alone still says which it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct JournalHead {
    /// The name of the project whose journal this is.
    pub project: String,
    /// This segment's id, as the index line that opens it names it.
    pub id: Id,
    /// This segment's stamp, as its file name gives it.
    pub segment: SegmentStamp,
}

impl JournalHead {
    #[must_use]
    pub fn new(project: impl Into<String>, id: Id, segment: SegmentStamp) -> Self {
        Self {
            project: project.into(),
            id,
            segment,
        }
    }
}

/// `journal` 1.0: one segment. The head on the first line, then one [`EntryFrame`] per line.
///
/// The head's `segment` was added in place at 1.0: before a first release a format changes in
/// place (owner, 2026-11-17, the domain model's ruling 12).
impl Kind for JournalHead {
    const NAME: &'static str = "journal";
    const VERSION: Version = Version::new(1, 0);
}

/// Every later line is one entry, framed as [`EntryFrame`] gives. Its body is read by its own
/// event kind ([`read_line`]), not by the journal's version.
impl StreamKind for JournalHead {
    type Record = EntryFrame;
}

const EVENT_PATTERN: &str = r"^[a-z][a-z0-9]*(-[a-z0-9]+)*(\.[a-z][a-z0-9]*(-[a-z0-9]+)*)+$";

fn event_name_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": EVENT_PATTERN,
        "description": "The event kind's name, dotted: `ask.answered`.",
    })
}

fn body_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "description": "The event's body, read by the event kind's own schema at `v`: \
            schemas/journal/events/<event>/<v>.json.",
    })
}

/// One entry as `journal` 1.0 frames it, its body unread: what the committed schema's
/// `$defs/Record` describes. A reader reads the body by its event kind ([`read_line`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EntryFrame {
    /// The entry's own id.
    pub id: Id,
    /// When it was written.
    pub at: At,
    /// Who wrote it.
    pub by: By,
    /// The entry that caused it, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Id>,
    /// The event kind.
    #[schemars(schema_with = "event_name_schema")]
    pub event: String,
    /// The event kind's version the body is written at.
    pub v: Version,
    /// The event's body.
    #[schemars(schema_with = "body_schema")]
    pub body: Value,
}

// --- The events ---

/// An event kind: a body type with a dotted name and a version of its own.
///
/// A body denies unknown fields, so a misspelled or newer field in a known event at a known
/// version is refused by name. A minor is additive, as for every kind.
pub trait EventKind: Serialize + DeserializeOwned + JsonSchema + Into<Event> {
    /// The event kind's name, dotted: `ask.answered`.
    const NAME: &'static str;
    /// The version this build reads up to and writes.
    const VERSION: Version;

    /// Reads a body written at `minor` of [`Self::VERSION`]'s major, from its exact text.
    ///
    /// # Errors
    ///
    /// When the body does not read as `Self`; serde's message names an unknown field.
    fn read_minor(minor: u16, body: &str) -> serde_json::Result<Self> {
        let _ = minor;
        serde_json::from_str(body)
    }
}

/// One event kind this build knows: its name, the version it reads up to and writes, and its
/// schema as committed under `schemas/journal/events/<name>/`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EventInfo {
    pub name: &'static str,
    pub version: Version,
    /// Generates the committed schema of the body at `version`.
    pub schema: fn() -> Value,
    read: fn(u16, &str) -> serde_json::Result<Event>,
}

fn read_event<E: EventKind>(minor: u16, body: &str) -> serde_json::Result<Event> {
    E::read_minor(minor, body).map(Into::into)
}

const fn event<E: EventKind>() -> EventInfo {
    EventInfo {
        name: E::NAME,
        version: E::VERSION,
        schema: event_schema::<E>,
        read: read_event::<E>,
    }
}

/// Every event kind this build reads and writes.
pub const EVENTS: &[EventInfo] = &[
    event::<AskAsked>(),
    event::<AskAnswered>(),
    event::<AskWithdrawn>(),
    event::<MomentPlayed>(),
    event::<VerdictGiven>(),
    event::<TakeAdded>(),
    event::<DocWritten>(),
];

/// The event kind named `name`, if this build knows it.
#[must_use]
pub fn event_kind(name: &str) -> Option<&'static EventInfo> {
    EVENTS.iter().find(|info| info.name == name)
}

/// Which party an ask is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Party {
    Owner,
    Claude,
}

/// Whom a moment was played for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Audience {
    Owner,
    Claude,
    Both,
}

/// `ask.asked` 1.0: a question one party puts to the other. The entry's `id` is the ask's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AskAsked {
    /// The question, in the asker's words.
    pub question: String,
    /// What it is about: a reference, as duet names a take, a span of one, or a document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// The answers offered, if the ask offers a choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// What to listen to before answering: references, as `about` is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<String>,
    /// Whom it is for. Absent: the other party than the one who asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Party>,
}

impl AskAsked {
    #[must_use]
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            about: None,
            options: Vec::new(),
            clips: Vec::new(),
            to: None,
        }
    }
}

impl EventKind for AskAsked {
    const NAME: &'static str = "ask.asked";
    const VERSION: Version = Version::new(1, 0);
}

/// `ask.answered` 1.0: the answer to an ask, which it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AskAnswered {
    /// The ask answered: its entry's id.
    pub ask: Id,
    /// The answer: one of the ask's options, or the answerer's own.
    pub answer: String,
    /// The answerer's own words beside the answer, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
}

impl AskAnswered {
    #[must_use]
    pub fn new(ask: Id, answer: impl Into<String>) -> Self {
        Self {
            ask,
            answer: answer.into(),
            said: None,
        }
    }
}

impl EventKind for AskAnswered {
    const NAME: &'static str = "ask.answered";
    const VERSION: Version = Version::new(1, 0);
}

/// `ask.withdrawn` 1.0: an ask its asker takes back, unanswered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AskWithdrawn {
    /// The ask withdrawn: its entry's id.
    pub ask: Id,
    /// Why, if said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl AskWithdrawn {
    #[must_use]
    pub const fn new(ask: Id) -> Self {
        Self { ask, why: None }
    }
}

impl EventKind for AskWithdrawn {
    const NAME: &'static str = "ask.withdrawn";
    const VERSION: Version = Version::new(1, 0);
}

/// `moment.played` 1.0: a moment played for someone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MomentPlayed {
    /// What was played: a reference, as an ask's `about` is.
    pub what: String,
    /// Whom it was played for.
    #[serde(rename = "for")]
    pub for_: Audience,
    /// How it was rendered: `take`, `melody`, `steps`, `bends`, …
    pub rendering: String,
}

impl MomentPlayed {
    #[must_use]
    pub fn new(what: impl Into<String>, for_: Audience, rendering: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            for_,
            rendering: rendering.into(),
        }
    }
}

impl EventKind for MomentPlayed {
    const NAME: &'static str = "moment.played";
    const VERSION: Version = Version::new(1, 0);
}

/// `verdict.given` 1.0: the owner's verdict on something, that picks nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct VerdictGiven {
    /// What it is about: a reference, as an ask's `about` is.
    pub about: String,
    /// The verdict, in the owner's words.
    pub verdict: String,
    /// The ask it answers, if it answers one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Id>,
}

impl VerdictGiven {
    #[must_use]
    pub fn new(about: impl Into<String>, verdict: impl Into<String>) -> Self {
        Self {
            about: about.into(),
            verdict: verdict.into(),
            ask: None,
        }
    }
}

impl EventKind for VerdictGiven {
    const NAME: &'static str = "verdict.given";
    const VERSION: Version = Version::new(1, 0);
}

/// `take.added` 1.0: a take added to the project, named by its name and its digest, never by
/// a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TakeAdded {
    /// The take's name in the project.
    pub take: String,
    /// The take's digest, as `capture-analysis` states one.
    pub digest: TakeDigest,
}

impl TakeAdded {
    #[must_use]
    pub fn new(take: impl Into<String>, digest: TakeDigest) -> Self {
        Self {
            take: take.into(),
            digest,
        }
    }
}

impl EventKind for TakeAdded {
    const NAME: &'static str = "take.added";
    const VERSION: Version = Version::new(1, 0);
}

/// A document's digest, as a view's `from` names one: the 64-bit FNV-1a of its body as compact
/// JSON, written as 16 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocDigest(pub u64);

impl DocDigest {
    /// The FNV-1a of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
            (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        }))
    }
}

impl fmt::Display for DocDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for DocDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for DocDigest {
    type Err = String;

    /// Reads 16 lowercase hex digits, and nothing else.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let ok = s.len() == 16 && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
        ok.then(|| u64::from_str_radix(s, 16).ok())
            .flatten()
            .map(Self)
            .ok_or_else(|| {
                format!("{s:?} is not a document digest: one is 16 lowercase hex digits")
            })
    }
}

impl Serialize for DocDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DocDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for DocDigest {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("DocDigest")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::journal::DocDigest")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9a-f]{16}$",
            "description": "The 64-bit FNV-1a of the document's body as compact JSON, as 16 \
                lowercase hex digits.",
        })
    }
}

/// A path relative to the project's root, with `/` between its parts and no `.` or `..` part.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// One part of a relative path: not empty, not `.` or `..`, with no `/` or `\`.
const PART: &str = r"([^/\\.][^/\\]*|\.[^/\\.][^/\\]*|\.\.[^/\\]+)";

impl FromStr for RelPath {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let ok = s
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.contains('\\'));
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "{s:?} is not a relative path: one names a file under the project's root, with \
                 `/` between its parts and no empty, `.` or `..` part"
            ))
        }
    }
}

impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for RelPath {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("RelPath")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::journal::RelPath")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": format!("^{PART}(/{PART})*$"),
            "description": "A path relative to the project's root, with `/` between its parts \
                and no empty, `.` or `..` part.",
        })
    }
}

fn kind_name_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": r"^[a-z][a-z0-9]*(-[a-z0-9]+)*$",
        "description": "The document's kind, as its envelope names it.",
    })
}

/// `doc.written` 1.0: a document an action wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DocWritten {
    /// Where, relative to the project's root.
    pub path: RelPath,
    /// The document's kind, as its envelope names it.
    #[schemars(schema_with = "kind_name_schema")]
    pub kind: String,
    /// The kind's version it was written at.
    pub version: Version,
    /// The digest of its body.
    pub digest: DocDigest,
    /// Whether it replaced a file that was there.
    pub replaced: bool,
}

impl DocWritten {
    #[must_use]
    pub fn new(
        path: RelPath,
        kind: impl Into<String>,
        version: Version,
        digest: DocDigest,
        replaced: bool,
    ) -> Self {
        Self {
            path,
            kind: kind.into(),
            version,
            digest,
            replaced,
        }
    }
}

impl EventKind for DocWritten {
    const NAME: &'static str = "doc.written";
    const VERSION: Version = Version::new(1, 0);
}

/// An event this build reads: one variant per kind in [`EVENTS`]. It serializes as its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Event {
    AskAsked(AskAsked),
    AskAnswered(AskAnswered),
    AskWithdrawn(AskWithdrawn),
    MomentPlayed(MomentPlayed),
    VerdictGiven(VerdictGiven),
    TakeAdded(TakeAdded),
    DocWritten(DocWritten),
}

impl Event {
    /// The event kind's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::AskAsked(_) => AskAsked::NAME,
            Self::AskAnswered(_) => AskAnswered::NAME,
            Self::AskWithdrawn(_) => AskWithdrawn::NAME,
            Self::MomentPlayed(_) => MomentPlayed::NAME,
            Self::VerdictGiven(_) => VerdictGiven::NAME,
            Self::TakeAdded(_) => TakeAdded::NAME,
            Self::DocWritten(_) => DocWritten::NAME,
        }
    }

    /// The version this build writes it at.
    #[must_use]
    pub const fn version(&self) -> Version {
        match self {
            Self::AskAsked(_) => AskAsked::VERSION,
            Self::AskAnswered(_) => AskAnswered::VERSION,
            Self::AskWithdrawn(_) => AskWithdrawn::VERSION,
            Self::MomentPlayed(_) => MomentPlayed::VERSION,
            Self::VerdictGiven(_) => VerdictGiven::VERSION,
            Self::TakeAdded(_) => TakeAdded::VERSION,
            Self::DocWritten(_) => DocWritten::VERSION,
        }
    }
}

macro_rules! into_event {
    ($($kind:ident),*) => {$(
        impl core::convert::From<$kind> for Event {
            fn from(body: $kind) -> Self {
                Self::$kind(body)
            }
        }
    )*};
}

into_event!(
    AskAsked,
    AskAnswered,
    AskWithdrawn,
    MomentPlayed,
    VerdictGiven,
    TakeAdded,
    DocWritten
);

// --- Entries, as written and as read ---

/// An entry to write: its frame and a known event. [`Entry::line`] is its line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    pub id: Id,
    pub at: At,
    pub by: By,
    pub cause: Option<Id>,
    pub event: Event,
}

impl Entry {
    #[must_use]
    pub fn new(id: Id, at: At, by: By, event: impl Into<Event>) -> Self {
        Self {
            id,
            at,
            by,
            cause: None,
            event: event.into(),
        }
    }

    /// The entry as one line of compact JSON with its newline, its fields in the frame's order.
    ///
    /// # Errors
    ///
    /// When the body does not serialize.
    pub fn line(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Out<'a> {
            id: &'a Id,
            at: &'a At,
            by: &'a By,
            #[serde(skip_serializing_if = "Option::is_none")]
            cause: Option<&'a Id>,
            event: &'static str,
            v: Version,
            body: &'a Event,
        }
        let out = Out {
            id: &self.id,
            at: &self.at,
            by: &self.by,
            cause: self.cause.as_ref(),
            event: self.event.name(),
            v: self.event.version(),
            body: &self.event,
        };
        let mut text = serde_json::to_string(&out).map_err(|source| Error::Write {
            kind: JournalHead::NAME,
            source,
        })?;
        text.push('\n');
        Ok(text)
    }
}

/// Why a reader kept an entry without reading its body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Skip {
    /// An event kind this build does not know.
    UnknownEvent,
    /// A newer minor of an event kind this build knows.
    NewerMinor { reads: Version },
    /// Another major of an event kind this build knows.
    OtherMajor { reads: Version },
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEvent => f.write_str("an event kind this duet does not know"),
            Self::NewerMinor { reads } => write!(f, "newer than the {reads} this duet reads"),
            Self::OtherMajor { reads } => {
                write!(f, "another major than the {reads} this duet reads")
            }
        }
    }
}

/// An entry's body: read as its event kind, or kept unread and stated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Body {
    Read(Event),
    /// Kept verbatim in [`Line::text`], and skipped by everything that reads events.
    Skipped(Skip),
}

/// One entry as read: its exact text, its frame, and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Line {
    /// The line as it is in the file, without its newline.
    pub text: String,
    pub id: Id,
    pub at: At,
    pub by: By,
    pub cause: Option<Id>,
    /// The event kind's name, as the line gives it.
    pub event: String,
    /// The event kind's version, as the line gives it.
    pub v: Version,
    pub body: Body,
}

impl Line {
    /// The event, when this build read it.
    #[must_use]
    pub const fn read(&self) -> Option<&Event> {
        match &self.body {
            Body::Read(event) => Some(event),
            Body::Skipped(_) => None,
        }
    }

    /// The line as this build writes it: re-written from the event when it was read, or its
    /// text verbatim when it was skipped. With its newline.
    ///
    /// # Errors
    ///
    /// When the body does not serialize.
    pub fn write(&self) -> Result<String> {
        match &self.body {
            Body::Read(event) => Entry {
                id: self.id.clone(),
                at: self.at,
                by: self.by.clone(),
                cause: self.cause.clone(),
                event: event.clone(),
            }
            .line(),
            Body::Skipped(_) => Ok(format!("{}\n", self.text)),
        }
    }
}

/// The frame of a line, its body left as its exact text.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLine<'a> {
    id: Id,
    at: At,
    by: By,
    #[serde(default)]
    cause: Option<Id>,
    event: String,
    v: Version,
    #[serde(borrow)]
    body: &'a RawValue,
}

/// Reads one entry's line (without its newline): its frame by `journal` 1.0, and its body by
/// its event kind's own version policy.
///
/// # Errors
///
/// [`Error::Json`] when the line is not JSON or its frame does not read (naming an unknown
/// field), and [`Error::Body`] when a known event's body at a version this build reads does
/// not read (naming an unknown field). An unknown event, or one at a version this build does
/// not read, is no error: it is [`Body::Skipped`].
pub fn read_line(text: &str) -> Result<Line> {
    let raw: RawLine<'_> = serde_json::from_str(text).map_err(Error::Json)?;
    if !raw.body.get().starts_with('{') {
        return Err(Error::Json(serde::de::Error::custom(
            "an entry's body is a JSON object",
        )));
    }
    let body = match event_kind(&raw.event) {
        None => Body::Skipped(Skip::UnknownEvent),
        Some(info) if raw.v.major != info.version.major => Body::Skipped(Skip::OtherMajor {
            reads: info.version,
        }),
        Some(info) if raw.v.minor > info.version.minor => Body::Skipped(Skip::NewerMinor {
            reads: info.version,
        }),
        Some(info) => {
            Body::Read(
                (info.read)(raw.v.minor, raw.body.get()).map_err(|source| Error::Body {
                    kind: info.name,
                    version: raw.v,
                    part: "the body",
                    source,
                })?,
            )
        }
    };
    Ok(Line {
        text: text.to_string(),
        id: raw.id,
        at: raw.at,
        by: raw.by,
        cause: raw.cause,
        event: raw.event,
        v: raw.v,
        body,
    })
}

/// A whole journal as read: its head, and every whole entry in file order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Journal {
    /// What wrote the head.
    pub tool: Tool,
    pub from: Option<From>,
    pub statements: Vec<String>,
    pub head: JournalHead,
    /// Every whole entry, in file order: `lines[0]` is `seq` 1.
    pub lines: Vec<Line>,
    /// A last line with no newline: a write that never finished. Not an entry.
    pub torn: Option<String>,
}

impl Journal {
    /// A journal of `head`, written by `tool`, with no entries.
    #[must_use]
    pub const fn new(tool: Tool, head: JournalHead) -> Self {
        Self {
            tool,
            from: None,
            statements: Vec::new(),
            head,
            lines: Vec::new(),
            torn: None,
        }
    }

    /// The first line of a journal of `head`, written by `tool`, with its newline.
    ///
    /// # Errors
    ///
    /// When the head does not serialize.
    pub fn head_line(tool: &Tool, head: &JournalHead) -> Result<String> {
        Stream::new(tool.clone(), head.clone()).head_line()
    }

    /// Reads a journal: the head by the version policy (a newer minor of `journal` is refused),
    /// and each whole line after it by [`read_line`]. A last line with no newline is kept apart
    /// in [`Journal::torn`].
    ///
    /// # Errors
    ///
    /// [`Error::EmptyStream`] when there is no whole first line, and [`Error::Line`] naming the
    /// 1-based line and why it was refused.
    pub fn read(text: &str) -> Result<Self> {
        let (whole, torn) = split_torn(text);
        let mut lines = whole.split_terminator('\n');
        let first = lines.next().ok_or(Error::EmptyStream)?;
        let head = Stream::<JournalHead>::read(first)?;
        let lines = lines
            .enumerate()
            .map(|(n, line)| {
                read_line(line).map_err(|source| Error::Line {
                    line: n + 2,
                    source: Box::new(source),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tool: head.tool,
            from: head.from,
            statements: head.statements,
            head: head.head,
            lines,
            torn,
        })
    }

    /// Writes the journal: the head line, then each entry as [`Line::write`] writes it. A torn
    /// last line is not written: it was never an entry.
    ///
    /// # Errors
    ///
    /// When the head or a body does not serialize.
    pub fn write(&self) -> Result<String> {
        let mut stream = Stream::new(self.tool.clone(), self.head.clone());
        stream.from.clone_from(&self.from);
        stream.statements.clone_from(&self.statements);
        let mut text = stream.head_line()?;
        for line in &self.lines {
            text.push_str(&line.write()?);
        }
        Ok(text)
    }
}

// --- What an event concerns ---

/// A take as the index names it: its name and its digest, never a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TakeRef {
    pub take: String,
    pub digest: TakeDigest,
}

impl TakeRef {
    #[must_use]
    pub fn new(take: impl Into<String>, digest: TakeDigest) -> Self {
        Self {
            take: take.into(),
            digest,
        }
    }
}

/// The project-level elements some events concern (bus design decision 17): songs, parts as
/// `<song>/<part>`, and takes by name and digest. Songs and parts are named by name until the
/// songs-and-composition spec gives them a stable id, when the index names them by it, the name
/// beside it; a rename until then loses per-song lookup of older segments. Never a session, which is no duet entity.
/// Comps, snapshots and renders are found through their song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Concerns {
    /// Sorted, each once.
    pub songs: Vec<String>,
    /// `<song>/<part>`, sorted, each once.
    pub parts: Vec<String>,
    /// In the order first concerned, each once.
    pub takes: Vec<TakeRef>,
}

impl Concerns {
    /// Nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether it concerns nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.parts.is_empty() && self.takes.is_empty()
    }

    fn add_song(&mut self, song: &str) {
        if let Err(at) = self.songs.binary_search_by(|s| s.as_str().cmp(song)) {
            self.songs.insert(at, song.to_string());
        }
    }

    fn add_part(&mut self, part: String) {
        if let Err(at) = self.parts.binary_search(&part) {
            self.parts.insert(at, part);
        }
    }

    /// Adds everything `other` concerns.
    pub fn extend(&mut self, other: &Self) {
        for song in &other.songs {
            self.add_song(song);
        }
        for part in &other.parts {
            self.add_part(part.clone());
        }
        for take in &other.takes {
            if !self.takes.contains(take) {
                self.takes.push(take.clone());
            }
        }
    }
}

impl Event {
    /// What this event concerns, by its kind:
    /// - `take.added`: its take;
    /// - `doc.written`: the song whose folder holds the path (`songs/<song>/…`), and the part
    ///   when the path is under the song's `parts/` (`songs/<song>/parts/<part>…`, the part
    ///   named by the file's name up to its first `.`);
    /// - every other event: nothing yet. The asks, `moment.played` and `verdict.given` will
    ///   concern what their reference names, once the reference grammar is ruled.
    #[must_use]
    pub fn concerns(&self) -> Concerns {
        let mut concerns = Concerns::new();
        match self {
            Self::TakeAdded(added) => concerns
                .takes
                .push(TakeRef::new(added.take.clone(), added.digest)),
            Self::DocWritten(written) => {
                let parts: Vec<&str> = written.path.as_str().split('/').collect();
                if let ["songs", song, rest @ ..] = parts.as_slice()
                    && !rest.is_empty()
                {
                    concerns.add_song(song);
                    if let ["parts", file, ..] = rest {
                        let part = file.split('.').next().unwrap_or(file);
                        concerns.add_part(format!("{song}/{part}"));
                    }
                }
            }
            _ => {}
        }
        concerns
    }
}

// --- The index ---

/// `journal-index` 1.0's first line holds the head: the project's name, and the journal's own
/// id, made when it was started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IndexHead {
    /// The name of the project whose journal this is.
    pub project: String,
    /// The journal's id.
    pub id: Id,
}

impl IndexHead {
    #[must_use]
    pub fn new(project: impl Into<String>, id: Id) -> Self {
        Self {
            project: project.into(),
            id,
        }
    }
}

/// The segment an index line opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Opens {
    /// Its stamp, which names its file.
    pub segment: SegmentStamp,
    /// Its id, as its head names it.
    pub id: Id,
    /// The moment it was opened.
    pub from: At,
}

impl Opens {
    #[must_use]
    pub const fn new(segment: SegmentStamp, id: Id, from: At) -> Self {
        Self { segment, id, from }
    }
}

/// The summary of the segment an index line closes. The segment's file stays the truth: `journal
/// check` compares the two and states any difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Closes {
    /// Its stamp.
    pub segment: SegmentStamp,
    /// When its last entry was written.
    pub to: At,
    /// How many entries it holds.
    pub entries: u64,
    /// Its last entry's id.
    pub last: Id,
    /// The songs its events concern, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub songs: Vec<String>,
    /// The parts its events concern, as `<song>/<part>`, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// The takes its events concern, by name and digest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<TakeRef>,
}

impl Closes {
    /// The summary of `segment`, whose last entry was `last`, written at `to`.
    #[must_use]
    pub fn new(segment: SegmentStamp, to: At, entries: u64, last: Id, concerns: Concerns) -> Self {
        Self {
            segment,
            to,
            entries,
            last,
            songs: concerns.songs,
            parts: concerns.parts,
            takes: concerns.takes,
        }
    }
}

/// One index line: it opens a segment, and, after the first, closes the one before it, in one
/// write (bus design decision 21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IndexLine {
    pub opens: Opens,
    /// Absent on the first line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closes: Option<Closes>,
}

impl IndexLine {
    #[must_use]
    pub const fn new(opens: Opens, closes: Option<Closes>) -> Self {
        Self { opens, closes }
    }

    /// The line with its newline.
    ///
    /// # Errors
    ///
    /// When it does not serialize.
    pub fn line(&self) -> Result<String> {
        Stream::<IndexHead>::record_line(self)
    }
}

/// `journal-index` 1.0: the index head on the first line, then one [`IndexLine`] per line.
impl Kind for IndexHead {
    const NAME: &'static str = "journal-index";
    const VERSION: Version = Version::new(1, 0);
}

impl StreamKind for IndexHead {
    type Record = IndexLine;
}

/// One index line as read: its exact text and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexEntry {
    /// The line as it is in the file, without its newline.
    pub text: String,
    pub line: IndexLine,
}

/// A whole index as read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JournalIndex {
    pub tool: Tool,
    pub from: Option<From>,
    pub statements: Vec<String>,
    pub head: IndexHead,
    /// Every whole line after the head, in file order. The last opens the current segment.
    pub lines: Vec<IndexEntry>,
    /// A last line with no newline: a write that never finished.
    pub torn: Option<String>,
}

impl JournalIndex {
    /// The first line of an index of `head`, written by `tool`, with its newline.
    ///
    /// # Errors
    ///
    /// When the head does not serialize.
    pub fn head_line(tool: &Tool, head: &IndexHead) -> Result<String> {
        Stream::new(tool.clone(), head.clone()).head_line()
    }

    /// Reads an index: the head by the version policy, and each whole line strictly. A last line
    /// with no newline is kept apart in [`JournalIndex::torn`].
    ///
    /// # Errors
    ///
    /// [`Error::EmptyStream`] when there is no whole first line, and [`Error::Line`] naming the
    /// 1-based line and why it was refused.
    pub fn read(text: &str) -> Result<Self> {
        let (whole, torn) = split_torn(text);
        let mut lines = whole.split_terminator('\n');
        let first = lines.next().ok_or(Error::EmptyStream)?;
        let head = Stream::<IndexHead>::read(first)?;
        let lines = lines
            .enumerate()
            .map(|(n, text)| {
                serde_json::from_str::<IndexLine>(text)
                    .map(|line| IndexEntry {
                        text: text.to_string(),
                        line,
                    })
                    .map_err(|source| Error::Line {
                        line: n + 2,
                        source: Box::new(Error::Body {
                            kind: IndexHead::NAME,
                            version: IndexHead::VERSION,
                            part: "an index line",
                            source,
                        }),
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tool: head.tool,
            from: head.from,
            statements: head.statements,
            head: head.head,
            lines,
            torn,
        })
    }

    /// Writes the index: the head line, then each line as this build writes it.
    ///
    /// # Errors
    ///
    /// When a line does not serialize.
    pub fn write(&self) -> Result<String> {
        let mut stream = Stream::new(self.tool.clone(), self.head.clone());
        stream.from.clone_from(&self.from);
        stream.statements.clone_from(&self.statements);
        let mut text = stream.head_line()?;
        for entry in &self.lines {
            text.push_str(&entry.line.line()?);
        }
        Ok(text)
    }

    /// The current segment: what the last line opens (bus design decision 21).
    #[must_use]
    pub fn current(&self) -> Option<&Opens> {
        self.lines.last().map(|entry| &entry.line.opens)
    }
}

/// `text` split at its last newline: the whole lines, and a torn last line if there is one.
fn split_torn(text: &str) -> (&str, Option<String>) {
    let (whole, torn) = match text.rfind('\n') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text),
    };
    (whole, (!torn.is_empty()).then(|| torn.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_has_one_spelling() {
        let id = Id::from_parts(0x0196_f1a2_7b3c, [0xff; 10]);
        assert_eq!(id.as_str(), "0196f1a2-7b3c-7fff-bfff-ffffffffffff");
        assert_eq!(id.as_str().parse::<Id>().unwrap(), id);
        let zero = Id::from_parts(0, [0; 10]);
        assert_eq!(zero.as_str(), "00000000-0000-7000-8000-000000000000");
        for bad in [
            "0196F1A2-7B3C-7FFF-BFFF-FFFFFFFFFFFF",
            "0196f1a2-7b3c-4fff-bfff-ffffffffffff",
            "0196f1a2-7b3c-7fff-7fff-ffffffffffff",
            "0196f1a27b3c7fffbfffffffffffffff",
            "",
        ] {
            assert!(bad.parse::<Id>().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_time_has_one_spelling_and_is_real() {
        for (ms, text) in [
            (0, "1970-01-01T00:00:00.000Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (1_795_446_245_123, "2026-11-23T15:04:05.123Z"),
        ] {
            assert_eq!(At::from_unix_ms(ms).to_string(), text);
            assert_eq!(text.parse::<At>().unwrap().unix_ms(), ms);
        }
        for bad in [
            "2026-02-30T00:00:00.000Z",
            "2026-13-01T00:00:00.000Z",
            "2026-11-17T24:00:00.000Z",
            "2026-11-17T15:04:05Z",
            "2026-11-17T15:04:05.123+00:00",
            "1969-12-31T23:59:59.999Z",
        ] {
            assert!(bad.parse::<At>().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_segment_stamp_has_one_spelling_and_names_its_file() {
        let stamp = SegmentStamp::new(At::from_unix_ms(1_795_446_245_123));
        assert_eq!(stamp.to_string(), "20261123T150405123Z");
        assert_eq!(stamp.file_name(), "journal.20261123T150405123Z.duet.jsonl");
        assert_eq!(SegmentStamp::of_file_name(&stamp.file_name()), Some(stamp));
        assert_eq!(SegmentStamp::of_file_name("journal.duet.jsonl"), None);
        assert_eq!(stamp.next().to_string(), "20261123T150405124Z");
        for bad in [
            "20261123T150405123",
            "2026-11-23T15:04:05.123Z",
            "20260230T000000000Z",
            "20261123t150405123Z",
        ] {
            assert!(bad.parse::<SegmentStamp>().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_doc_written_concerns_its_song_and_part_and_a_take_added_its_take() {
        let written = |path: &str| {
            Event::from(DocWritten::new(
                path.parse().unwrap(),
                "part",
                Version::new(1, 0),
                DocDigest(0),
                false,
            ))
            .concerns()
        };
        let lead = written("songs/wayfaring/parts/lead.duet");
        assert_eq!(lead.songs, ["wayfaring"]);
        assert_eq!(lead.parts, ["wayfaring/lead"]);
        let comp = written("songs/wayfaring/comps/a.duet");
        assert_eq!((comp.songs.len(), comp.parts.len()), (1, 0));
        assert!(written("renders/wayfaring.mid").is_empty());
        assert!(written("songs/x.duet").is_empty());
        let digest: TakeDigest =
            "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34"
                .parse()
                .unwrap();
        let take = Event::from(TakeAdded::new("wayfaring", digest)).concerns();
        assert_eq!(take.takes, [TakeRef::new("wayfaring", digest)]);
        let mut all = lead;
        all.extend(&take);
        all.extend(&take);
        assert_eq!(all.takes.len(), 1);
        assert!(Event::from(AskAsked::new("?")).concerns().is_empty());
    }

    #[test]
    fn a_relative_path_stays_under_the_root() {
        for good in ["comps/a.duet", ".hidden", "..x", "a/...", "takes/w.flac"] {
            assert!(good.parse::<RelPath>().is_ok(), "{good}");
        }
        for bad in ["/abs", "a/../b", "./a", "a//b", "a/", "", "..", "a\\b"] {
            assert!(bad.parse::<RelPath>().is_err(), "{bad}");
        }
    }

    #[test]
    fn every_event_kind_is_registered_once_and_writes_its_name() {
        for (i, info) in EVENTS.iter().enumerate() {
            assert!(
                EVENTS[i + 1..].iter().all(|other| other.name != info.name),
                "{} twice",
                info.name
            );
            assert!(
                info.name.contains('.') && event_kind(info.name).is_some(),
                "{}",
                info.name
            );
        }
    }
}
