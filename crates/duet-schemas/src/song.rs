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
//! - **The song links takes** ([`TakeLink`]), each by its project name and its digest. A link
//!   may say which parts the take is for, and may be placed on the song's bars ([`Placed`]): by
//!   the click the take was played to, by an estimate of its tempo, or by the owner's word
//!   ([`By`]). The take's audio is never written here; the link only says where it falls.
//!
//! The design is `docs/superpowers/specs/2026-11-17-songs-and-composition-design.md`, §1 and §7.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use duet_core::{Key, Position, TimeMap, TimeSig};
use duet_dsp::TakeDigest;
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
    /// The takes the song links, each by its name and digest, in the order they were linked.
    /// Written always, and read as none when a song written before takes has no such key.
    #[serde(default)]
    pub takes: Vec<TakeLink>,
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
            takes: Vec::new(),
        }
    }

    /// The link to the take of this name, if the song has one.
    #[must_use]
    pub fn link(&self, take: &Name) -> Option<&TakeLink> {
        self.takes.iter().find(|link| &link.take == take)
    }

    /// The link to the take of this name, to change, if the song has one.
    pub fn link_mut(&mut self, take: &Name) -> Option<&mut TakeLink> {
        self.takes.iter_mut().find(|link| &link.take == take)
    }

    /// The link to the take with this digest, if the song has one. A take is the same take
    /// under any name, so this finds it by its audio.
    #[must_use]
    pub fn linking(&self, digest: &TakeDigest) -> Option<&TakeLink> {
        self.takes.iter().find(|link| &link.digest == digest)
    }

    /// Says which links are `for` a part the song does not have, one line each. A take is
    /// usually linked before its part is written, so this is stated and never an error.
    #[must_use]
    pub fn check_for<'a>(&self, parts: impl IntoIterator<Item = &'a Name>) -> Vec<String> {
        let parts: BTreeSet<&Name> = parts.into_iter().collect();
        let mut lines = Vec::new();
        for link in &self.takes {
            for part in &link.r#for {
                if !parts.contains(part) {
                    lines.push(format!(
                        "the take {:?} is for the part {:?}, and the song has no part of that \
                         name yet",
                        link.take.as_str(),
                        part.as_str()
                    ));
                }
            }
        }
        lines
    }

    /// Checks what a schema cannot: the title says something, the composition holds
    /// ([`Composition::check`]), and the takes it links hold ([`Song::check_takes`]).
    ///
    /// # Errors
    ///
    /// A message naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("the song's title is empty: a song has a name for people".into());
        }
        self.composition.check()?;
        self.check_takes(&self.composition.time_map()?)
    }

    /// Checks the links: one to a take by name and one by digest; each `for` names a part
    /// once; and each placement is as its kind says, with a `bar` the song's bars hold, written
    /// as it falls, a finite `at_secs` of at least 0, and a tempo that is a tempo. `bpm` is
    /// given for `estimated` and `set` and not for `click`; `clock_ppm` only for `click`.
    ///
    /// # Errors
    ///
    /// A message naming the field and what is wrong with it.
    pub fn check_takes(&self, map: &TimeMap) -> Result<(), String> {
        for (n, link) in self.takes.iter().enumerate() {
            if let Some(m) = self.takes[..n].iter().position(|had| had.take == link.take) {
                return Err(format!(
                    "takes[{n}] links the take {:?}, which takes[{m}] already links: a song \
                     links a take once",
                    link.take.as_str()
                ));
            }
            if let Some(m) = self.takes[..n]
                .iter()
                .position(|had| had.digest == link.digest)
            {
                return Err(format!(
                    "takes[{n}] links the take {:?} by the digest {}, which takes[{m}] ({:?}) \
                     already links: a song links one take's audio once",
                    link.take.as_str(),
                    link.digest,
                    self.takes[m].take.as_str()
                ));
            }
            for (k, part) in link.r#for.iter().enumerate() {
                if link.r#for[..k].contains(part) {
                    return Err(format!(
                        "takes[{n}].for names the part {:?} twice: each part is named once",
                        part.as_str()
                    ));
                }
            }
            if let Some(placed) = &link.placed {
                placed.check(&format!("takes[{n}].placed"), map)?;
            }
        }
        Ok(())
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

    /// The tempo in force at `at`, in quarter notes per minute: the latest change at or before
    /// it, held until the next.
    ///
    /// # Errors
    ///
    /// When the meter map is wrong, `at` is not a place the bars hold, or no tempo change is in
    /// force there.
    pub fn tempo_at(&self, at: Position) -> Result<f64, String> {
        let map = self.time_map()?;
        let tick = map.tick(at).map_err(|e| e.to_string())?;
        let mut held = None;
        for change in &self.tempo {
            if map.tick(change.at).map_err(|e| e.to_string())? <= tick {
                held = Some(change.bpm);
            }
        }
        held.ok_or_else(|| format!("no tempo is in force at {at}: the first change is at 1.1.0"))
    }

    /// The meter in force at `at`.
    ///
    /// # Errors
    ///
    /// When the meter map is wrong, or `at` is not a place the bars hold.
    pub fn meter_at(&self, at: Position) -> Result<TimeSig, String> {
        let map = self.time_map()?;
        let tick = map.tick(at).map_err(|e| e.to_string())?;
        Ok(map.sig_at(tick))
    }

    /// The key in force at `at`, or none when the song states no key by then.
    #[must_use]
    pub fn key_at(&self, at: Position) -> Option<Key> {
        let map = self.time_map().ok()?;
        let tick = map.tick(at).ok()?;
        let mut held = None;
        for change in &self.key {
            if map.tick(change.at).ok()? <= tick {
                held = Some(change.key);
            }
        }
        held
    }

    /// The seconds from `from` to `to` through the tempo map, each tempo held until the next
    /// change. Negative when `to` is before `from`.
    ///
    /// # Errors
    ///
    /// When the meter map is wrong, either place is not one the bars hold, or the tempo map is
    /// empty or holds a tempo that is not one.
    pub fn secs_between(&self, from: Position, to: Position) -> Result<f64, String> {
        let map = self.time_map()?;
        let a = map.tick(from).map_err(|e| e.to_string())?;
        let b = map.tick(to).map_err(|e| e.to_string())?;
        let (lo, hi, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
        if self.tempo.is_empty() {
            return Err("composition.tempo is empty: a song has a tempo from 1.1.0".into());
        }
        let mut starts = Vec::with_capacity(self.tempo.len());
        for change in &self.tempo {
            if !(change.bpm.is_finite() && change.bpm > 0.0) {
                return Err(format!("the tempo {} is not a tempo", change.bpm));
            }
            starts.push(map.tick(change.at).map_err(|e| e.to_string())?);
        }
        let tpq = f64::from(self.ticks_per_quarter);
        let mut secs = 0.0;
        for (n, change) in self.tempo.iter().enumerate() {
            let begin = starts[n].max(lo);
            let end = starts.get(n + 1).map_or(hi, |next| (*next).min(hi));
            if end > begin {
                secs += (end - begin) as f64 / tpq * 60.0 / change.bpm;
            }
        }
        Ok(sign * secs)
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
        let real = |field: String, at: Position| real_place(&map, field, at);
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

/// Checks that `at` is a place the bars hold and is written as the bars write it.
fn real_place(map: &TimeMap, field: String, at: Position) -> Result<(), String> {
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

/// A take a song links: the take by its project name and its digest, the parts it is for, and
/// where it falls on the song's bars once placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TakeLink {
    /// The take's name in the project.
    pub take: Name,
    /// The take's digest: the song finds the take by its audio, whatever it is called.
    pub digest: TakeDigest,
    /// The parts the take is for, by name, each once. A part need not be written yet.
    #[serde(default, rename = "for")]
    pub r#for: Vec<Name>,
    /// Where the take falls on the song's bars. Absent when the link is not placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed: Option<Placed>,
}

impl TakeLink {
    /// A link to a take, for no part and not placed.
    #[must_use]
    pub const fn new(take: Name, digest: TakeDigest) -> Self {
        Self {
            take,
            digest,
            r#for: Vec::new(),
            placed: None,
        }
    }
}

/// How a take was placed on the song's bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum By {
    /// The take was played to the song's click: the composition's tempo map is its tempo.
    Click,
    /// The take's tempo was estimated from its audio.
    Estimated,
    /// The owner stated the place and the tempo.
    Set,
}

impl By {
    /// The kind's name, as the document writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Estimated => "estimated",
            Self::Set => "set",
        }
    }
}

