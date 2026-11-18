//! `song`: a song's document, `songs/<song>/song.duet`. It holds the song (its id, title,
//! status and intent) and its composition (the song's bars: the tempo, meter and key maps,
//! sections, and where each part is placed).
//!
//! The song and its composition are different things, kept as two parts of one body (the
//! owner's ruling 6 in decision 01a1eb48): the composition can be replaced wholesale while the
//! song stays the song.
//!
//! - **The song lists no parts.** A part is a file in the song's `parts/`, named by its stem,
//!   and the composition's placements name parts by that stem ([`Song::check_parts`]).
//! - **The status is the owner's record** of where the song stands, a closed state machine
//!   ([`Status`], ruling 9). It gates no command (decision 01a1ee66): only a move the machine
//!   does not draw is refused.
//! - **Takes and their placement** on the song's bars are songs task 3; they come into 1.0 in
//!   place when it is built (ruling 12).
//!
//! The design is `docs/superpowers/specs/2026-11-17-songs-and-composition-design.md`, §1 and §7.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use duet_core::{Key, Position, TimeMap, TimeSig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::journal::Id;
use crate::project::{Defaults, Name};
use crate::{Kind, Version};

/// The resolution a new song's composition counts its ticks at.
pub const TICKS_PER_QUARTER: u16 = 480;
/// A new song's tempo when neither a flag nor the project gives one.
pub const DEFAULT_BPM: f64 = 120.0;

/// A song's document: the body of `song` 1.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Song {
    /// The song's stable id: a UUIDv7, minted by `duet song new` and never changed. Renaming
    /// the song's folder or its title leaves it.
    pub id: Id,
    /// The song's name for people. Its folder's name is its name for commands.
    pub title: String,
    /// Where the song stands: `idea`, `sketch`, `developing` or `arranged`.
    pub status: Status,
    /// What the song is meant to be, in the owner's words. duet never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// How the song is written down: its bars, and where each part sits on them.
    pub composition: Composition,
}

impl Song {
    /// A new song, at `idea`.
    #[must_use]
    pub fn new(id: Id, title: impl Into<String>, composition: Composition) -> Self {
        Self {
            id,
            title: title.into(),
            status: Status::Idea,
            intent: None,
            composition,
        }
    }

    /// Checks what a schema cannot: the title says something, and the composition holds
    /// ([`Composition::check`]).
    ///
    /// # Errors
    ///
    /// A message naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("the song's title is empty: a song has a name for people".into());
        }
        self.composition.check()
    }

    /// Checks the placements against the song's parts, by name: every placement names one.
    ///
    /// # Errors
    ///
    /// A message naming the first placement whose part the song does not have.
    pub fn check_parts<'a>(&self, parts: impl IntoIterator<Item = &'a Name>) -> Result<(), String> {
        let parts: BTreeSet<&Name> = parts.into_iter().collect();
        for (n, placement) in self.composition.placements.iter().enumerate() {
            if !parts.contains(&placement.part) {
                let have = if parts.is_empty() {
                    "it has none".to_owned()
                } else {
                    format!(
                        "its parts are {}",
                        parts
                            .iter()
                            .map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return Err(format!(
                    "composition.placements[{n}] places the part {:?}, and the song has no part \
                     of that name: {have}",
                    placement.part.as_str()
                ));
            }
        }
        Ok(())
    }

    /// Moves the song to `to`, if the state machine draws that move.
    ///
    /// # Errors
    ///
    /// A message naming the current state and the moves allowed from it; the song is
    /// unchanged.
    pub fn set_status(&mut self, to: Status) -> Result<(), String> {
        self.status = self.status.to(to)?;
        Ok(())
    }
}

/// `song` 1.0's body is a [`Song`]. A body is checked as it is read ([`Song::check`]): a meter
/// change off a bar line, a map out of order, or a position the bars do not hold, are refused
/// by name.
impl Kind for Song {
    const NAME: &'static str = "song";
    const VERSION: Version = Version::new(1, 0);

    fn read_minor(minor: u16, body: serde_json::Value) -> serde_json::Result<Self> {
        let _ = minor;
        let song: Song = serde_json::from_value(body)?;
        song.check().map_err(serde::de::Error::custom)?;
        Ok(song)
    }
}

/// Where a song stands: a closed state machine, set by the owner or by an agent with the
/// owner, never inferred (ruling 9).
///
/// ```text
/// idea ──▶ sketch ──▶ developing ──▶ arranged
///                         ▲              │
///                         └── reopen ────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Where every song starts.
    Idea,
    Sketch,
    Developing,
    Arranged,
}

