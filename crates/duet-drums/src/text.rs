//! The drum grid's text form: one row per lane, one column per step, `|` at bar lines.
//!
//! ```text
//! # notefmt drums v1: one row per lane, one column per step, | at bar lines
//! title "Groove in D"
//! tpq 480
//! tempo 96
//! time 4/4
//! grid 1/16
//! map gm
//!
//! [bars 1-2]
//! #       1 . . . 2 . . . 3 . . . 4 . . .   1 . . . 2 . . . 3 . . . 4 . . .
//! hh    | x . x . x . x . x . x . x . x . | x . x . x . x . x . x . x . . . |
//! snare | . . . . X . . . . . . g X . . . | . . . . X . . . . . . . X . g . |
//! kick  | X . . . . . . X . . X . . . . . | X . . . . . . X . . X . . X . . |
//! ```
//!
//! The header is the notefmt header (title, tpq, tempo, time, key and their `at`
//! changes, and `end`, which in a grid is only for one that stops inside its last bar)
//! plus five lines of its own:
//!
//! - `grid 1/16`: the step. Required.
//! - `map gm`: the lane map that names the rows. `gm` is the default and the only one.
//! - `lane <name> <key>`: adds a lane to the map, or moves one to another key.
//! - `channel <n>`: the MIDI channel, when it is not the map's (10).
//! - `vel g 40 x 90 X 120`: the velocities of the cells, when they are not these.
//!
//! A cell is `.` (no hit), `g` (ghost), `x` (hit) or `X` (accent), and cells are
//! separated by single spaces. Blocks cover the bars in order, from bar 1, with none left
//! out, and every block lists the same lanes in the same order, so the text always has
//! the matrix's shape. Each bar's cell count is checked against its meter and the grid,
//! and a wrong count names the line, the lane, the bar and both counts.
//!
//! The row above each block that starts with `#` counts the beats, and like every line
//! starting with `#` it is a comment.

use std::fmt::Write as _;

use duet_core::{Grid, Song};

use crate::error::{Error, Result};
use crate::grid::{Bar, BarShape, DrumGrid, Lane, Levels};
use crate::lanes::{LaneMap, valid_lane_name};

const HEADER_COMMENT: &str =
    "# notefmt drums v1: one row per lane, one column per step, | at bar lines";

/// The most bars, and the most steps, the writer puts in one block.
const BLOCK_BARS: usize = 4;
const BLOCK_STEPS: u32 = 64;

/// Everything the header says.
struct Header {
    conductor: Song,
    grid: Option<Grid>,
    map: String,
    lanes: Vec<Lane>,
    channel: Option<u8>,
    levels: Levels,
}

/// A block being read: its first bar, its bars' shapes, and its rows.
struct Block {
    line: usize,
    first: u32,
    shapes: Vec<BarShape>,
    lanes: Vec<Lane>,
    rows: Vec<Vec<Vec<u8>>>,
}

/// Where the reader is: in the header, or in a block with the header settled.
enum State {
    Header(Header),
    Body(Frame, Block),
}

/// What the header knows once the first block starts.
struct Frame {
    grid: DrumGrid,
    map: LaneMap,
    time: duet_core::TimeMap,
    step: u64,
}

fn words(line: &str) -> Vec<&str> {
    line.split_whitespace()
        .take_while(|w| !w.starts_with('#'))
        .collect()
}

fn u7(s: &str, what: &str) -> Result<u8> {
    s.parse::<u8>()
        .ok()
        .filter(|v| *v <= 127)
        .ok_or_else(|| Error::invalid(format!("'{s}' is not a {what} from 0 to 127")))
}

