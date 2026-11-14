//! Quantizing: snapping a played take's notes to a grid, the step from a performance to a
//! score.
//!
//! Quantizing is always asked for. [`Song::quantized`] returns a new song and leaves the
//! one it was called on as it was; no read, write or conversion quantizes on its own.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::model::Song;
use crate::time::{Position, TimeMap, beats_from_ticks, string_serde, ticks_from_beats};

/// A quantize grid: one step is `1/den` of a whole note, or, for a triplet grid, two
/// thirds of that. Written `1/16` for sixteenths and `1/8t` for triplet eighths, in text
/// and in JSON and TOML alike.
///
/// The grid restarts at every bar line, so bar lines are always on it. In a bar whose
/// length is not a whole number of steps, such as one cut short by a time signature
/// change, the last step before the bar line is short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grid {
    den: u32,
    triplet: bool,
}

impl Grid {
    /// A grid of `1/den` notes: `Grid::new(16, false)` is sixteenths, and
    /// `Grid::new(8, true)` is triplet eighths.
    pub fn new(den: u32, triplet: bool) -> Result<Self> {
        if den == 0 {
            return Err(Error::invalid("a grid of 1/0 has no steps"));
        }
        Ok(Grid { den, triplet })
    }

    /// The length of one step in ticks, or an error naming the grid and resolution when
    /// the step is not a whole number of ticks.
    pub fn step_ticks(self, ticks_per_quarter: u16) -> Result<u64> {
        let whole = u64::from(ticks_per_quarter) * 4;
        let (num, den) = if self.triplet {
            (whole * 2, u64::from(self.den) * 3)
        } else {
            (whole, u64::from(self.den))
        };
        if num == 0 || !num.is_multiple_of(den) {
            return Err(Error::invalid(format!(
                "a {self} grid cannot divide {ticks_per_quarter} ticks per quarter into whole \
                 ticks; choose a coarser grid"
            )));
        }
        Ok(num / den)
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "1/{}{}", self.den, if self.triplet { "t" } else { "" })
    }
}

impl FromStr for Grid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bad = || Error::invalid(format!("'{s}' is not a grid like 1/16 or 1/8t"));
        let rest = s.trim().strip_prefix("1/").ok_or_else(bad)?;
        let (digits, triplet) = match rest.strip_suffix('t') {
            Some(d) => (d, true),
            None => (rest, false),
        };
        let den: u32 = digits.parse().map_err(|_| bad())?;
        Grid::new(den, triplet)
    }
}

string_serde!(Grid);

/// Grid points near a tick, read against the bar the tick falls in.
struct Snapper<'a> {
    map: &'a TimeMap,
    step: u64,
}

impl Snapper<'_> {
    /// The start and end ticks of the bar holding `tick`.
    fn bar_of(&self, tick: u64) -> Result<(u64, u64)> {
        let bar = self.map.position(tick).bar;
        Ok((
            self.map.tick(Position::bar(bar))?,
            self.map.tick(Position::bar(bar + 1))?,
        ))
    }

    /// The nearest grid point. A tick exactly halfway between two points goes to the
    /// later one.
    fn nearest(&self, tick: u64) -> Result<u64> {
        let (start, end) = self.bar_of(tick)?;
        let down = start + (tick - start) / self.step * self.step;
        let up = (down + self.step).min(end);
        Ok(if tick - down < up - tick { down } else { up })
    }

    /// The next grid point after `tick`, which is itself on the grid.
    fn next(&self, tick: u64) -> Result<u64> {
        let (start, end) = self.bar_of(tick)?;
        Ok((start + ((tick - start) / self.step + 1) * self.step).min(end))
    }
}

impl Song {
    /// A copy of the song with every note's start and end moved to the nearest point of
    /// `grid`. This song is not changed.
    ///
    /// - Pitch, velocity, track and the order of each track's notes are kept.
    /// - A note that had length but would collapse to zero length is given one step instead,
    ///   so no played note disappears. A note that was already zero length stays zero length.
    /// - Controls, tempo, key and time signature changes are not moved.
    /// - Notes of the same pitch that quantizing puts one inside another are kept as they
    ///   come out; writing them to MIDI refuses them by name, as it does any nested note.
    ///
    /// Fails, naming the grid, when one step is not a whole number of this song's ticks,
    /// and, naming the note, when the song names an [end](Song::end) and a note quantizing
    /// would end past it. The end is never moved and the note is never cut to fit.
    pub fn quantized(&self, grid: Grid) -> Result<Song> {
        let tpq = self.ticks_per_quarter;
        let map = self.time_map()?;
        let snap = Snapper {
            map: &map,
            step: grid.step_ticks(tpq)?,
        };
        let mut out = self.clone();
        for track in &mut out.tracks {
            for note in &mut track.notes {
                let start = map.tick(note.at)?;
                let len = ticks_from_beats(note.beats, tpq)?;
                let q_start = snap.nearest(start)?;
                let mut q_end = snap.nearest(start + len)?;
                if len > 0 && q_end <= q_start {
                    q_end = snap.next(q_start)?;
                }
                note.at = map.position(q_start);
                note.beats = beats_from_ticks(q_end - q_start, tpq);
            }
        }
        out.check_end()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_parse_and_print() {
        for s in ["1/4", "1/16", "1/8t", "1/32t"] {
            assert_eq!(s.parse::<Grid>().unwrap().to_string(), s);
        }
        for bad in ["", "1/", "1/0", "2/16", "1/x", "1/16tt", "16"] {
            assert!(bad.parse::<Grid>().is_err(), "{bad}");
        }
    }

    #[test]
    fn step_ticks_or_a_named_refusal() {
        let g = |s: &str| s.parse::<Grid>().unwrap();
        assert_eq!(g("1/16").step_ticks(480).unwrap(), 120);
        assert_eq!(g("1/8t").step_ticks(480).unwrap(), 160);
        assert_eq!(g("1/4").step_ticks(96).unwrap(), 96);
        let err = g("1/7").step_ticks(480).unwrap_err().to_string();
        assert!(
            err.contains("1/7") && err.contains("480 ticks per quarter"),
            "{err}"
        );
        assert!(g("1/5").step_ticks(96).is_err());
        assert!(g("1/512t").step_ticks(96).is_err());
    }
}