impl fmt::Display for By {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a take falls on the song's bars: the place on the bars where its own start falls, and
/// the seconds into the take where that place sounds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Placed {
    /// How it was placed.
    pub by: By,
    /// The place on the song's bars.
    pub bar: Position,
    /// The seconds into the take at which `bar` falls. At least 0.
    pub at_secs: f64,
    /// The take's tempo in quarter notes per minute. Present for `estimated` and `set`, absent
    /// for `click`, where the composition's tempo map is the take's tempo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// How far the take's clock ran from the click's, in parts per million, positive when the
    /// take's ran fast: a moment `t` seconds after `bar` on the click's clock falls
    /// `at_secs + t * (1 + clock_ppm / 1,000,000)` seconds into the take. For `click` only.
    /// Absent means no drift was measured or applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_ppm: Option<f64>,
}

impl Placed {
    /// A take played to the song's click, falling at `at_secs` into it.
    #[must_use]
    pub const fn click(bar: Position, at_secs: f64, clock_ppm: Option<f64>) -> Self {
        Self {
            by: By::Click,
            bar,
            at_secs,
            bpm: None,
            clock_ppm,
        }
    }

    /// A take whose tempo was estimated from its audio.
    #[must_use]
    pub const fn estimated(bar: Position, at_secs: f64, bpm: f64) -> Self {
        Self {
            by: By::Estimated,
            bar,
            at_secs,
            bpm: Some(bpm),
            clock_ppm: None,
        }
    }