impl Header {
    fn apply(&mut self, line: &str) -> Result<()> {
        if self.conductor.apply_header_line(line)? {
            return Ok(());
        }
        let w = words(line);
        match w.as_slice() {
            ["grid", g] => self.grid = Some(g.parse()?),
            ["map", name] => {
                LaneMap::builtin(name)?;
                self.map = (*name).to_owned();
            }
            ["lane", name, key] => {
                if !valid_lane_name(name) {
                    return Err(Error::invalid(format!(
                        "'{name}' is not a lane name: use lowercase letters, digits and hyphens"
                    )));
                }
                let key = u7(key, "key")?;
                self.lanes.retain(|l| l.name != *name);
                self.lanes.push(Lane {
                    name: (*name).to_owned(),
                    key,
                });
            }
            ["channel", ch] => {
                self.channel = Some(
                    ch.parse()
                        .ok()
                        .filter(|c| (1..=16).contains(c))
                        .ok_or_else(|| {
                            Error::invalid(format!("'{ch}' is not a channel 1 to 16"))
                        })?,
                );
            }
            ["vel", pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
                for pair in pairs.chunks(2) {
                    let v = u7(pair[1], "velocity")?;
                    match pair[0] {
                        "g" => self.levels.ghost = v,
                        "x" => self.levels.hit = v,
                        "X" => self.levels.accent = v,
                        other => {
                            return Err(Error::invalid(format!(
                                "'{other}' is not a cell with a velocity; use g, x or X"
                            )));
                        }
                    }
                }
                self.levels.check()?;
            }
            [kw @ ("grid" | "map" | "lane" | "channel" | "vel"), ..] => {
                let shape = match *kw {
                    "grid" => "grid 1/16",
                    "map" => "map gm",
                    "lane" => "lane shaker 70",
                    "channel" => "channel 10",
                    _ => "vel g 40 x 90 X 120",
                };
                return Err(Error::invalid(format!("a {kw} line looks like '{shape}'")));
            }
            [kw, ..] => {
                return Err(Error::invalid(format!(
                    "'{kw}' is not a header line; expected title, tpq, tempo, time, key, end, grid, \
                     map, lane, channel, vel or a [bars a-b] block"
                )));
            }
            [] => {}
        }
        Ok(())
    }

    fn frame(self) -> Result<Frame> {
        let grid = self
            .grid
            .ok_or_else(|| Error::invalid("a drum grid needs a grid line, such as 'grid 1/16'"))?;
        let mut conductor = self.conductor;
        conductor.normalize()?;
        let map = LaneMap::builtin(&self.map)?.overlaid(&self.lanes)?;
        let mut out = DrumGrid::with_conductor(&conductor, grid);
        out.end = conductor.end;
        out.channel = self.channel.unwrap_or(map.channel());
        out.levels = self.levels;
        let time = out.time_map()?;
        let step = out.step_ticks()?;
        Ok(Frame {
            grid: out,
            map,
            time,
            step,
        })
    }
}

/// Parses `[bars a-b]` or `[bars a]`, from just after its `[`.
fn parse_block_header(inner: &str) -> Result<(u32, u32)> {
    let bad = || Error::invalid("a block starts with a line like [bars 1-4]");
    let inner = inner.trim().strip_suffix(']').ok_or_else(bad)?;
    let range = inner.trim().strip_prefix("bars").ok_or_else(bad)?.trim();
    let num = |s: &str| {
        s.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(bad)
    };
    let (a, b) = match range.split_once('-') {
        Some((a, b)) => (num(a)?, num(b)?),
        None => (num(range)?, num(range)?),
    };
    if b < a {
        return Err(Error::invalid(format!("[bars {a}-{b}] runs backwards")));
    }
    Ok((a, b))
}

impl Frame {
    fn open(&self, line: usize, first: u32, last: u32) -> Result<Block> {
        let expected = self.grid.bars.len() as u32 + 1;
        if first != expected {
            return Err(Error::invalid(format!(
                "this block starts at bar {first}, but bar {expected} comes next; blocks cover \
                 every bar in order, and an empty bar is written as rests"
            )));
        }
        let shapes = (first..=last)
            .map(|b| self.grid.bar_shape(&self.time, self.step, b))
            .collect::<Result<Vec<_>>>()?;
        Ok(Block {
            line,
            first,
            shapes,
            lanes: Vec::new(),
            rows: Vec::new(),
        })
    }

