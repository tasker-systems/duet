//! Converting between a drum grid and a [`Song`], the hub every format passes through.

use duet_core::time::beats_from_ticks;
use duet_core::{Grid, Note, Pitch, Song, Track};

use crate::error::{Error, Result};
use crate::grid::{Bar, DrumGrid, Lane};
use crate::lanes::LaneMap;

/// The name of the track a drum grid becomes.
pub const DRUM_TRACK: &str = "Drums";

impl DrumGrid {
    /// The grid as a song: its conductor, and one track named `Drums` on the grid's
    /// channel with no program. Each hit becomes a note on its lane's key, starting on
    /// its step, one step long (drum sounds are one-shots, and General MIDI kits ignore
    /// note-off), with the hit's velocity. Notes are in time order, and top row first
    /// at the same step.
    pub fn to_song(&self) -> Result<Song> {
        self.check()?;
        let map = self.time_map()?;
        let step = self.step_ticks()?;
        let beats = beats_from_ticks(step, self.ticks_per_quarter);
        let mut track = Track::new(DRUM_TRACK, self.channel);
        for bar in &self.bars {
            let shape = self.bar_shape(&map, step, bar.bar)?;
            for s in 0..bar.steps as usize {
                for (lane, row) in self.lanes.iter().zip(&bar.vel) {
                    if row[s] > 0 {
                        track.notes.push(Note::new(
                            map.position(shape.start + s as u64 * step),
                            Pitch::new(lane.key)?,
                            beats,
                            row[s],
                        ));
                    }
                }
            }
        }
        let mut song = self.conductor();
        song.tracks.push(track);
        song.normalize()?;
        Ok(song)
    }

    /// The drum track of a song as a grid of `grid` steps, with lanes from `lanes`.
    ///
    /// The drum track is the one named `track`, or else the only track on the map's
    /// channel. The grid takes the song's conductor and that track's channel. Its lanes
    /// are the map's lanes the track plays, in the map's row order. It runs to `bars`
    /// bars, or to the last bar with a hit.
    ///
    /// Only note starts and velocities are kept: note lengths, the track's controls and
    /// the song's other tracks are not part of a drum grid.
    ///
    /// Nothing is moved or changed to fit. It fails, naming what does not fit, when a
    /// note starts off the grid (quantize first), plays a key no lane maps, lands on the
    /// same lane and step as another, or falls past `bars`, or when a bar is not a
    /// whole number of steps.
    pub fn from_song(
        song: &Song,
        grid: Grid,
        lanes: &LaneMap,
        track: Option<&str>,
        bars: Option<u32>,
    ) -> Result<DrumGrid> {
        let drums = drum_track(song, lanes, track)?;
        let mut out = DrumGrid::with_conductor(song, grid);
        out.channel = drums.channel;
        let map = out.time_map()?;
        let step = out.step_ticks()?;

        // (lane, bar, step, vel) for every note, checked against the grid and the map.
        let mut hits = Vec::with_capacity(drums.notes.len());
        for note in &drums.notes {
            let tick = map.tick(note.at)?;
            let at = map.position(tick);
            let shape = out.bar_shape(&map, step, at.bar)?;
            let offset = tick - shape.start;
            if !offset.is_multiple_of(step) {
                return Err(Error::invalid(format!(
                    "the note on key {} at {at} is not on the {grid} grid; quantize it first \
                     (duet song quantize --grid {grid})",
                    note.pitch.midi()
                )));
            }
            let lane = lanes.by_key(note.pitch.midi()).ok_or_else(|| {
                Error::invalid(format!(
                    "the note on key {} at {at} plays no lane in the {} lane map",
                    note.pitch.midi(),
                    lanes.name()
                ))
            })?;
            hits.push((lane, at.bar, (offset / step) as usize, note.vel, at));
        }

        let last = hits.iter().map(|h| h.1).max().unwrap_or(0);
        let count = match bars {
            Some(n) if n < last => {
                return Err(Error::invalid(format!(
                    "there is a hit in bar {last}, past the {n} bars asked for"
                )));
            }
            Some(n) => n,
            None => last,
        };

        let mut used: Vec<&crate::lanes::LaneDef> = Vec::new();
        for (lane, ..) in &hits {
            if !used.iter().any(|u| u.name == lane.name) {
                used.push(lane);
            }
        }
        used.sort_by_key(|l| lanes.rank(&l.name));
        out.lanes = used
            .iter()
            .map(|l| Lane {
                name: l.name.clone(),
                key: l.key,
            })
            .collect();

        for number in 1..=count {
            let shape = out.bar_shape(&map, step, number)?;
            out.bars.push(Bar {
                bar: number,
                steps: shape.steps,
                vel: vec![vec![0; shape.steps as usize]; out.lanes.len()],
            });
        }
        for (lane, bar, s, vel, at) in hits {
            let row = out
                .lanes
                .iter()
                .position(|l| l.name == lane.name)
                .expect("every hit's lane is among the grid's lanes");
            let cell = &mut out.bars[bar as usize - 1].vel[row][s];
            if *cell != 0 {
                return Err(Error::invalid(format!(
                    "two hits on {} at {at} (bar {bar}, step {}); a cell holds one hit",
                    lane.name,
                    s + 1
                )));
            }
            *cell = vel;
        }
        out.check()?;
        Ok(out)
    }
}

/// The song's drum track: the one named, or the only one on the map's channel.
fn drum_track<'a>(song: &'a Song, lanes: &LaneMap, name: Option<&str>) -> Result<&'a Track> {
    if let Some(name) = name {
        return song
            .tracks
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::invalid(format!("the song has no track named \"{name}\"")));
    }
    let on_channel: Vec<&Track> = song
        .tracks
        .iter()
        .filter(|t| t.channel == lanes.channel())
        .collect();
    match on_channel.as_slice() {
        [one] => Ok(one),
        [] => Err(Error::invalid(format!(
            "the song has no track on channel {}, the drum channel; name the drum track",
            lanes.channel()
        ))),
        several => Err(Error::invalid(format!(
            "the song has {} tracks on channel {} ({}); name the one to read",
            several.len(),
            lanes.channel(),
            several
                .iter()
                .map(|t| format!("\"{}\"", t.name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}