    /// A take placed by the owner's word, at the tempo the owner gave.
    #[must_use]
    pub const fn set(bar: Position, at_secs: f64, bpm: f64) -> Self {
        Self {
            by: By::Set,
            bar,
            at_secs,
            bpm: Some(bpm),
            clock_ppm: None,
        }
    }

    fn check(&self, field: &str, map: &TimeMap) -> Result<(), String> {
        real_place(map, format!("{field}.bar"), self.bar)?;
        if !(self.at_secs.is_finite() && self.at_secs >= 0.0) {
            return Err(format!(
                "{field}.at_secs is {}: a place in a take is a finite number of seconds, at \
                 least 0",
                self.at_secs
            ));
        }
        match (self.by, self.bpm) {
            (By::Click, Some(bpm)) => {
                return Err(format!(
                    "{field}.bpm is {bpm}, and a click placement has none: the song's tempo map \
                     is the take's tempo"
                ));
            }
            (By::Estimated | By::Set, None) => {
                return Err(format!(
                    "{field}.bpm is missing: a placement by {} gives the take's tempo",
                    self.by
                ));
            }
            (_, Some(bpm)) if !(bpm.is_finite() && bpm > 0.0) => {
                return Err(format!(
                    "{field}.bpm is {bpm}: a tempo is a positive number of quarter notes per \
                     minute"
                ));
            }
            _ => {}
        }
        if let Some(ppm) = self.clock_ppm {
            if self.by != By::Click {
                return Err(format!(
                    "{field}.clock_ppm is {ppm}, and a placement by {} has none: only a click \
                     placement states how far the take's clock ran from the click's",
                    self.by
                ));
            }
            if !ppm.is_finite() {
                return Err(format!("{field}.clock_ppm is {ppm}: it is a finite number"));
            }
        }
        Ok(())
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
