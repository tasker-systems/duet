//! The drum grid: a conductor, a step, lanes, and one lanes × steps velocity matrix for the
//! whole grid. These types are the exact form: they serialize with serde as a drum-grid
//! document's body, and their `JsonSchema` is that body's schema.

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};

use duet_core::{
    Grid, Key, KeyChange, Position, Song, TempoChange, Tensor, TimeChange, TimeMap, TimeSig,
};

use crate::error::{Error, Result};
use crate::lanes::valid_lane_name;

/// A drum part as a matrix: [`DrumGrid::vel`] is one lanes × steps tensor for the whole
/// grid, whose row for a lane is every step of every bar in order, and 0 means no hit.
/// [`DrumGrid::bar_steps`] gives each bar's step count, from bar 1, so bar lines are read
/// from it.
///
/// The conductor (resolution, tempo, meter, key and their changes) means exactly what
/// the same fields of a [`Song`] mean. Every bar must be a whole number of steps of
/// [`DrumGrid::grid`], so each column is one step wide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DrumGrid {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[schemars(range(min = 1, max = 32767))]
    pub ticks_per_quarter: u16,
    pub tempo_bpm: f64,
    pub time_sig: TimeSig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_changes: Vec<TempoChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_changes: Vec<TimeChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_changes: Vec<KeyChange>,
    /// Where the grid stops: a position on a step of its last bar, with no hit at or
    /// after it, or the bar line that ends that bar. `None` means that bar line, and is
    /// the form an end on it takes once [normalized](DrumGrid::normalize) or read; the
    /// text form always states the end either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Position>,
    /// One step, such as `1/16`.
    pub grid: Grid,
    /// The MIDI channel the lanes play on, 1 to 16; 10 is General MIDI's drum channel.
    #[serde(default = "drum_channel")]
    #[schemars(range(min = 1, max = 16))]
    pub channel: u8,
    /// The velocities the text form's cells stand for.
    #[serde(default, skip_serializing_if = "Levels::is_default")]
    pub levels: Levels,
    /// The rows, top to bottom.
    pub lanes: Vec<Lane>,
    /// How many steps each bar holds, from bar 1, with none left out. Bars differ when the
    /// meter changes. The sum is the number of columns of [`DrumGrid::vel`].
    pub bar_steps: Vec<u32>,
    /// The velocities: lanes × steps, row-major, so lane `l` at step `s` of the whole grid is
    /// `data[l * steps + s]`. 0 is no hit.
    #[serde(deserialize_with = "vel_field")]
    #[schemars(schema_with = "vel_schema")]
    pub vel: Tensor<u8>,
}

fn drum_channel() -> u8 {
    10
}

/// Reads `vel`, naming it when the tensor is refused.
fn vel_field<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Tensor<u8>, D::Error> {
    Tensor::deserialize(deserializer).map_err(|e| serde::de::Error::custom(format!("vel: {e}")))
}

/// `vel`'s schema: a `u8` tensor whose velocities run 0 to 127.
fn vel_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = Tensor::<u8>::json_schema(generator);
    if let Some(data) = schema
        .get_mut("properties")
        .and_then(|p| p.get_mut("data"))
        .and_then(serde_json::Value::as_object_mut)
    {
        data.insert(
            "items".into(),
            serde_json::json!({ "type": "integer", "minimum": 0, "maximum": 127 }),
        );
    }
    schema
}

/// One row of the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    /// Lowercase letters, digits and hyphens.
    #[schemars(regex(pattern = r"^[a-z0-9]+(-[a-z0-9]+)*$"))]
    pub name: String,
    /// The MIDI key it plays.
    #[schemars(range(max = 127))]
    pub key: u8,
}

/// The velocities of the text form's three hit cells: `g` ghost, `x` hit and `X` accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Levels {
    #[schemars(range(min = 1, max = 127))]
    pub ghost: u8,
    #[schemars(range(min = 1, max = 127))]
    pub hit: u8,
    #[schemars(range(min = 1, max = 127))]
    pub accent: u8,
}

impl Default for Levels {
    fn default() -> Self {
        Levels {
            ghost: 40,
            hit: 90,
            accent: 120,
        }
    }
}

impl Levels {
    fn is_default(&self) -> bool {
        *self == Levels::default()
    }

    /// Checks that the levels rise from ghost to accent within 1 to 127.
    pub fn check(&self) -> Result<()> {
        if !(1 <= self.ghost
            && self.ghost < self.hit
            && self.hit < self.accent
            && self.accent <= 127)
        {
            return Err(Error::invalid(format!(
                "velocity levels g {} x {} X {} must rise from g to X, within 1 to 127",
                self.ghost, self.hit, self.accent
            )));
        }
        Ok(())
    }