impl Status {
    /// Every state, in order.
    pub const ALL: [Self; 4] = [Self::Idea, Self::Sketch, Self::Developing, Self::Arranged];

    /// The states this one may move to.
    #[must_use]
    pub const fn moves(self) -> &'static [Self] {
        match self {
            Self::Idea => &[Self::Sketch],
            Self::Sketch => &[Self::Developing],
            Self::Developing => &[Self::Arranged],
            Self::Arranged => &[Self::Developing],
        }
    }

    /// The move from this state to `to`.
    ///
    /// # Errors
    ///
    /// A move the machine does not draw, a state to itself included, naming this state and the
    /// moves allowed from it.
    pub fn to(self, to: Self) -> Result<Self, String> {
        let allowed = self
            .moves()
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" or ");
        if self.moves().contains(&to) {
            Ok(to)
        } else if self == to {
            Err(format!(
                "the song is already at {self}; from {self} it moves only to {allowed}"
            ))
        } else {
            Err(format!(
                "the song is at {self}, and {self} → {to} is not a move: from {self} it moves \
                 only to {allowed}"
            ))
        }
    }

    /// The state's name, as the document writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idea => "idea",
            Self::Sketch => "sketch",
            Self::Developing => "developing",
            Self::Arranged => "arranged",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == s)
            .ok_or_else(|| {
                format!("{s:?} is not a song's status: one is idea, sketch, developing or arranged")
            })
    }
}

/// How a song is written down: the tempo, meter and key maps, the sections, and where each part
/// sits. Every position is a `bar.beat.tick` [`Position`] on the song's bars, at
/// `ticks_per_quarter`, written as it falls: `3.4.0` in 3/4 is refused, since that place is
/// `4.1.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Composition {
    /// The resolution tick offsets in positions are counted in.
    #[schemars(range(min = 1, max = 32767))]
    pub ticks_per_quarter: u16,
    /// Tempo changes in order, the first at `1.1.0`. Each holds until the next.
    pub tempo: Vec<TempoAt>,
    /// Meter changes in order, the first at `1.1.0`, each on a bar line.
    pub meter: Vec<MeterAt>,
    /// Key changes in order, the first at `1.1.0`, or none when no key is stated. The key is
    /// information for operations and the score: it changes no note.
    pub key: Vec<KeyAt>,
    /// Named spans in order, each from its `at` to the next section's, the last to the song's
    /// end. Names need not be unique.
    pub sections: Vec<Section>,
    /// Where each part sits. Several over the same bars are layered.
    pub placements: Vec<Placement>,
    /// Where the song ends, when it says; otherwise the latest end of any placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Position>,
}

impl Composition {
    /// A composition of one tempo, one meter and, when given, one key, from `1.1.0`, with no
    /// sections and nothing placed.
    #[must_use]
    pub fn new(bpm: f64, meter: TimeSig, key: Option<Key>) -> Self {
        let start = Position::bar(1);
        Self {
            ticks_per_quarter: TICKS_PER_QUARTER,
            tempo: vec![TempoAt { at: start, bpm }],
            meter: vec![MeterAt {
                at: start,
                sig: meter,
            }],
            key: key
                .map(|key| KeyAt { at: start, key })
                .into_iter()
                .collect(),
            sections: Vec::new(),
            placements: Vec::new(),
            end: None,
        }
    }

    /// A new song's composition: each of tempo, meter and key from its flag when given, else
    /// from the project's defaults, else 120 bpm, 4/4 and no key.
    #[must_use]
    pub fn starting(
        tempo: Option<f64>,
        meter: Option<TimeSig>,
        key: Option<Key>,
        defaults: Option<&Defaults>,
    ) -> Self {
        Self::new(
            tempo
                .or_else(|| defaults.and_then(|d| d.tempo_bpm))
                .unwrap_or(DEFAULT_BPM),
            meter
                .or_else(|| defaults.and_then(|d| d.meter))
                .unwrap_or_default(),
            key.or_else(|| defaults.and_then(|d| d.key)),
        )
    }

