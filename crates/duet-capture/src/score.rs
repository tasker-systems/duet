//! The score: a captured take quantized so that no two notes, or two hits on one lane, share a
//! grid step.
//!
//! A quick run or a flam can put two notes on one step once quantized, which neither score
//! writer can hold. So the grid is refined first, halving its step down to a 64th
//! ([`quantize_apart`]), and only what that finest grid still cannot part is merged into one
//! note ([`merge_collisions`]). The `.notes` is never quantized and keeps every note.

use duet_core::{Grid, Note, Position, Song};

use crate::assemble::Kind;
use crate::error::Result;

/// The finest grid a score is refined to is a 64th: this many steps to a whole note.
const FINEST_STEPS_PER_WHOLE: u64 = 64;

/// A take quantized to the grid it was refined to.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Quantized {
    /// The grid the song is quantized to: the one asked for, or finer.
    pub grid: Grid,
    /// The song quantized to `grid`, with every note still in it.
    pub song: Song,
}

/// Notes or hits on one step that no grid parted, merged into the one kept.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Merge {
    /// The step they share.
    pub at: Position,
    /// How many were merged into one.
    pub count: usize,
    /// The one kept: a melody's longest note, or the strongest hit.
    pub kept: Note,
}

/// `song` quantized to `grid`, or, when two of its notes would share a step there (two hits on
/// one lane, for a rhythm), to the grid with half the step, and so on down to a 64th. A grid
/// already as fine as a 64th is not refined. The finest grid tried is returned even if notes
/// still share a step on it; [`merge_collisions`] merges those.
///
/// Fails when a grid cannot divide the song's ticks per quarter into whole ticks.
pub fn quantize_apart(song: &Song, grid: Grid, kind: &Kind) -> Result<Quantized> {
    let tpq = song.ticks_per_quarter;
    let finest = 4 * u64::from(tpq) / FINEST_STEPS_PER_WHOLE;
    let mut grid = grid;
    loop {
        let quantized = song.quantized(grid)?;
        let step = grid.step_ticks(tpq)?;
        let finer = grid
            .halved()
            .filter(|g| step > finest && g.step_ticks(tpq).is_ok());
        match finer {
            Some(finer) if collides(&quantized, kind)? => grid = finer,
            _ => {
                return Ok(Quantized {
                    grid,
                    song: quantized,
                });
            }
        }
    }
}

/// Merges, in each track of a quantized `song`, the notes that share a step (for a rhythm, the
/// hits on one lane that do) into one: a melody keeps its longest note, and a rhythm its
/// strongest hit, the first of them on a tie. Returns every merge, in the order of their steps.
///
/// Fails when a note's position is not in the song, which a quantized song cannot have.
pub fn merge_collisions(song: &mut Song, kind: &Kind) -> Result<Vec<Merge>> {
    let map = song.time_map()?;
    let mut merges: Vec<(u64, Merge)> = Vec::new();
    for track in &mut song.tracks {
        // Each step's kept note, how many share it, and the step.
        let mut kept: Vec<(Note, usize, (u64, Option<u8>))> = Vec::with_capacity(track.notes.len());
        for note in &track.notes {
            let slot = (map.tick(note.at)?, lane_of(note, kind));
            match kept.iter_mut().find(|k| k.2 == slot) {
                Some((held, count, _)) => {
                    *count += 1;
                    if wins(note, held, kind) {
                        *held = note.clone();
                    }
                }
                None => kept.push((note.clone(), 1, slot)),
            }
        }
        merges.extend(kept.iter().filter(|(_, count, _)| *count > 1).map(
            |(note, count, (tick, _))| {
                (
                    *tick,
                    Merge {
                        at: note.at,
                        count: *count,
                        kept: note.clone(),
                    },
                )
            },
        ));
        track.notes = kept.into_iter().map(|(note, ..)| note).collect();
    }
    merges.sort_by_key(|(tick, _)| *tick);
    Ok(merges.into_iter().map(|(_, merge)| merge).collect())
}

/// Whether two notes of a track in `song` share a step: any two in a melody, two on one lane
/// in a rhythm.
fn collides(song: &Song, kind: &Kind) -> Result<bool> {
    let map = song.time_map()?;
    for track in &song.tracks {
        let mut seen = Vec::with_capacity(track.notes.len());
        for note in &track.notes {
            let slot = (map.tick(note.at)?, lane_of(note, kind));
            if seen.contains(&slot) {
                return Ok(true);
            }
            seen.push(slot);
        }
    }
    Ok(false)
}