    /// The cell for a velocity, or `None` when the velocity is not one of the levels.
    pub fn cell(&self, vel: u8) -> Option<char> {
        match vel {
            0 => Some('.'),
            v if v == self.ghost => Some('g'),
            v if v == self.hit => Some('x'),
            v if v == self.accent => Some('X'),
            _ => None,
        }
    }

    /// The velocity a cell stands for, or `None` when it is not a cell.
    pub fn vel(&self, cell: &str) -> Option<u8> {
        match cell {
            "." => Some(0),
            "g" => Some(self.ghost),
            "x" => Some(self.hit),
            "X" => Some(self.accent),
            _ => None,
        }
    }

    /// The nearest level to a velocity; 0 stays 0. A velocity exactly halfway between
    /// two levels goes to the louder one.
    pub fn nearest(&self, vel: u8) -> u8 {
        if vel == 0 {
            return 0;
        }
        let mut best = self.ghost;
        for level in [self.hit, self.accent] {
            if vel.abs_diff(level) <= vel.abs_diff(best) {
                best = level;
            }
        }
        best
    }
}

/// Where one bar sits and how many steps it holds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BarShape {
    pub start: u64,
    pub steps: u32,
    pub sig: TimeSig,
}

impl DrumGrid {
    /// An empty grid with this song's conductor: no lanes, no bars and no end.
    pub fn with_conductor(song: &Song, grid: Grid) -> DrumGrid {
        DrumGrid {
            title: song.title.clone(),
            ticks_per_quarter: song.ticks_per_quarter,
            tempo_bpm: song.tempo_bpm,
            time_sig: song.time_sig,
            key: song.key,
            tempo_changes: song.tempo_changes.clone(),
            time_changes: song.time_changes.clone(),
            key_changes: song.key_changes.clone(),
            end: None,
            grid,
            channel: drum_channel(),
            levels: Levels::default(),
            lanes: Vec::new(),
            bar_steps: Vec::new(),
            vel: Tensor::empty(),
        }
    }

    /// How many bars the grid holds.
    #[must_use]
    pub fn bar_count(&self) -> u32 {
        self.bar_steps.len() as u32
    }

    /// How many steps the whole grid holds: the columns of [`DrumGrid::vel`].
    #[must_use]
    pub fn steps(&self) -> usize {
        self.bar_steps.iter().map(|s| *s as usize).sum()
    }

    /// The column bar `bar` (from 1) starts at.
    #[must_use]
    pub fn bar_start(&self, bar: u32) -> usize {
        self.bar_steps[..(bar as usize - 1).min(self.bar_steps.len())]
            .iter()
            .map(|s| *s as usize)
            .sum()
    }

    /// Lane `lane`'s velocities over the whole grid.
    #[must_use]
    pub fn row(&self, lane: usize) -> &[u8] {
        let steps = self.steps();
        &self.vel.data()[lane * steps..(lane + 1) * steps]
    }

    /// Lane `lane`'s velocities in bar `bar` (from 1).
    #[must_use]
    pub fn bar_row(&self, lane: usize, bar: u32) -> &[u8] {
        let start = self.bar_start(bar);
        &self.row(lane)[start..start + self.bar_steps[bar as usize - 1] as usize]
    }

    /// Sets the matrix: `bar_steps` and one row per lane, in lane order, each the sum of
    /// `bar_steps` long.
    ///
    /// # Errors
    ///
    /// When a row is not that long, or there is not one row per lane.
    pub fn set_matrix(&mut self, bar_steps: Vec<u32>, rows: Vec<Vec<u8>>) -> Result<()> {
        let steps: usize = bar_steps.iter().map(|s| *s as usize).sum();
        if rows.len() != self.lanes.len() {
            return Err(Error::invalid(format!(
                "{} rows for {} lanes",
                rows.len(),
                self.lanes.len()
            )));
        }
        if let Some((lane, row)) = self.lanes.iter().zip(&rows).find(|(_, r)| r.len() != steps) {
            return Err(Error::invalid(format!(
                "{} has {} steps; the bars hold {steps}",
                lane.name,
                row.len()
            )));
        }
        self.vel = Tensor::from_parts(
            vec![rows.len(), steps],
            rows.into_iter().flatten().collect(),
        )?;
        self.bar_steps = bar_steps;
        Ok(())
    }