    fn row(&self, block: &mut Block, line: &str) -> Result<()> {
        let line = line.split('#').next().unwrap_or("");
        let mut parts = line.split('|');
        let name = parts.next().unwrap_or("").trim();
        let lane = self.map.lookup(name).ok_or_else(|| {
            Error::invalid(format!(
                "'{name}' is not a lane in the {} map; add a line like 'lane {name} 70' to the header",
                self.map.name()
            ))
        })?;
        if let Some(seen) = block.lanes.iter().find(|l| l.name == lane.name) {
            return Err(Error::invalid(format!(
                "{} is listed twice in this block",
                seen.name
            )));
        }
        let mut bars: Vec<&str> = parts.collect();
        if bars.last().map(|s| s.trim().is_empty()) != Some(true) {
            return Err(Error::invalid(format!(
                "{}: a row ends with | after its last bar",
                lane.name
            )));
        }
        bars.pop();
        let want = block.shapes.len();
        if bars.len() != want {
            let last = block.first + want as u32 - 1;
            return Err(Error::invalid(format!(
                "{} has {} bars, but the block is bars {}-{last} ({want} bars)",
                lane.name,
                bars.len(),
                block.first
            )));
        }
        let mut row = Vec::with_capacity(want);
        for (i, (cells, shape)) in bars.iter().zip(&block.shapes).enumerate() {
            let bar = block.first + i as u32;
            let cells: Vec<&str> = cells.split_whitespace().collect();
            if cells.len() != shape.steps as usize {
                return Err(Error::invalid(format!(
                    "{}, bar {bar} has {} steps; {} at {} needs {}",
                    lane.name,
                    cells.len(),
                    shape.sig,
                    self.grid.grid,
                    shape.steps
                )));
            }
            let vel = cells
                .iter()
                .enumerate()
                .map(|(s, c)| {
                    self.grid.levels.vel(c).ok_or_else(|| {
                        Error::invalid(format!(
                            "{}, bar {bar}, step {}: '{c}' is not a cell; cells are . g x X, \
                             separated by spaces",
                            lane.name,
                            s + 1
                        ))
                    })
                })
                .collect::<Result<Vec<u8>>>()?;
            row.push(vel);
        }
        block.lanes.push(Lane {
            name: lane.name.clone(),
            key: lane.key,
        });
        block.rows.push(row);
        Ok(())
    }

    fn close(&mut self, block: Block) -> Result<()> {
        let names = |lanes: &[Lane]| {
            lanes
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if block.first == 1 {
            self.grid.lanes = block.lanes;
        } else if block.lanes != self.grid.lanes {
            return Err(Error::parse(
                block.line,
                format!(
                    "this block lists {}, but every block lists the same lanes as the first, in \
                     the same order: {}",
                    names(&block.lanes),
                    names(&self.grid.lanes)
                ),
            ));
        }
        for (i, shape) in block.shapes.iter().enumerate() {
            self.grid.bars.push(Bar {
                bar: block.first + i as u32,
                steps: shape.steps,
                vel: block.rows.iter().map(|r| r[i].clone()).collect(),
            });
        }
        Ok(())
    }
}

impl DrumGrid {
    /// Parses the text form. The conductor comes back normalized, as
    /// [`Song::from_text`] gives it; cells come back as the velocities they stand for.
    pub fn from_text(text: &str) -> Result<DrumGrid> {
        let mut state = State::Header(Header {
            conductor: Song::default(),
            grid: None,
            map: "gm".to_owned(),
            lanes: Vec::new(),
            channel: None,
            levels: Levels::default(),
        });

        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let at = |e: Error| match e {
                Error::Invalid(msg) => Error::parse(line_no, msg),
                other => other,
            };
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(inner) = line.strip_prefix('[') {
                let (first, last) = parse_block_header(inner).map_err(at)?;
                let frame = match state {
                    State::Header(h) => h.frame().map_err(at)?,
                    State::Body(mut f, done) => {
                        f.close(done)?;
                        f
                    }
                };
                let block = frame.open(line_no, first, last).map_err(at)?;
                state = State::Body(frame, block);
                continue;
            }
            match &mut state {
                State::Header(h) => h.apply(line).map_err(at)?,
                State::Body(f, b) if line.contains('|') => f.row(b, line).map_err(at)?,
                State::Body(..) => {
                    return Err(Error::parse(
                        line_no,
                        format!(
                            "'{}' is not a row like 'kick | X . . . |'; header lines go \
                             before the first [bars] block",
                            words(line).first().unwrap_or(&"")
                        ),
                    ));
                }
            }
        }

        let frame = match state {
            State::Header(h) => h.frame()?,
            State::Body(mut f, done) => {
                f.close(done)?;
                f
            }
        };
        frame.grid.check()?;
        Ok(frame.grid)
    }

