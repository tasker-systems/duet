//! A kind's schema version: `<major>.<minor>`, with no patch level.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

/// The version of a kind's schema, written `"<major>.<minor>"`.
///
/// A minor is an additive change: everything an older minor holds reads at the newer one. A
/// major is any other change, and comes with a converter. There is no patch level: a change that
/// moves neither number is a change to a description only, which no reader can see.
///
/// The text form is strict, so that it round-trips: two decimal numbers, each `0` or without a
/// leading zero, joined by one `.`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    /// `major.minor`.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// One of the two numbers: ASCII digits, `0` or without a leading zero, that fit a `u16`.
fn number(s: &str) -> Option<u16> {
    let digits = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let canonical = s == "0" || !s.starts_with('0');
    if digits && canonical {
        s.parse().ok()
    } else {
        None
    }
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = || Error::BadVersion {
            found: s.to_string(),
        };
        let (major, minor) = s.split_once('.').ok_or_else(bad)?;
        Ok(Self::new(
            number(major).ok_or_else(bad)?,
            number(minor).ok_or_else(bad)?,
        ))
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The pattern of a version's text form.
pub(crate) const PATTERN: &str = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$";

impl JsonSchema for Version {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Version")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_schemas::Version")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": PATTERN,
            "description": "A kind's schema version, <major>.<minor>, such as 1.0.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_form_round_trips() {
        for v in [
            Version::new(1, 0),
            Version::new(0, 7),
            Version::new(12, 345),
        ] {
            assert_eq!(v.to_string().parse::<Version>().unwrap(), v);
        }
        assert_eq!(Version::new(2, 10).to_string(), "2.10");
    }

    #[test]
    fn only_the_canonical_form_reads() {
        for bad in [
            "", "1", "1.", ".1", "1.0.0", "01.0", "1.00", "+1.0", "1.-0", " 1.0", "1.0 ", "a.b",
            "65536.0",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?} read");
        }
    }

    #[test]
    fn versions_order_by_major_then_minor() {
        assert!(Version::new(1, 9) < Version::new(1, 10));
        assert!(Version::new(1, 10) < Version::new(2, 0));
    }
}