    /// The conductor, and the grid's end, as a song with no tracks.
    pub fn conductor(&self) -> Song {
        let mut song = Song::default();
        song.title = self.title.clone();
        song.ticks_per_quarter = self.ticks_per_quarter;
        song.tempo_bpm = self.tempo_bpm;
        song.time_sig = self.time_sig;
        song.key = self.key;
        song.tempo_changes = self.tempo_changes.clone();
        song.time_changes = self.time_changes.clone();
        song.key_changes = self.key_changes.clone();
        song.end = self.end;
        song
    }

    pub(crate) fn time_map(&self) -> Result<TimeMap> {
        Ok(self.conductor().time_map()?)
    }

    pub(crate) fn step_ticks(&self) -> Result<u64> {
        Ok(self.grid.step_ticks(self.ticks_per_quarter)?)
    }

    /// The shape of bar `bar` (from 1): where it starts and how many steps it holds.
    /// Fails, naming the bar, when it is not a whole number of steps.
    pub(crate) fn bar_shape(&self, map: &TimeMap, step: u64, bar: u32) -> Result<BarShape> {
        let start = map.tick(Position::bar(bar))?;
        let end = map.tick(Position::bar(bar + 1))?;
        let sig = map.sig_at(start);
        let len = end - start;
        if !len.is_multiple_of(step) {
            let steps = len as f64 / step as f64;
            return Err(Error::invalid(format!(
                "bar {bar} ({sig}) is {steps} steps of {}; a drum grid needs whole steps, \
                 so choose a finer grid",
                self.grid
            )));
        }
        Ok(BarShape {
            start,
            steps: (len / step) as u32,
            sig,
        })
    }

    /// Checks everything the matrix promises: a valid conductor and grid, valid and
    /// distinct lanes, rising velocity levels, and bars numbered from 1, each a whole
    /// number of steps with one row per lane of exactly that many velocities.
    pub fn check(&self) -> Result<()> {
        let map = self.time_map()?;
        let step = self.step_ticks()?;
        if !(1..=16).contains(&self.channel) {
            return Err(Error::invalid(format!(
                "channel {} is not 1 to 16",
                self.channel
            )));
        }
        self.levels.check()?;
        for (i, lane) in self.lanes.iter().enumerate() {
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
            if let Some(other) = self.lanes[..i]
                .iter()
                .find(|o| o.name == lane.name || o.key == lane.key)
            {
                return Err(Error::invalid(format!(
                    "lanes {} ({}) and {} ({}) share a name or a key",
                    other.name, other.key, lane.name, lane.key
                )));
            }
        }
        let want = [self.lanes.len(), self.steps()];
        if self.vel.shape() != &want[..] {
            return Err(Error::invalid(format!(
                "vel has shape {:?}; {} lanes over the bars' {} steps need {want:?}",
                self.vel.shape(),
                self.lanes.len(),
                self.steps()
            )));
        }
        for (i, steps) in self.bar_steps.iter().enumerate() {
            let number = i as u32 + 1;
            let shape = self.bar_shape(&map, step, number)?;
            if *steps != shape.steps {
                return Err(Error::invalid(format!(
                    "bar {number} says {steps} steps; {} at {} needs {}",
                    shape.sig, self.grid, shape.steps
                )));
            }
        }
        for (l, lane) in self.lanes.iter().enumerate() {
            if let Some(s) = self.row(l).iter().position(|v| *v > 127) {
                let (bar, at) = self.bar_and_step(s);
                return Err(Error::invalid(format!(
                    "{}, bar {bar}, step {at}: velocity {} is not 0 to 127",
                    lane.name,
                    self.row(l)[s]
                )));
            }
        }
        self.check_end(&map, step)?;
        // Nothing in the conductor may lie past where the grid ends.
        let mut conductor = self.conductor();
        conductor.end = Some(self.stated_end());
        conductor.check_end()?;
        Ok(())
    }