/// What besides its step a note shares a step by: nothing in a melody, its key (its lane) in a
/// rhythm.
fn lane_of(note: &Note, kind: &Kind) -> Option<u8> {
    match kind {
        Kind::Melody => None,
        Kind::Rhythm { .. } => Some(note.pitch.midi()),
    }
}

/// Whether `note` is kept over `held` on their shared step: a longer note in a melody, a
/// stronger hit in a rhythm.
fn wins(note: &Note, held: &Note, kind: &Kind) -> bool {
    match kind {
        Kind::Melody => note.beats > held.beats,
        Kind::Rhythm { .. } => note.vel > held.vel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duet_core::{Pitch, Track};
    use duet_drums::LaneMap;

    fn grid(s: &str) -> Grid {
        s.parse().unwrap()
    }

    fn rhythm() -> Kind {
        Kind::Rhythm {
            lanes: LaneMap::gm(),
            asked: None,
        }
    }

    /// A song of one track with a note at each `(tick, midi, beats, vel)`.
    fn song(notes: &[(u64, u8, f64, u8)]) -> Song {
        let mut song = Song::default();
        let map = song.time_map().unwrap();
        let mut track = Track::new("t", 1);
        for &(tick, midi, beats, vel) in notes {
            track.notes.push(Note::new(
                map.position(tick),
                Pitch::new(midi).unwrap(),
                beats,
                vel,
            ));
        }
        song.tracks.push(track);
        song
    }

    #[test]
    fn a_grid_nothing_collides_on_is_kept() {
        let take = song(&[(0, 60, 1.0, 80), (480, 62, 1.0, 80)]);
        let q = quantize_apart(&take, grid("1/16"), &Kind::Melody).unwrap();
        assert_eq!(q.grid, grid("1/16"));
    }

    #[test]
    fn the_grid_halves_until_nothing_collides() {
        // 15 ticks apart: a sixteenth's (120) or a 1/32's (60) step holds both; a 1/64's (30)
        // does not, nor a 1/64 triplet's (20).
        let take = song(&[(0, 60, 0.1, 80), (15, 62, 0.1, 80)]);
        let q = quantize_apart(&take, grid("1/16"), &Kind::Melody).unwrap();
        assert_eq!(q.grid, grid("1/64"));
        assert_eq!(q.song.tracks[0].notes.len(), 2);
        // A triplet grid halves as a triplet grid.
        let q = quantize_apart(&take, grid("1/8t"), &Kind::Melody).unwrap();
        assert_eq!(q.grid, grid("1/64t"));
    }

    #[test]
    fn the_finest_grid_is_a_64th() {
        let take = song(&[(0, 60, 0.1, 80), (10, 62, 0.1, 80)]);
        let q = quantize_apart(&take, grid("1/16"), &Kind::Melody).unwrap();
        assert_eq!(q.grid, grid("1/64"));
        // A grid finer than that is used as it is.
        let q = quantize_apart(&take, grid("1/128"), &Kind::Melody).unwrap();
        assert_eq!(q.grid, grid("1/128"));
    }

    #[test]
    fn hits_on_different_lanes_do_not_collide() {
        let take = song(&[(0, 36, 0.25, 80), (10, 42, 0.25, 80)]);
        let q = quantize_apart(&take, grid("1/16"), &rhythm()).unwrap();
        assert_eq!(q.grid, grid("1/16"));
    }

    #[test]
    fn a_melody_merge_keeps_the_longer_note() {
        let mut take = song(&[(0, 60, 0.1, 80), (0, 64, 0.5, 40), (480, 62, 1.0, 80)]);
        let merges = merge_collisions(&mut take, &Kind::Melody).unwrap();
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].count, 2);
        assert_eq!(merges[0].kept.pitch.midi(), 64);
        let left: Vec<u8> = take.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch.midi())
            .collect();
        assert_eq!(left, [64, 62]);
    }

    #[test]
    fn a_rhythm_merge_keeps_the_stronger_hit_and_only_on_one_lane() {
        let mut take = song(&[(0, 38, 0.25, 50), (0, 38, 0.25, 110), (0, 36, 0.25, 80)]);
        let merges = merge_collisions(&mut take, &rhythm()).unwrap();
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].kept.vel, 110);
        assert_eq!(take.tracks[0].notes.len(), 2);
    }
}