    /// The map from positions to ticks the meter map gives.
    ///
    /// # Errors
    ///
    /// When the map is empty, does not start at `1.1.0`, has a change off a bar line or out of
    /// order, or a meter the resolution cannot divide.
    pub fn time_map(&self) -> Result<TimeMap, String> {
        let Some(first) = self.meter.first() else {
            return Err("composition.meter is empty: a song has a meter from 1.1.0".into());
        };
        starts_at_one("composition.meter", first.at)?;
        let mut map = TimeMap::new(self.ticks_per_quarter, first.sig).map_err(|e| e.to_string())?;
        for (n, change) in self.meter.iter().enumerate().skip(1) {
            let at = change.at;
            if at.beat != 1 || at.tick != 0 {
                return Err(format!(
                    "composition.meter[{n}] changes the meter at {at}, off a bar line: a meter \
                     changes on a bar line, such as {}",
                    Position::bar(at.bar + 1)
                ));
            }
            if at.bar <= self.meter[n - 1].at.bar {
                return Err(format!(
                    "composition.meter[{n}] is at {at}, not after the change before it at {}: \
                     changes are in order, one to a place",
                    self.meter[n - 1].at
                ));
            }
            map.change_at(at, change.sig).map_err(|e| e.to_string())?;
        }
        Ok(map)
    }

    /// Checks what a schema cannot: each map starts at `1.1.0` and runs in order, one change to
    /// a place; a meter changes only on a bar line; every tempo is a tempo; every position is
    /// written as it falls on the song's bars; and a placement repeats at least once and plays
    /// a stretch that runs forward.
    ///
    /// # Errors
    ///
    /// A message naming the field and what is wrong with it.
    pub fn check(&self) -> Result<(), String> {
        let map = self.time_map()?;
        let real = |field: String, at: Position| -> Result<(), String> {
            let tick = map.tick(at).map_err(|e| format!("{field}: {e}"))?;
            let fall = map.position(tick);
            if fall == at {
                Ok(())
            } else {
                Err(format!(
                    "{field} is {at}, which is not how the song's bars write that place: it is \
                     {fall}, by the meter map"
                ))
            }
        };
        let in_order = |field: &str, ats: &[Position]| -> Result<(), String> {
            for (n, pair) in ats.windows(2).enumerate() {
                if pair[1] <= pair[0] {
                    return Err(format!(
                        "{field}[{}] is at {}, not after {field}[{n}] at {}: changes are in \
                         order, one to a place",
                        n + 1,
                        pair[1],
                        pair[0]
                    ));
                }
            }
            Ok(())
        };

        let Some(first) = self.tempo.first() else {
            return Err("composition.tempo is empty: a song has a tempo from 1.1.0".into());
        };
        starts_at_one("composition.tempo", first.at)?;
        for (n, change) in self.tempo.iter().enumerate() {
            real(format!("composition.tempo[{n}].at"), change.at)?;
            if !(change.bpm.is_finite() && change.bpm > 0.0) {
                return Err(format!(
                    "composition.tempo[{n}].bpm is {}: a tempo is a positive number of quarter \
                     notes per minute",
                    change.bpm
                ));
            }
        }
        in_order(
            "composition.tempo",
            &self.tempo.iter().map(|c| c.at).collect::<Vec<_>>(),
        )?;

        if let Some(first) = self.key.first() {
            starts_at_one("composition.key", first.at)?;
        }
        for (n, change) in self.key.iter().enumerate() {
            real(format!("composition.key[{n}].at"), change.at)?;
        }
        in_order(
            "composition.key",
            &self.key.iter().map(|c| c.at).collect::<Vec<_>>(),
        )?;

        for (n, section) in self.sections.iter().enumerate() {
            if section.name.trim().is_empty() {
                return Err(format!(
                    "composition.sections[{n}] has no name: a section is found by its name"
                ));
            }
            real(format!("composition.sections[{n}].at"), section.at)?;
        }
        in_order(
            "composition.sections",
            &self.sections.iter().map(|s| s.at).collect::<Vec<_>>(),
        )?;

        for (n, placement) in self.placements.iter().enumerate() {
            real(format!("composition.placements[{n}].at"), placement.at)?;
            if placement.repeat == Some(0) {
                return Err(format!(
                    "composition.placements[{n}].repeat is 0: a placed part plays at least once"
                ));
            }
            if let (Some(from), Some(to)) = (placement.from, placement.to)
                && to <= from
            {
                return Err(format!(
                    "composition.placements[{n}] plays from {from} to {to}, which runs \
                     backwards: `to` is after `from`"
                ));
            }
        }

        if let Some(end) = self.end {
            real("composition.end".into(), end)?;
        }
        Ok(())
    }
}