    /// Checks the grid's end, when it has one: on a step inside the last bar, with no hit
    /// at or after it.
    fn check_end(&self, map: &TimeMap, step: u64) -> Result<()> {
        let Some(end) = self.end else {
            return Ok(());
        };
        let tick = map.tick(end)?;
        let end = map.position(tick);
        let last = self.bar_count();
        if last == 0 {
            return Err(Error::invalid(format!(
                "the grid ends at {end} but has no bars; a grid's end lies inside its last bar"
            )));
        }
        let shape = self.bar_shape(map, step, last)?;
        let bars_end = shape.start + u64::from(shape.steps) * step;
        if tick <= shape.start || tick > bars_end {
            return Err(Error::invalid(format!(
                "the grid ends at {end}, but its last bar, bar {}, runs to {}; an end lies \
                 inside the last bar or on the bar line that ends it",
                last,
                map.position(bars_end)
            )));
        }
        let offset = tick - shape.start;
        if !offset.is_multiple_of(step) {
            return Err(Error::invalid(format!(
                "the grid ends at {end}, which is not on the {} grid",
                self.grid
            )));
        }
        let first_past = (offset / step) as usize;
        for (l, lane) in self.lanes.iter().enumerate() {
            let row = self.bar_row(l, last);
            if let Some(s) = row[first_past..].iter().position(|v| *v > 0) {
                let s = first_past + s;
                return Err(Error::invalid(format!(
                    "{}, bar {}, step {}: the hit at {} lies at or past the grid's end at {end}; \
                     move the end, or the hit",
                    lane.name,
                    last,
                    s + 1,
                    map.position(shape.start + s as u64 * step)
                )));
            }
        }
        Ok(())
    }

    /// Where the grid's bars end: the bar line after its last bar.
    pub(crate) fn bars_end(&self) -> Position {
        Position::bar(self.bar_count() + 1)
    }

    /// Where the grid ends: its own end, or else the bar line after its last bar.
    pub fn stated_end(&self) -> Position {
        self.end.unwrap_or_else(|| self.bars_end())
    }

    /// Writes an end on the last bar line as no end, the one form both have.
    pub(crate) fn canonical_end(&mut self) -> Result<()> {
        if let Some(end) = self.end {
            let map = self.time_map()?;
            if map.tick(end)? == map.tick(self.bars_end())? {
                self.end = None;
            }
        }
        Ok(())
    }

    /// Puts the conductor in the form a song's normalizing gives (tempos as MIDI stores
    /// them, positions in canonical bar.beat.tick form), then [checks](DrumGrid::check)
    /// the grid. Hits are not touched.
    pub fn normalize(&mut self) -> Result<()> {
        let mut conductor = self.conductor();
        conductor.normalize()?;
        let grid = DrumGrid::with_conductor(&conductor, self.grid);
        self.tempo_bpm = grid.tempo_bpm;
        self.tempo_changes = grid.tempo_changes;
        self.time_changes = grid.time_changes;
        self.key_changes = grid.key_changes;
        self.end = conductor.end;
        self.check()?;
        self.canonical_end()
    }

    /// A copy with every hit's velocity moved to the nearest of [`DrumGrid::levels`]
    /// ([`Levels::nearest`]), so the text form can write it. This grid is not changed;
    /// nothing snaps velocities unless this is called.
    pub fn snapped_to_levels(&self) -> DrumGrid {
        let mut out = self.clone();
        for v in out.vel.data_mut() {
            *v = self.levels.nearest(*v);
        }
        out
    }

    /// The first hit whose velocity is not one of the levels, as (lane, bar, step, vel),
    /// with bar and step counted from 1.
    pub fn first_off_level(&self) -> Option<(&str, u32, u32, u8)> {
        let steps = self.steps();
        (0..steps)
            .flat_map(|s| (0..self.lanes.len()).map(move |l| (l, s)))
            .find(|(l, s)| self.levels.cell(self.row(*l)[*s]).is_none())
            .map(|(l, s)| {
                let (bar, at) = self.bar_and_step(s);
                (self.lanes[l].name.as_str(), bar, at, self.row(l)[s])
            })
    }

    /// The bar (from 1) a column of the whole grid falls in, and its step in that bar (from 1).
    fn bar_and_step(&self, column: usize) -> (u32, u32) {
        let mut start = 0;
        for (i, steps) in self.bar_steps.iter().enumerate() {
            let steps = *steps as usize;
            if column < start + steps {
                return (i as u32 + 1, (column - start) as u32 + 1);
            }
            start += steps;
        }
        (self.bar_count(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_goes_to_the_nearest_level_and_halfway_goes_louder() {
        let l = Levels::default();
        let cases = [
            (0, 0),
            (1, 40),
            (40, 40),
            (64, 40),
            (65, 90),
            (66, 90),
            (105, 120),
            (104, 90),
            (127, 120),
        ];
        for (vel, want) in cases {
            assert_eq!(l.nearest(vel), want, "velocity {vel}");
        }
    }

    #[test]
    fn levels_must_rise() {
        for (g, x, big) in [(0, 90, 120), (90, 90, 120), (40, 130, 120), (40, 90, 128)] {
            let levels = Levels {
                ghost: g,
                hit: x,
                accent: big,
            };
            assert!(levels.check().is_err(), "{levels:?}");
        }
        assert!(Levels::default().check().is_ok());
    }
}
