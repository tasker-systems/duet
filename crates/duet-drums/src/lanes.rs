//! Lane maps: which MIDI key each named lane of a drum grid plays, and the order its rows
//! are written in.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::grid::Lane;

/// The group a grid's own lanes go in when the map has no lane of their name.
const OTHER_GROUP: &str = "other";

/// The built-in `gm` map, every General MIDI Level 1 percussion key on channel 10.
const GM_TOML: &str = include_str!("../lanes.toml");

/// One lane: a row of a drum grid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LaneDef {
    /// The lane's name, such as `kick` or `hh-open`.
    pub name: String,
    /// The MIDI key the lane plays, and the key read back as this lane.
    pub key: u8,
    /// The row group, such as `cymbals`; rows are written in the map's group order.
    pub group: String,
    /// Other names a hand-written grid may use for this lane, matched ignoring case.
    pub aliases: Vec<String>,
}

/// A named table of lanes on one MIDI channel.
///
/// Every name and alias is unique, ignoring case, and no two lanes play the same key, so
/// both directions are unambiguous: a name finds one key, and a key finds one lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneMap {
    name: String,
    channel: u8,
    order: Vec<String>,
    lanes: Vec<LaneDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMap {
    channel: u8,
    order: Vec<String>,
    lanes: toml::Table,
}

/// A change to one lane of a map, as a config file's `[lanes.<name>]` table gives it:
/// each field it sets replaces the lane's, and the rest are kept. A patch for a name the
/// map does not have adds a lane, and then needs a key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LanePatch {
    /// The MIDI key the lane plays.
    pub key: Option<u8>,
    /// The row group, which must be one the map orders, or `other`. A new lane with no
    /// group goes in `other`, after every group the map orders.
    pub group: Option<String>,
    /// The lane's aliases, replacing any it had.
    pub aliases: Option<Vec<String>>,
}

