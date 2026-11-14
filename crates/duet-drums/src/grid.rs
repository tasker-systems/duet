//! The drum grid: a conductor, a step, lanes, and a lanes × steps velocity matrix per bar.
//! These types are the exact form, and serialize to JSON and TOML with serde.

use serde::{Deserialize, Serialize};

use duet_core::{Grid, Key, KeyChange, Position, Song, TempoChange, TimeChange, TimeMap, TimeSig};

use crate::error::{Error, Result};
use crate::lanes::valid_lane_name;

/// A drum part as a matrix: `bars[b].vel[lane][step]` is the velocity of the hit on that
/// lane at that step of bar `b + 1`, and 0 means no hit.
///
/// The conductor (resolution, tempo, meter, key and their changes) means exactly what
/// the same fields of a [`Song`] mean. Every bar must be a whole number of steps of
/// [`DrumGrid::grid`], so each column is one step wide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrumGrid {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    pub channel: u8,
    /// The velocities the text form's cells stand for.
    #[serde(default, skip_serializing_if = "Levels::is_default")]
    pub levels: Levels,
    /// The rows, top to bottom.
    pub lanes: Vec<Lane>,
    /// Every bar from bar 1, in order.
    pub bars: Vec<Bar>,
}

fn drum_channel() -> u8 {
    10
}

/// One row of the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    pub name: String,
    /// The MIDI key it plays.
    pub key: u8,
}

/// One bar of the matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bar {
    /// The bar number, from 1.
    pub bar: u32,
    /// How many steps the bar holds.
    pub steps: u32,
    /// One row per lane, in lane order, each `steps` long; 0 is no hit.
    pub vel: Vec<Vec<u8>>,
}

/// The velocities of the text form's three hit cells: `g` ghost, `x` hit and `X` accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Levels {
    pub ghost: u8,
    pub hit: u8,
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
            bars: Vec::new(),
        }
    }

    /// The conductor, and the grid's end, as a song with no tracks.
    pub fn conductor(&self) -> Song {
        Song {
            title: self.title.clone(),
            ticks_per_quarter: self.ticks_per_quarter,
            tempo_bpm: self.tempo_bpm,
            time_sig: self.time_sig,
            key: self.key,
            tempo_changes: self.tempo_changes.clone(),
            time_changes: self.time_changes.clone(),
            key_changes: self.key_changes.clone(),
            end: self.end,
            tracks: Vec::new(),
        }
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
        for (i, bar) in self.bars.iter().enumerate() {
            let number = i as u32 + 1;
            if bar.bar != number {
                return Err(Error::invalid(format!(
                    "bar {} is listed where bar {number} belongs; bars run from 1 with none left out",
                    bar.bar
                )));
            }
            let shape = self.bar_shape(&map, step, number)?;
            if bar.steps != shape.steps {
                return Err(Error::invalid(format!(
                    "bar {number} says {} steps; {} at {} needs {}",
                    bar.steps, shape.sig, self.grid, shape.steps
                )));
            }
            if bar.vel.len() != self.lanes.len() {
                return Err(Error::invalid(format!(
                    "bar {number} has {} rows for {} lanes",
                    bar.vel.len(),
                    self.lanes.len()
                )));
            }
            for (lane, row) in self.lanes.iter().zip(&bar.vel) {
                if row.len() != shape.steps as usize {
                    return Err(Error::invalid(format!(
                        "{}, bar {number} has {} steps; {} at {} needs {}",
                        lane.name,
                        row.len(),
                        shape.sig,
                        self.grid,
                        shape.steps
                    )));
                }
                if let Some(v) = row.iter().find(|v| **v > 127) {
                    return Err(Error::invalid(format!(
                        "{}, bar {number}: velocity {v} is not 0 to 127",
                        lane.name
                    )));
                }
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
        let Some(last) = self.bars.last() else {
            return Err(Error::invalid(format!(
                "the grid ends at {end} but has no bars; a grid's end lies inside its last bar"
            )));
        };
        let shape = self.bar_shape(map, step, last.bar)?;
        let bars_end = shape.start + u64::from(shape.steps) * step;
        if tick <= shape.start || tick > bars_end {
            return Err(Error::invalid(format!(
                "the grid ends at {end}, but its last bar, bar {}, runs to {}; an end lies \
                 inside the last bar or on the bar line that ends it",
                last.bar,
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
        for (lane, row) in self.lanes.iter().zip(&last.vel) {
            if let Some(s) = row[first_past..].iter().position(|v| *v > 0) {
                let s = first_past + s;
                return Err(Error::invalid(format!(
                    "{}, bar {}, step {}: the hit at {} lies at or past the grid's end at {end}; \
                     move the end, or the hit",
                    lane.name,
                    last.bar,
                    s + 1,
                    map.position(shape.start + s as u64 * step)
                )));
            }
        }
        Ok(())
    }

    /// Where the grid's bars end: the bar line after its last bar.
    pub(crate) fn bars_end(&self) -> Position {
        Position::bar(self.bars.len() as u32 + 1)
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
        for bar in &mut out.bars {
            for row in &mut bar.vel {
                for v in row {
                    *v = self.levels.nearest(*v);
                }
            }
        }
        out
    }

    /// The first hit whose velocity is not one of the levels, as (lane, bar, step, vel),
    /// with bar and step counted from 1.
    pub fn first_off_level(&self) -> Option<(&str, u32, u32, u8)> {
        for bar in &self.bars {
            for (lane, row) in self.lanes.iter().zip(&bar.vel) {
                if let Some(step) = row.iter().position(|v| self.levels.cell(*v).is_none()) {
                    return Some((&lane.name, bar.bar, step as u32 + 1, row[step]));
                }
            }
        }
        None
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
