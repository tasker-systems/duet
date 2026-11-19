//! `snapshot`: what a song's snapshot is, `songs/<song>/snapshots/<date>-<name>/snapshot.duet`.
//!
//! A snapshot is a deliberate checkpoint of a song (the domain model's ruling 5): a folder of
//! copies, made once and never written again. It holds a byte-for-byte copy of the song's
//! `song.duet`, a copy of every part the composition places (not every file in `parts/`), and
//! this document, which states:
//! - the snapshot's name, the date it was taken, and the owner's words, if any;
//! - every file copied, by its path in the snapshot's folder and the SHA-256 of its bytes
//!   ([`FileDigest`]), so a file changed after the snapshot was taken is found by its digest;
//! - the takes the song linked, each by its name and digest. Takes are named, never copied:
//!   they are the project's and never change.
//!
//! A snapshot keeps the ids it copied (ruling 13): it is the same song at a point in time.
//!
//! The design is `docs/superpowers/specs/2026-11-17-songs-and-composition-design.md`, §3.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::journal::{RelPath, TakeRef};
use crate::project::{Date, Name};
use crate::{Kind, Version};

/// The file in a snapshot's folder that says what it is.
pub const SNAPSHOT_FILE: &str = "snapshot.duet";
/// The song's document, in a song's folder and in each of its snapshots.
pub const SONG_FILE: &str = "song.duet";

/// What a snapshot is: the body of `snapshot` 1.0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Snapshot {
    /// The snapshot's name, as the owner gave it. Its folder is `<date>-<name>`.
    pub name: Name,
    /// The day it was taken, in UTC.
    pub date: Date,
    /// What the owner said of it, in their words. duet never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    /// Every file copied into the snapshot's folder, `song.duet` first, then each part the
    /// composition placed, as `parts/<file>`, each with the digest of its bytes.
    pub files: Vec<Copied>,
    /// The takes the song linked when the snapshot was taken, each by its name and digest.
    pub takes: Vec<TakeRef>,
}

impl Snapshot {
    /// A snapshot named `name`, taken on `date`, of `files`, linking no takes.
    #[must_use]
    pub const fn new(name: Name, date: Date, files: Vec<Copied>) -> Self {
        Self {
            name,
            date,
            said: None,
            files,
            takes: Vec::new(),
        }
    }

    /// The snapshot's folder under its song's `snapshots/`: `<date>-<name>`.
    #[must_use]
    pub fn folder(&self) -> String {
        folder(self.date, &self.name)
    }

    /// Checks what a schema cannot: `song.duet` comes first, every other file is a part under
    /// `parts/`, and no file is stated twice.
    ///
    /// # Errors
    ///
    /// A message naming the first file that is wrong.
    pub fn check(&self) -> Result<(), String> {
        match self.files.first() {
            Some(first) if first.path.as_str() == SONG_FILE => {}
            Some(first) => {
                return Err(format!(
                    "files[0] is {}: a snapshot's first file is its {SONG_FILE}",
                    first.path
                ));
            }
            None => return Err(format!("files is empty: a snapshot holds its {SONG_FILE}")),
        }
        let mut seen = BTreeSet::new();
        for (n, file) in self.files.iter().enumerate() {
            let path = file.path.as_str();
            if n > 0 && !is_part_path(path) {
                return Err(format!(
                    "files[{n}] is {path}: a snapshot holds its {SONG_FILE} and parts, as \
                     parts/<file>"
                ));
            }
            if !seen.insert(path.to_ascii_lowercase()) {
                return Err(format!("files[{n}] states {path} twice"));
            }
        }
        Ok(())
    }
}

/// `parts/<file>`, one level down, and nothing else.
fn is_part_path(path: &str) -> bool {
    matches!(path.split('/').collect::<Vec<_>>().as_slice(), ["parts", file] if !file.is_empty())
}

/// A snapshot's folder under its song's `snapshots/`: `<date>-<name>`.
#[must_use]
pub fn folder(date: Date, name: &Name) -> String {
    format!("{date}-{name}")
}

/// `snapshot` 1.0's body is a [`Snapshot`], checked as it is read ([`Snapshot::check`]).
impl Kind for Snapshot {
    const NAME: &'static str = "snapshot";
    const VERSION: Version = Version::new(1, 0);

    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        let snapshot: Self = serde_json::from_value(body)?;
        snapshot.check().map_err(serde::de::Error::custom)?;
        Ok(snapshot)
    }
}

/// One file a snapshot copied: its path in the snapshot's folder, and the digest of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Copied {
    /// Its path in the snapshot's folder: `song.duet`, or `parts/<file>`. The working file it
    /// was copied from is at the same path in the song's folder.
    pub path: RelPath,
    /// The SHA-256 of its bytes, as they were copied.
    pub digest: FileDigest,
}

impl Copied {
    #[must_use]
    pub const fn new(path: RelPath, digest: FileDigest) -> Self {
        Self { path, digest }
    }
}

/// The SHA-256 of a file's bytes, written `sha256:` and 64 lowercase hex digits. A file whose
/// bytes change, by one space, has another digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileDigest(pub [u8; 32]);

const PREFIX: &str = "sha256:";

impl FileDigest {
    /// The SHA-256 of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PREFIX)?;
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for FileDigest {
    type Err = String;

    /// Reads `sha256:` and 64 lowercase hex digits, and nothing else.
    fn from_str(s: &str) -> Result<Self, String> {
        let bad =
            || format!("{s:?} is not a file digest: one is sha256: and 64 lowercase hex digits");
        let hex = s.strip_prefix(PREFIX).ok_or_else(bad)?;
        if hex.len() != 64 || !hex.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(bad());
        }
        let mut bytes = [0u8; 32];
        for (n, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * n..2 * n + 2], 16).map_err(|_| bad())?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for FileDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FileDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for FileDigest {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FileDigest")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::snapshot::FileDigest")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The SHA-256 of a file's bytes: sha256: and 64 lowercase hex digits.",
            "pattern": "^sha256:[0-9a-f]{64}$",
            "type": "string"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_digest_reads_back_what_it_writes_and_nothing_else() {
        let digest = FileDigest::of(b"song");
        let text = digest.to_string();
        assert!(text.starts_with(PREFIX) && text.len() == 71, "{text}");
        assert_eq!(text.parse::<FileDigest>().unwrap(), digest);
        for bad in [
            &text[7..],
            &text.to_uppercase(),
            &text[..70],
            &format!("{text}0"),
            "sha1:00",
        ] {
            assert!(bad.parse::<FileDigest>().is_err(), "{bad}");
        }
        assert_ne!(FileDigest::of(b"song"), FileDigest::of(b"song "));
    }
}