    /// Writes the text form, naming rows from the built-in `gm` map; a lane that map does
    /// not name, or names on another key, gets a `lane` line.
    ///
    /// Fails, naming the lane, bar and step, when a hit's velocity is not one of the
    /// grid's [levels](DrumGrid::levels): the text cannot hold it. Write the exact form
    /// (JSON or TOML) instead, or snap the velocities first
    /// ([`DrumGrid::snapped_to_levels`]).
    pub fn to_text(&self) -> Result<String> {
        self.check()?;
        if let Some((lane, bar, step, vel)) = self.first_off_level() {
            let l = self.levels;
            return Err(Error::invalid(format!(
                "{lane}, bar {bar}, step {step}: velocity {vel} is not a level of the text form \
                 (g {}, x {}, X {}); write the exact form (.drums.json or .drums.toml), or \
                 snap velocities to the levels first",
                l.ghost, l.hit, l.accent
            )));
        }
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = self.write_text(&mut out);
        Ok(out)
    }

    fn write_text(&self, out: &mut String) -> std::fmt::Result {
        let gm = LaneMap::gm();
        writeln!(out, "{HEADER_COMMENT}")?;
        out.push_str(&self.conductor().header_text());
        writeln!(out, "grid {}", self.grid)?;
        writeln!(out, "map {}", gm.name())?;
        if self.channel != gm.channel() {
            writeln!(out, "channel {}", self.channel)?;
        }
        if self.levels != Levels::default() {
            let l = self.levels;
            writeln!(out, "vel g {} x {} X {}", l.ghost, l.hit, l.accent)?;
        }
        for lane in &self.lanes {
            let named = gm.lanes().any(|l| l.name == lane.name && l.key == lane.key);
            if !named {
                writeln!(out, "lane {} {}", lane.name, lane.key)?;
            }
        }

        // Shapes cannot fail here: to_text checked the grid.
        let (Ok(time), Ok(step)) = (self.time_map(), self.step_ticks()) else {
            return Ok(());
        };
        let shapes: Vec<BarShape> = self
            .bars
            .iter()
            .filter_map(|b| self.bar_shape(&time, step, b.bar).ok())
            .collect();
        let width = self.lanes.iter().map(|l| l.name.len()).max().unwrap_or(1);

        let mut start = 0;
        while start < self.bars.len() {
            let mut end = start + 1;
            let mut steps = shapes[start].steps;
            while end < self.bars.len()
                && end - start < BLOCK_BARS
                && steps + shapes[end].steps <= BLOCK_STEPS
                && shapes[end].sig == shapes[start].sig
                && shapes[end].steps == shapes[start].steps
            {
                steps += shapes[end].steps;
                end += 1;
            }
            writeln!(out)?;
            if end - start == 1 {
                writeln!(out, "[bars {}]", start + 1)?;
            } else {
                writeln!(out, "[bars {}-{end}]", start + 1)?;
            }
            let counts: Vec<String> = shapes[start..end]
                .iter()
                .map(|s| self.count_row(s, step))
                .collect();
            writeln!(out, "{:<width$}   {}", "#", counts.join("   "))?;
            for (i, lane) in self.lanes.iter().enumerate() {
                write!(out, "{:<width$} |", lane.name)?;
                for bar in &self.bars[start..end] {
                    for v in &bar.vel[i] {
                        write!(out, " {}", self.levels.cell(*v).unwrap_or('?'))?;
                    }
                    write!(out, " |")?;
                }
                writeln!(out)?;
            }
            start = end;
        }
        Ok(())
    }

    /// The beat count over one bar: each step that starts a beat shows the beat's number
    /// (its last digit, past 9), and every other step shows `.`.
    fn count_row(&self, shape: &BarShape, step: u64) -> String {
        let beat = u64::from(self.ticks_per_quarter) * 4 / u64::from(shape.sig.den);
        (0..u64::from(shape.steps))
            .map(|s| {
                let offset = s * step;
                if offset.is_multiple_of(beat) {
                    char::from(b'0' + ((offset / beat + 1) % 10) as u8)
                } else {
                    '.'
                }
            })
            .map(String::from)
            .collect::<Vec<_>>()
            .join(" ")
    }
}