impl LanePatch {
    /// A patch that sets only the key.
    pub fn key(key: u8) -> LanePatch {
        LanePatch {
            key: Some(key),
            ..LanePatch::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLane {
    key: u8,
    group: String,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Whether `name` can be a lane name: lowercase letters, digits and hyphens, starting
/// with a letter, so it cannot be mistaken for a cell or a bar line.
pub fn valid_lane_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl LaneMap {
    /// The built-in `gm` map: General MIDI Level 1 percussion, keys 35 to 81, on
    /// channel 10.
    pub fn gm() -> LaneMap {
        LaneMap::from_toml("gm", GM_TOML)
            .expect("the built-in gm lane map is valid; a test checks it")
    }

    /// The built-in map with this name. Only `gm` exists so far.
    pub fn builtin(name: &str) -> Result<LaneMap> {
        match name {
            "gm" => Ok(LaneMap::gm()),
            other => Err(Error::invalid(format!(
                "'{other}' is not a lane map; the built-in map is gm"
            ))),
        }
    }

    /// Reads a lane map from TOML, checking every rule a map must keep. Lanes keep the
    /// order they are written in.
    pub fn from_toml(name: &str, text: &str) -> Result<LaneMap> {
        let bad = |msg: String| Error::invalid(format!("lane map {name}: {msg}"));
        let raw: RawMap = toml::from_str(text).map_err(|e| bad(e.to_string()))?;
        if !(1..=16).contains(&raw.channel) {
            return Err(bad(format!("channel {} is not 1 to 16", raw.channel)));
        }
        let mut map = LaneMap {
            name: name.to_owned(),
            channel: raw.channel,
            order: raw.order,
            lanes: Vec::new(),
        };
        for (lane, value) in raw.lanes {
            let raw: RawLane = value
                .try_into()
                .map_err(|e: toml::de::Error| bad(format!("lane {lane}: {e}")))?;
            map.lanes.push(LaneDef {
                name: lane,
                key: raw.key,
                group: raw.group,
                aliases: raw.aliases,
            });
        }
        map.check().map_err(|e| bad(e.to_string()))?;
        Ok(map)
    }

    fn check(&self) -> Result<()> {
        let mut names: Vec<String> = Vec::new();
        for lane in &self.lanes {
            if !valid_lane_name(&lane.name) {
                return Err(Error::invalid(format!(
                    "'{}' is not a lane name: use lowercase letters, digits and hyphens",
                    lane.name
                )));
            }
            if lane.key > 127 {
                return Err(Error::invalid(format!(
                    "lane {}: key {} is not 0 to 127",
                    lane.name, lane.key
                )));
            }
            if let Some(other) = self
                .lanes
                .iter()
                .find(|o| o.key == lane.key && o.name != lane.name)
            {
                return Err(Error::invalid(format!(
                    "lanes {} and {} both play key {}",
                    other.name, lane.name, lane.key
                )));
            }
            if !self.order.contains(&lane.group) {
                return Err(Error::invalid(format!(
                    "lane {}: group '{}' is not in order",
                    lane.name, lane.group
                )));
            }
            for name in std::iter::once(&lane.name).chain(&lane.aliases) {
                let folded = name.to_ascii_lowercase();
                if names.contains(&folded) {
                    return Err(Error::invalid(format!("'{name}' names more than one lane")));
                }
                names.push(folded);
            }
        }
        for (i, group) in self.order.iter().enumerate() {
            if self.order[..i].contains(group) {
                return Err(Error::invalid(format!("group '{group}' is in order twice")));
            }
            if !self.lanes.iter().any(|l| &l.group == group) {
                return Err(Error::invalid(format!("group '{group}' has no lanes")));
            }
        }
        Ok(())
    }

    /// This map with a grid's own lanes laid over it, as a file's `lane` lines do.
    ///
    /// A lane with a map lane's name takes the new key and keeps its row; a new name goes
    /// in an `other` group after every map lane. A map lane whose key a laid-over lane
    /// takes is left out, and so is a map alias that a laid-over name takes, so each name
    /// and each key still finds one lane.
    pub fn overlaid(&self, extra: &[Lane]) -> Result<LaneMap> {
        let mut out = self.clone();
        for lane in extra {
            out.lanes
                .retain(|l| l.name == lane.name || l.key != lane.key);
            for l in &mut out.lanes {
                l.aliases.retain(|a| !a.eq_ignore_ascii_case(&lane.name));
            }
            match out.lanes.iter_mut().find(|l| l.name == lane.name) {
                Some(l) => l.key = lane.key,
                None => {
                    if !out.order.iter().any(|g| g == OTHER_GROUP) {
                        out.order.push(OTHER_GROUP.to_owned());
                    }
                    out.lanes.push(LaneDef {
                        name: lane.name.clone(),
                        key: lane.key,
                        group: OTHER_GROUP.to_owned(),
                        aliases: Vec::new(),
                    });
                }
            }
        }
        out.order
            .retain(|g| out.lanes.iter().any(|l| &l.group == g));
        out.check()?;
        Ok(out)
    }

    /// This map with `patches` applied, in order, lane by lane: a patch for a lane the
    /// map has changes the fields it sets, and one for a new name adds a lane. No lane is
    /// ever removed, so a patch that would leave two lanes on one key, or one name or
    /// alias on two lanes, is refused by name; move the other lane in the same patches.
    pub fn patched(&self, patches: &[(String, LanePatch)]) -> Result<LaneMap> {
        let mut out = self.clone();
        for (name, patch) in patches {
            if !valid_lane_name(name) {
                return Err(Error::invalid(format!(
                    "'{name}' is not a lane name: use lowercase letters, digits and hyphens"
                )));
            }
            if let Some(group) = &patch.group
                && group != OTHER_GROUP
                && !out.order.contains(group)
            {
                return Err(Error::invalid(format!(
                    "lane {name}: group '{group}' is not one of {}, or {OTHER_GROUP}",
                    out.order.join(", ")
                )));
            }
            match out.lanes.iter_mut().find(|l| &l.name == name) {
                Some(lane) => {
                    if let Some(key) = patch.key {
                        lane.key = key;
                    }
                    if let Some(group) = &patch.group {
                        lane.group = group.clone();
                    }
                    if let Some(aliases) = &patch.aliases {
                        lane.aliases = aliases.clone();
                    }
                }
                None => {
                    let key = patch.key.ok_or_else(|| {
                        Error::invalid(format!(
                            "lane {name} is not in the {} map, so it needs a key",
                            self.name
                        ))
                    })?;
                    out.lanes.push(LaneDef {
                        name: name.clone(),
                        key,
                        group: patch
                            .group
                            .clone()
                            .unwrap_or_else(|| OTHER_GROUP.to_owned()),
                        aliases: patch.aliases.clone().unwrap_or_default(),
                    });
                }
            }
        }
        if out.lanes.iter().any(|l| l.group == OTHER_GROUP)
            && !out.order.iter().any(|g| g == OTHER_GROUP)
        {
            out.order.push(OTHER_GROUP.to_owned());
        }
        out.order
            .retain(|g| out.lanes.iter().any(|l| &l.group == g));
        out.check()?;
        Ok(out)
    }

    /// What `map gm` stands for when reading .drums text under a config's `patches`:
    /// General MIDI, with only the config's additions. A shared file must mean the same
    /// on every machine, so a config may add lanes, and aliases to a gm lane, but a gm
    /// lane keeps its key, group and aliases here. Reading a MIDI drum track is where a
    /// moved key applies; use [`LaneMap::patched`] for that.
    ///
    /// Returns the map and a notice for every part of a patch that does not apply to
    /// reading text, so nothing is dropped silently: a gm lane's key moved off its gm
    /// key, or an added lane that would collide with gm (as one on a key only a move
    /// frees does).
    pub fn gm_for_text(patches: &[(String, LanePatch)]) -> (LaneMap, Vec<String>) {
        let gm = LaneMap::gm();
        let mut out = gm.clone();
        let mut notices = Vec::new();
        for (name, patch) in patches {
            let patch = match gm.lanes.iter().find(|l| &l.name == name) {
                Some(lane) => {
                    if let Some(key) = patch.key
                        && key != lane.key
                    {
                        notices.push(format!(
                            "lane {name}: key {key} does not apply to reading .drums text, \
                             where map gm keeps {name} on {}; a file's own lane line moves it",
                            lane.key
                        ));
                    }
                    let Some(added) = &patch.aliases else {
                        continue;
                    };
                    let mut aliases = lane.aliases.clone();
                    aliases.extend(added.iter().filter(|a| !lane.aliases.contains(a)).cloned());
                    LanePatch {
                        aliases: Some(aliases),
                        ..LanePatch::default()
                    }
                }
                None => patch.clone(),
            };
            match out.patched(&[(name.clone(), patch)]) {
                Ok(map) => out = map,
                Err(e) => notices.push(format!(
                    "lane {name} does not apply to reading .drums text, where map gm keeps \
                     every gm lane: {e}"
                )),
            }
        }
        (out, notices)
    }

    /// The map's name, such as `gm`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The MIDI channel its lanes play on, 1 to 16.
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Every lane, in the order rows are written: by group, then as the map lists them.
    pub fn lanes(&self) -> impl Iterator<Item = &LaneDef> {
        self.order
            .iter()
            .flat_map(|g| self.lanes.iter().filter(move |l| &l.group == g))
    }

    /// The lane a name or alias refers to, ignoring case.
    pub fn lookup(&self, name: &str) -> Option<&LaneDef> {
        self.lanes.iter().find(|l| {
            l.name.eq_ignore_ascii_case(name)
                || l.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
        })
    }

    /// The lane that plays a key.
    pub fn by_key(&self, key: u8) -> Option<&LaneDef> {
        self.lanes.iter().find(|l| l.key == key)
    }

    /// Where a lane's row goes: its place in [`LaneMap::lanes`]. A lane not in the map
    /// goes after every lane that is.
    pub fn rank(&self, name: &str) -> usize {
        self.lanes()
            .position(|l| l.name == name)
            .unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gm_holds_every_gm1_key_once_on_channel_10() {
        let gm = LaneMap::gm();
        assert_eq!(gm.channel(), 10);
        let mut keys: Vec<u8> = gm.lanes().map(|l| l.key).collect();
        keys.sort_unstable();
        assert_eq!(keys, (35..=81).collect::<Vec<u8>>());
        assert_eq!(gm.lookup("kick").map(|l| l.key), Some(36));
        assert_eq!(gm.lookup("BD").map(|l| l.name.as_str()), Some("kick"));
        assert_eq!(gm.lookup("sidestick").map(|l| l.key), Some(37));
        assert_eq!(gm.lookup("rimshot").map(|l| l.key), Some(40));
        assert_eq!(gm.by_key(46).map(|l| l.name.as_str()), Some("hh-open"));
        assert!(
            gm.lookup("rim").is_none(),
            "rim is ambiguous, so it names no lane"
        );
        let first = gm.lanes().next().map(|l| l.name.clone());
        assert_eq!(first.as_deref(), Some("crash"), "cymbals are the top row");
    }

    #[test]
    fn a_map_that_breaks_a_rule_is_refused_by_name() {
        let map = |lanes: &str| {
            LaneMap::from_toml("t", &format!("channel = 10\norder = [\"a\"]\n{lanes}"))
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        assert!(map("[lanes.k]\nkey = 36\ngroup = \"a\"\n").is_ok());
        let cases = [
            (
                "[lanes.K]\nkey = 36\ngroup = \"a\"\n",
                "'K' is not a lane name",
            ),
            (
                "[lanes.k]\nkey = 36\ngroup = \"a\"\n[lanes.j]\nkey = 36\ngroup = \"a\"\n",
                "both play key 36",
            ),
            (
                "[lanes.k]\nkey = 36\ngroup = \"b\"\n",
                "group 'b' is not in order",
            ),
            (
                "[lanes.k]\nkey = 36\ngroup = \"a\"\n[lanes.j]\nkey = 35\ngroup = \"a\"\naliases = [\"K\"]\n",
                "'K' names more than one lane",
            ),
            ("[lanes.k]\nkey = 200\ngroup = \"a\"\n", "lane map t"),
        ];
        for (lanes, want) in cases {
            let err = map(lanes).expect_err(want);
            assert!(err.contains(want), "{err} should say {want}");
        }
    }
}
