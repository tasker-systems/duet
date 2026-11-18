//! `project`: a project's document, `.duet/project.duet`. It names the project, the duet that
//! made it, the defaults a new song starts from, and the project's takes.
//!
//! A project is a workspace that holds songs (decision 1 of temper `01a1eb60`). **The document
//! lists no songs.** A song is a folder in `songs/` holding a `song.duet`, and listing them here
//! too would put one fact in two places. **There is no sharing status:** no `kept`,
//! `restriction` or `licence` field (decision 3, dropped). Whether a take was copied in or named
//! where it is is found on disk, not stored.
//!
//! The design is `docs/superpowers/specs/2026-11-17-project-design.md`, §2.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use duet_core::{Grid, Key, TimeSig};
use duet_dsp::Take;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Kind, Version};

/// A project's document: the body of `project` 1.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Project {
    /// The project's name for people. `duet init` sets it from the directory's name.
    pub title: String,
    /// The date the project was made.
    pub created: Date,
    /// The `<major>.<minor>` of the duet that made the project. Information only: it is never
    /// checked. The envelope's `tool` says who last wrote the file; this says who made the
    /// project.
    pub duet: Version,
    /// What a new song's composition, a render and a new grid start from. Capture never takes
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Defaults>,
    /// The project's takes, each named once.
    pub takes: Vec<ProjectTake>,
}

impl Project {
    /// A project with no defaults and no takes.
    #[must_use]
    pub fn new(title: impl Into<String>, created: Date, duet: Version) -> Self {
        Self {
            title: title.into(),
            created,
            duet,
            defaults: None,
            takes: Vec::new(),
        }
    }

    /// The take named `name`.
    #[must_use]
    pub fn take(&self, name: &Name) -> Option<&ProjectTake> {
        self.takes.iter().find(|take| take.name == *name)
    }

    /// Checks what a schema cannot: every take's name is unique, and every tempo is a positive,
    /// finite number.
    ///
    /// # Errors
    ///
    /// A message naming the duplicate take, or the field whose tempo is not a tempo.
    pub fn check(&self) -> Result<(), String> {
        if let Some(defaults) = &self.defaults {
            check_tempo("defaults.tempo_bpm", defaults.tempo_bpm)?;
            if let Some(tuning) = &defaults.tuning {
                tuning.cents()?;
            }
        }
        let mut names = HashSet::new();
        for take in &self.takes {
            if !names.insert(&take.name) {
                return Err(format!(
                    "two takes are named {:?}: a take's name is unique in the project",
                    take.name.as_str()
                ));
            }
            if let Some(given) = &take.given {
                check_tempo(
                    &format!("takes[{}].given.tempo_bpm", take.name),
                    given.tempo_bpm,
                )?;
            }
        }
        Ok(())
    }
}

fn check_tempo(field: &str, tempo: Option<f64>) -> Result<(), String> {
    match tempo {
        Some(bpm) if !(bpm.is_finite() && bpm > 0.0) => Err(format!(
            "{field} is {bpm}: a tempo is a positive number of quarter notes per minute"
        )),
        _ => Ok(()),
    }
}

/// `project` 1.0's body is a [`Project`]. A body is checked as it is read ([`Project::check`]):
/// two takes with one name, or a tempo that is not one, are refused by name.
impl Kind for Project {
    const NAME: &'static str = "project";
    const VERSION: Version = Version::new(1, 0);

    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        let project: Project = serde_json::from_value(body)?;
        project.check().map_err(serde::de::Error::custom)?;
        Ok(project)
    }
}

/// What a new song's composition, a render and a new grid start from, each spelled as its flag
/// spells it. Every field is optional. A closed object: an unknown field is refused, so a typo
/// is caught rather than ignored. The lane map is not here: lanes are config, in
/// `.duet/duet.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Defaults {
    /// The tempo, in quarter notes per minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub tempo_bpm: Option<f64>,
    /// The meter, such as 3/4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<TimeSig>,
    /// The key, such as Ab major.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    /// The tuning renders take: A4 in Hz (440, 442) or in cents from 440 Hz (+0c, -12c).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tuning: Option<Tuning>,
    /// The step a new grid takes, such as 1/16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
}