fn starts_at_one(field: &str, at: Position) -> Result<(), String> {
    if at == Position::bar(1) {
        Ok(())
    } else {
        Err(format!(
            "{field} starts at {at}: its first change is at 1.1.0, so the song's start has one"
        ))
    }
}

/// A tempo from a place on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TempoAt {
    pub at: Position,
    /// Quarter notes per minute.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub bpm: f64,
}

impl TempoAt {
    #[must_use]
    pub const fn new(at: Position, bpm: f64) -> Self {
        Self { at, bpm }
    }
}

/// A meter from a bar line on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MeterAt {
    pub at: Position,
    /// The time signature, such as 3/4.
    pub sig: TimeSig,
}

impl MeterAt {
    #[must_use]
    pub const fn new(at: Position, sig: TimeSig) -> Self {
        Self { at, sig }
    }
}

/// A key from a place on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct KeyAt {
    pub at: Position,
    /// The key, such as G minor.
    pub key: Key,
}

impl KeyAt {
    #[must_use]
    pub const fn new(at: Position, key: Key) -> Self {
        Self { at, key }
    }
}

/// A named span of the song, from its `at` to the next section's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Section {
    /// The owner's name for it, such as `verse 2`.
    pub name: String,
    pub at: Position,
}

impl Section {
    #[must_use]
    pub fn new(name: impl Into<String>, at: Position) -> Self {
        Self {
            name: name.into(),
            at,
        }
    }
}

/// Where a part sits on the song's bars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Placement {
    /// The part, by its stem in the song's `parts/`.
    pub part: Name,
    /// The place on the song's bars where the part's own `1.1.0` falls.
    pub at: Position,
    /// How many times the part plays back to back, for a groove. Absent is once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub repeat: Option<u32>,
    /// Where in the part, in its own positions, the stretch that plays starts. Absent is its
    /// start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Position>,
    /// Where in the part, in its own positions, the stretch that plays ends. Absent is its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Position>,
}

impl Placement {
    /// The whole part, once, at `at`.
    #[must_use]
    pub const fn new(part: Name, at: Position) -> Self {
        Self {
            part,
            at,
            repeat: None,
            from: None,
            to: None,
        }
    }
}

/// The parts a song's `parts/` holds, by name, from the names of the files in it: each `.duet`
/// file is a part (or a drum grid) named by its stem, in order of name. Any other file is not a
/// part, and is left out.
///
/// # Errors
///
/// A file whose stem is not a name, or two files that are one part's name, such as
/// `melody.duet` and `melody.DUET`, naming both.
pub fn part_names<'a>(
    files: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<(Name, &'a str)>, String> {
    let mut parts: Vec<(Name, &str)> = Vec::new();
    for file in files {
        let Some((stem, ext)) = file.rsplit_once('.') else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("duet") {
            continue;
        }
        let name: Name = stem
            .parse()
            .map_err(|e| format!("parts/{file} is not named as a part is: {e}"))?;
        if let Some((_, first)) = parts.iter().find(|(have, _)| *have == name) {
            return Err(format!(
                "two parts are named {:?}: parts/{first} and parts/{file}. A part is named by \
                 its stem, which is unique in its song",
                name.as_str()
            ));
        }
        parts.push((name, file));
    }
    parts.sort();
    Ok(parts)
}

/// Checks that no two of a song's parts share an id: a copy of a part is a new part, and mints
/// a new id.
///
/// # Errors
///
/// A message naming the two parts that share one.
pub fn distinct_ids<'a>(parts: impl IntoIterator<Item = (&'a Name, &'a Id)>) -> Result<(), String> {
    let mut seen: Vec<(&Name, &Id)> = Vec::new();
    for (name, id) in parts {
        if let Some((first, _)) = seen.iter().find(|(_, have)| *have == id) {
            return Err(format!(
                "parts {:?} and {:?} have one id, {id}: a copy of a part is a new part, with an \
                 id of its own",
                first.as_str(),
                name.as_str()
            ));
        }
        seen.push((name, id));
    }
    Ok(())
}