/// One of the project's takes: its name, its identity, and what the owner gave it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProjectTake {
    /// The take's name in the project, unique in it. Its directory under `takes/` and its file
    /// stems use it.
    pub name: Name,
    /// The take: the digest of its decoded samples, which is its identity, and their rate,
    /// channels and frames. The type `capture-analysis` and `performance` state a take with.
    pub take: Take,
    /// The source's own file name when added, without its directory. Information, for people
    /// and for finding it again; never a path.
    pub file: FileName,
    /// The date the take was added.
    pub added: Date,
    /// Values the owner gave this take, used by every later capture of it and stated as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given: Option<Given>,
}

impl ProjectTake {
    /// A take with nothing given.
    #[must_use]
    pub fn new(name: Name, take: Take, file: FileName, added: Date) -> Self {
        Self {
            name,
            take,
            file,
            added,
            given: None,
        }
    }
}

/// Values the owner gave a take, remembered for it (decision 6). A closed object: an unknown
/// field is refused, so a typo is caught rather than ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Given {
    /// The tempo, in quarter notes per minute, as `--tempo` gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub tempo_bpm: Option<f64>,
    /// The meter, as `--meter` gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<TimeSig>,
    /// The key, as `--key` gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
}

/// Implements `Serialize`, `Deserialize` (through `FromStr`) and a pattern `JsonSchema` for a
/// type written as a string.
macro_rules! text_type {
    ($ty:ident, $name:literal, $pattern:expr, $description:literal) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let text = Cow::<str>::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $ty {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed($name)
            }

            fn schema_id() -> Cow<'static, str> {
                Cow::Borrowed(concat!("duet_schemas::project::", $name))
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": $pattern,
                    "description": $description,
                })
            }
        }
    };
}

/// A take's (or, later, a song's) name: lowercase letters and digits in words joined by single
/// hyphens, such as `wayfaring` or `blood-sport-v2`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

const NAME_PATTERN: &str = "^[a-z0-9]+(-[a-z0-9]+)*$";

impl Name {
    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Name {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let words = !s.is_empty()
            && s.split('-').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if words {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "{s:?} is not a name: one is lowercase letters and digits, words joined by \
                 single hyphens, such as wayfaring or blood-sport-v2"
            ))
        }
    }
}

text_type!(
    Name,
    "Name",
    NAME_PATTERN,
    "A name in the project: lowercase letters and digits, words joined by single hyphens, such as wayfaring or blood-sport-v2."
);

/// A file's own name, without its directory: never a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileName(String);

const FILE_NAME_PATTERN: &str = r"^[^/\\]+$";

impl FileName {
    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for FileName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        if s.is_empty() || s == "." || s == ".." || s.contains(['/', '\\']) {
            Err(format!(
                "{s:?} is not a file's name: it is the name alone, without its directory"
            ))
        } else {
            Ok(Self(s.to_string()))
        }
    }
}

text_type!(
    FileName,
    "FileName",
    FILE_NAME_PATTERN,
    "A file's own name, without its directory, such as Blood Sport 3.m4a."
);

/// A calendar date, written `2026-11-17`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

const DATE_PATTERN: &str = "^[0-9]{4}-[0-9]{2}-[0-9]{2}$";

impl Date {
    /// The date, if it is one: `Date::new(2026, 2, 30)` is `None`.
    #[must_use]
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (year <= 9999 && (1..=days).contains(&day)).then_some(Self { year, month, day })
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl FromStr for Date {
    type Err = String;

    /// Reads exactly `YYYY-MM-DD`, of a real date: `2026-02-30` is refused.
    fn from_str(s: &str) -> Result<Self, String> {
        let bad = || format!("{s:?} is not a date: one is YYYY-MM-DD, such as 2026-11-17");
        let b = s.as_bytes();
        let shape = b.len() == 10
            && b.iter().enumerate().all(|(i, c)| match i {
                4 | 7 => *c == b'-',
                _ => c.is_ascii_digit(),
            });
        if !shape {
            return Err(bad());
        }
        let (year, month, day) = (
            s[..4].parse().map_err(|_| bad())?,
            s[5..7].parse().map_err(|_| bad())?,
            s[8..].parse().map_err(|_| bad())?,
        );
        Self::new(year, month, day).ok_or_else(bad)
    }
}

text_type!(
    Date,
    "Date",
    DATE_PATTERN,
    "A calendar date, YYYY-MM-DD, such as 2026-11-17."
);

/// A tuning, as `--tuning` spells it: A4 in Hz (`440`, `442`) or in cents from 440 Hz (`+0c`,
/// `-12c`), at most 100 cents (a key) either way of 440 Hz. It is kept as it was written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tuning(String);

/// The furthest, in cents, a tuning may sit from 440 Hz: further is another key.
const MAX_TUNING_CENTS: f64 = 100.0;

const TUNING_PATTERN: &str = r"^([+-]?[0-9]+(\.[0-9]+)?c|[0-9]+(\.[0-9]+)?)$";

impl Tuning {
    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The tuning in cents from 440 Hz.
    ///
    /// # Errors
    ///
    /// A message when the text is not a tuning, or sits more than 100 cents from 440 Hz.
    pub fn cents(&self) -> Result<f64, String> {
        let s = self.0.as_str();
        let shape = |text: &str, signed: bool| {
            let text = if signed {
                text.strip_prefix(['+', '-']).unwrap_or(text)
            } else {
                text
            };
            let (whole, frac) = text.split_once('.').unwrap_or((text, "0"));
            !whole.is_empty()
                && !frac.is_empty()
                && whole
                    .bytes()
                    .chain(frac.bytes())
                    .all(|b| b.is_ascii_digit())
        };
        let cents = if let Some(cents) = s.strip_suffix('c') {
            shape(cents, true)
                .then(|| cents.parse::<f64>().ok())
                .flatten()
        } else {
            shape(s, false)
                .then(|| s.parse::<f64>().ok())
                .flatten()
                .filter(|hz| *hz > 0.0)
                .map(|hz| 1200.0 * (hz / 440.0).log2())
        };
        match cents {
            Some(c) if c.is_finite() && c.abs() <= MAX_TUNING_CENTS => Ok(c),
            Some(c) if c.is_finite() => Err(format!(
                "tuning {s:?} is {c:+.0} cents from 440 Hz; a tuning is at most \
                 {MAX_TUNING_CENTS} cents (a key) either way of it"
            )),
            _ => Err(format!(
                "{s:?} is not a tuning: one is A4 in Hz (440, 442) or cents from 440 Hz (+0c, \
                 -12c)"
            )),
        }
    }
}

impl fmt::Display for Tuning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Tuning {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let tuning = Self(s.to_string());
        tuning.cents()?;
        Ok(tuning)
    }
}

text_type!(
    Tuning,
    "Tuning",
    TUNING_PATTERN,
    "A tuning as --tuning spells it: A4 in Hz (440, 442) or cents from 440 Hz (+0c, -12c), at most 100 cents from 440 Hz."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_reads_only_as_written_and_only_when_real() {
        let date: Date = "2026-11-17".parse().unwrap();
        assert_eq!(date.to_string(), "2026-11-17");
        assert!("2024-02-29".parse::<Date>().is_ok());
        for bad in [
            "2026-02-29",
            "2100-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-1-17",
            "26-11-17",
            "2026-11-17T00",
        ] {
            assert!(bad.parse::<Date>().is_err(), "{bad} reads");
        }
    }

    #[test]
    fn a_name_is_lowercase_words_joined_by_single_hyphens() {
        for good in ["wayfaring", "blood-sport-v2", "take2"] {
            assert_eq!(good.parse::<Name>().unwrap().as_str(), good);
        }
        for bad in ["", "Wayfaring", "-a", "a-", "a--b", "a b", "a_b", "a/b"] {
            assert!(bad.parse::<Name>().is_err(), "{bad:?} reads");
        }
    }

    #[test]
    fn a_file_name_is_never_a_path() {
        assert!("Blood Sport 3.m4a".parse::<FileName>().is_ok());
        for bad in ["", ".", "..", "takes/a.flac", r"C:\a.flac", "/a.flac"] {
            assert!(bad.parse::<FileName>().is_err(), "{bad:?} reads");
        }
    }

    #[test]
    fn a_tuning_reads_as_its_flag_does_and_is_kept_as_written() {
        let cents = |s: &str| s.parse::<Tuning>().unwrap().cents().unwrap();
        assert_eq!(cents("+0c"), 0.0);
        assert_eq!(cents("-12c"), -12.0);
        assert_eq!(cents("440"), 0.0);
        assert!((cents("442") - 7.85).abs() < 0.01);
        assert_eq!("+0c".parse::<Tuning>().unwrap().to_string(), "+0c");
        for bad in [
            "", "c", "+c", "0", "-440", "440c5", "+101c", "500", "inf", "1e2c", "+.5c",
        ] {
            assert!(bad.parse::<Tuning>().is_err(), "{bad:?} reads");
        }
    }
}
