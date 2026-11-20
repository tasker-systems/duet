//! `Part` → ABC.
//!
//! The rules are the design's §4. Nothing is ever quantized or moved: a note that ABC
//! cannot write where it is, or a velocity that has no dynamic, is refused by name
//! ([`Error::Unwritable`]), and the part is left as it was. The output is canonical:
//! writing what it reads back gives the same text.

use std::fmt::Write as _;

use duet_core::time::ticks_from_beats;
use duet_core::{Key, Mode, Part, Position, TimeSig, Track};

use crate::error::{Error, Result};
use crate::ratio::Ratio;
use crate::read::{
    DEFAULT_VELOCITY, DYNAMICS, beats_per_beam_group, default_channel, key_alterations,
    pickup_meter,
};

/// What to do with a velocity that is not one of the dynamics' levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Dynamics {
    /// Refuse it, naming the note. Nothing is snapped unasked.
    #[default]
    Refuse,
    /// Snap it to the nearest level, and say so in a comment in the ABC.
    Nearest,
}

/// How to write a part as ABC.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct WriteOptions {
    /// The unit note length (`L:`), as a fraction of a whole note. Without it, the
    /// standard's default for the meter: 1/16 below 3/4, 1/8 otherwise.
    pub unit: Option<(u32, u32)>,
    pub dynamics: Dynamics,
}

impl WriteOptions {
    /// These options with `L:` set to `num/den` of a whole note, such as 1/4.
    pub fn with_unit(mut self, num: u32, den: u32) -> Self {
        self.unit = Some((num, den));
        self
    }

    /// These options with `dynamics` set.
    pub fn with_dynamics(mut self, dynamics: Dynamics) -> Self {
        self.dynamics = dynamics;
        self
    }
}

const QUANTIZE_HINT: &str = "run duet part quantize --grid 1/16 first";

/// The most bars a written tune holds. Every bar is laid out in memory, so a part that
/// ends far away (a stray end of track, say) is refused rather than written.
const MAX_BARS: u32 = 100_000;

struct BarSpan {
    start: u64,
    len: u64,
    sig: TimeSig,
}

/// A stretch of one track within one bar: notes sounding together, or a rest.
#[derive(Debug, Clone)]
struct Piece {
    start: u64,
    end: u64,
    /// Index into the track's chord groups; `None` for a rest.
    group: Option<usize>,
    /// The notes go on past this piece, tied into the next.
    tie: bool,
}

/// Notes that start and end together, in the order the track holds them.
struct Group {
    start: u64,
    end: u64,
    notes: Vec<usize>,
    vel: u8,
}

fn unwritable(at: Position, track: Option<&str>, why: String, hint: &str) -> Error {
    Error::Unwritable {
        at,
        track: track.map(str::to_owned),
        why,
        hint: hint.to_owned(),
    }
}

fn whole_bpm(bpm: f64, at: Position) -> Result<u64> {
    if bpm.is_finite() && bpm > 0.0 && (bpm - bpm.round()).abs() < 1e-9 {
        Ok(bpm.round() as u64)
    } else {
        Err(unwritable(
            at,
            None,
            format!("the tempo {bpm} is not a whole number of bpm"),
            "ABC writes Q:1/4=<n>, and rounding would change the tempo; set a whole bpm first",
        ))
    }
}

fn key_text(key: Key) -> String {
    match key.mode {
        Mode::Major => key.tonic().to_owned(),
        Mode::Minor => format!("{}m", key.tonic()),
        mode => format!("{} {}", key.tonic(), mode.name()),
    }
}

/// The length as a multiple of the unit: ``, `2`, `/`, `/4`, `3/2`.
fn length_text(ticks: u64, unit_ticks: u64) -> String {
    let r = Ratio::new(ticks, unit_ticks);
    match (r.num(), r.den()) {
        (1, 1) => String::new(),
        (n, 1) => n.to_string(),
        (1, 2) => "/".to_owned(),
        (1, d) => format!("/{d}"),
        (n, d) => format!("{n}/{d}"),
    }
}

fn note_name(pitch: duet_core::Pitch) -> String {
    let octave = pitch.octave();
    let upper = ['C', 'D', 'E', 'F', 'G', 'A', 'B'][usize::from(pitch.letter())];
    if octave >= 5 {
        let mut s = upper.to_ascii_lowercase().to_string();
        s.push_str(&"'".repeat((octave - 5) as usize));
        s
    } else {
        let mut s = upper.to_string();
        s.push_str(&",".repeat((4 - octave) as usize));
        s
    }
}

/// An inline field due at a tick: its text, and the key it sets, if it is `[K:]`.
type Change = (u64, String, Option<Key>);

struct TrackWriter<'a> {
    track: &'a Track,
    groups: Vec<Group>,
    unit_ticks: u64,
    whole_ticks: u64,
    vel: u8,
    key: Key,
    bar_acc: [Option<i8>; 7],
    map: &'a duet_core::TimeMap,
}

impl TrackWriter<'_> {
    fn at(&self, tick: u64) -> Position {
        self.map.position(tick)
    }

    fn binary(&self, ticks: u64) -> bool {
        (ticks * 64).is_multiple_of(self.whole_ticks)
    }

    fn off_grid(&self, piece: &Piece) -> Error {
        // A rest off the grid is so because of the note it leads to: name that note.
        let piece = &match piece.group {
            None => self
                .groups
                .iter()
                .position(|g| g.start == piece.end)
                .map_or_else(
                    || piece.clone(),
                    |g| Piece {
                        start: piece.end,
                        end: self.groups[g].end,
                        group: Some(g),
                        tie: false,
                    },
                ),
            Some(_) => piece.clone(),
        };
        let what = match piece.group {
            Some(g) => {
                let n = &self.track.notes[self.groups[g].notes[0]];
                format!("the {} at {}", n.pitch, n.at)
            }
            None => format!("the rest at {}", self.at(piece.start)),
        };
        unwritable(
            self.at(piece.start),
            Some(&self.track.name),
            format!(
                "{what} lies off the 1/64-note grid, and not in a triplet group (three equal \
                 slots filling whole beats, each note one or two slots long)"
            ),
            QUANTIZE_HINT,
        )
    }

    /// The group's notes as ABC, with any dynamic and accidental they need.
    fn group_text(&mut self, g: usize, written: u64, tie: bool) -> String {
        let mut out = String::new();
        let vel = self.groups[g].vel;
        if vel != self.vel {
            let name = DYNAMICS
                .iter()
                .find(|(_, v)| *v == vel)
                .map_or("f", |(d, _)| d);
            let _ = write!(out, "!{name}!");
            self.vel = vel;
        }
        let notes = self.groups[g].notes.clone();
        let chord = notes.len() > 1;
        if chord {
            out.push('[');
        }
        for i in notes {
            // A note is written as the part spells it: Cb5 is `c` under K:Ebm and `_c`
            // under K:C, and B4 is `=B` under K:Ebm.
            let pitch = self.track.notes[i].pitch;
            let (letter, alter) = (pitch.letter(), pitch.alteration());
            let in_force = self.bar_acc[usize::from(letter)]
                .unwrap_or(key_alterations(self.key)[usize::from(letter)]);
            if alter != in_force {
                out.push_str(match alter {
                    -2 => "__",
                    -1 => "_",
                    0 => "=",
                    1 => "^",
                    _ => "^^",
                });
                self.bar_acc[usize::from(letter)] = Some(alter);
            }
            out.push_str(&note_name(pitch));
        }
        if chord {
            out.push(']');
        }
        out.push_str(&length_text(written, self.unit_ticks));
        if tie {
            out.push('-');
        }
        out
    }

    fn rest_text(&self, written: u64) -> String {
        format!("z{}", length_text(written, self.unit_ticks))
    }

    /// The inline fields due at `tick`, written, with any key they set taking effect.
    fn changes_at(&mut self, out: &mut String, changes: &[Change], tick: u64) {
        for (_, text, key) in changes.iter().filter(|(t, _, _)| *t == tick) {
            out.push_str(text);
            if let Some(key) = key {
                self.key = *key;
            }
        }
    }

    /// Writes one bar's pieces. `changes` are the inline fields due at ticks in the bar.
    fn bar(&mut self, bar: &BarSpan, pieces: &mut [Piece], changes: &[Change]) -> Result<String> {
        self.bar_acc = [None; 7];
        let beat = self.whole_ticks / u64::from(bar.sig.den);
        let group_beat = beats_per_beam_group(bar.sig) * beat;
        let bar_end = bar.start + bar.len;
        let mut out = String::new();
        let mut i = 0;
        while i < pieces.len() {
            let p = pieces[i].clone();
            let off = p.start - bar.start;
            let len = p.end - p.start;
            let end;
            if self.binary(off) && self.binary(len) {
                self.changes_at(&mut out, changes, p.start);
                let text = match p.group {
                    Some(g) => self.group_text(g, len, p.tie),
                    None => self.rest_text(len),
                };
                out.push_str(&text);
                end = p.end;
                i += 1;
            } else {
                if !self.binary(off) {
                    return Err(self.off_grid(&p));
                }
                let (step, elements, next) = self
                    .triplet(pieces, i, bar_end, beat)
                    .ok_or_else(|| self.off_grid(&p))?;
                let group_end = p.start + 3 * step;
                let written = step * 3 / 2;
                // (3 is three elements; (3:2:n puts n elements, some of two slots, in the
                // time of two (§4.13).
                if elements.len() == 3 {
                    out.push_str("(3");
                } else {
                    let _ = write!(out, "(3:2:{}", elements.len());
                }
                for e in &elements {
                    self.changes_at(&mut out, changes, e.start);
                    let slots = (e.end - e.start) / step;
                    let text = match e.group {
                        Some(g) => self.group_text(g, written * slots, e.tie),
                        None => self.rest_text(written * slots),
                    };
                    out.push_str(&text);
                }
                // A rest running on past the group is written from where the group ends.
                if let Some(rest) = pieces.get_mut(next)
                    && rest.start < group_end
                {
                    rest.start = group_end;
                }
                i = next;
                end = group_end;
            }
            let into = end - bar.start;
            if end < bar_end && into.is_multiple_of(group_beat) {
                out.push(' ');
            }
        }
        Ok(out)
    }

    /// A triplet group starting at piece `i`: three equal slots that together fill whole
    /// beats, holding notes of one or two slots each, and rests split at the slots.
    /// Returns the slot length, the group's elements, and the index of the first piece
    /// not wholly inside it.
    fn triplet(
        &self,
        pieces: &[Piece],
        i: usize,
        bar_end: u64,
        beat: u64,
    ) -> Option<(u64, Vec<Piece>, usize)> {
        let start = pieces[i].start;
        // From the longest triplet (half-note triplets) to the shortest (64ths).
        'sizes: for k in 1..=6u32 {
            let written = self.whole_ticks >> k;
            if written == 0 || !(written * 2).is_multiple_of(3) {
                continue;
            }
            let step = written * 2 / 3;
            let group_end = start + 3 * step;
            if group_end > bar_end || !(3 * step).is_multiple_of(beat) {
                continue;
            }
            let mut elements = Vec::new();
            let mut j = i;
            let mut t = start;
            while t < group_end {
                let Some(piece) = pieces.get(j) else {
                    continue 'sizes;
                };
                match piece.group {
                    Some(_) => {
                        let fits = piece.start == t
                            && piece.end <= group_end
                            && (piece.end - t).is_multiple_of(step);
                        if !fits {
                            continue 'sizes;
                        }
                        elements.push(piece.clone());
                        t = piece.end;
                        j += 1;
                    }
                    None => {
                        let slot_end = t + step;
                        if piece.end < slot_end {
                            continue 'sizes;
                        }
                        elements.push(Piece {
                            start: t,
                            end: slot_end,
                            group: None,
                            tie: false,
                        });
                        t = slot_end;
                        if piece.end == slot_end {
                            j += 1;
                        }
                    }
                }
            }
            return Some((step, elements, j));
        }
        None
    }
}

/// Whether a velocity is one of the dynamics' levels, which ABC writes as it is.
fn is_level(vel: u8) -> bool {
    DYNAMICS.iter().any(|(_, v)| *v == vel)
}

/// How many of the part's notes have a velocity that is not a dynamic's level: the notes
/// [`write()`] snaps under [`Dynamics::Nearest`], and refuses otherwise. 0 means writing the
/// part changes no velocity.
pub fn off_level_velocities(part: &Part) -> usize {
    part.tracks
        .iter()
        .flat_map(|t| &t.notes)
        .filter(|n| !is_level(n.vel))
        .count()
}

/// Writes a part as one ABC tune, `X:1`.
///
/// Refused, naming what and where ([`Error::Unwritable`]): a note off the 1/64-note grid
/// that is not part of a triplet group (three equal slots filling whole beats, each note
/// one or two slots long);
/// notes that overlap but neither start and end together; a velocity that is not one of
/// the dynamics' levels (unless [`Dynamics::Nearest`]); a tempo that is not a whole bpm; a
/// track on channel 10 (drums are the drum grid's); controls; a bar cut short by a meter
/// change; a part that ends inside a bar; and tempo, meter or key changes in a part of
/// several tracks. The part is never changed.
///
/// ABC needs a key, so a part that names none is written in C major, and reads back so.
pub fn write(part: &Part, options: &WriteOptions) -> Result<String> {
    let tpq = u64::from(part.ticks_per_quarter);
    let whole_ticks = 4 * tpq;
    let map = part.time_map()?;
    part.check_end()?;
    let end = part.stated_end()?;
    let end_tick = map.tick(end)?;

    // The bars, each checked whole.
    let mut bars: Vec<BarSpan> = Vec::new();
    let mut number = 1;
    loop {
        let start = map.tick(Position::bar(number))?;
        if start >= end_tick {
            break;
        }
        if number > MAX_BARS {
            return Err(unwritable(
                end,
                None,
                format!("the part is longer than {MAX_BARS} bars"),
                "set an earlier end, or trim the part",
            ));
        }
        let next = map.tick(Position::bar(number + 1))?;
        let sig = map.sig_at(start);
        let full = whole_ticks * u64::from(sig.num) / u64::from(sig.den);
        if next - start != full {
            return Err(unwritable(
                Position::bar(number),
                None,
                format!("bar {number} is cut short by a meter change inside it"),
                "move the meter change to a bar line",
            ));
        }
        if next > end_tick {
            return Err(unwritable(
                end,
                None,
                format!("the part ends inside bar {number}"),
                "end the part on a bar line",
            ));
        }
        bars.push(BarSpan {
            start,
            len: full,
            sig,
        });
        number += 1;
    }

    // A first bar shorter than the second, at a meter change, is a pickup: written
    // before the first bar line, under the second bar's M:.
    let pickup = bars.len() >= 2
        && bars[0].sig != bars[1].sig
        && bars[0].len < bars[1].len
        && part
            .time_changes
            .first()
            .is_some_and(|c| map.tick(c.at).ok() == Some(bars[1].start))
        && pickup_meter(Ratio::new(bars[0].len, whole_ticks), bars[1].sig) == Some(bars[0].sig);
    let header_sig = match (pickup, bars.first()) {
        (true, _) => bars[1].sig,
        (false, Some(b)) => b.sig,
        (false, None) => part.time_sig,
    };

    let multi = part.tracks.len() > 1;
    let mut meter_changes: Vec<(u64, TimeSig)> = bars
        .windows(2)
        .enumerate()
        .filter(|(i, w)| w[0].sig != w[1].sig && !(pickup && *i == 0))
        .map(|(_, w)| (w[1].start, w[1].sig))
        .collect();
    // A meter change on the part's end opens no bar, so it is written after the last one.
    let last_sig = bars.last().map_or(part.time_sig, |b| b.sig);
    if map.sig_at(end_tick) != last_sig && !bars.is_empty() {
        meter_changes.push((end_tick, map.sig_at(end_tick)));
    }
    if multi
        && (!meter_changes.is_empty()
            || !part.key_changes.is_empty()
            || !part.tempo_changes.is_empty())
    {
        return Err(unwritable(
            Position::bar(1),
            None,
            "a part of several tracks with tempo, meter or key changes".to_owned(),
            "this version writes those changes only in a single-voice tune",
        ));
    }

    // L: is written always, so a reader that leaves it out still agrees with it.
    let unit = match options.unit {
        Some((n, d)) if n > 0 && d > 0 => Ratio::new(u64::from(n), u64::from(d)),
        Some(_) => {
            return Err(Error::invalid(
                "a unit length needs a positive fraction, such as 1/8",
            ));
        }
        None if u32::from(header_sig.num) * 4 < 3 * u32::from(header_sig.den) => Ratio::new(1, 16),
        None => Ratio::new(1, 8),
    };
    let unit_ticks = unit.ticks(whole_ticks).filter(|t| *t > 0).ok_or_else(|| {
        Error::invalid(format!("a unit of {unit} is not a whole number of ticks"))
    })?;

    let mut head = String::new();
    let _ = writeln!(head, "X:1");
    if let Some(title) = &part.title {
        let _ = writeln!(head, "T:{}", title.replace('%', "\\%").replace('\n', " "));
    }
    let _ = writeln!(head, "M:{header_sig}");
    let _ = writeln!(head, "L:{unit}");
    let _ = writeln!(
        head,
        "Q:1/4={}",
        whole_bpm(part.tempo_bpm, Position::bar(1))?
    );
    let key = part.key.unwrap_or(Key {
        sharps: 0,
        mode: Mode::Major,
    });
    let _ = writeln!(head, "K:{}", key_text(key));

    // Inline changes, by tick.
    // Inline changes, by tick: meter, then key, then tempo.
    let mut changes: Vec<Change> = Vec::new();
    for &(tick, sig) in &meter_changes {
        changes.push((tick, format!("[M:{sig}]"), None));
    }
    for c in &part.key_changes {
        changes.push((
            map.tick(c.at)?,
            format!("[K:{}]", key_text(c.key)),
            Some(c.key),
        ));
    }
    for c in &part.tempo_changes {
        changes.push((
            map.tick(c.at)?,
            format!("[Q:1/4={}]", whole_bpm(c.bpm, c.at)?),
            None,
        ));
    }
    changes.sort_by_key(|(t, _, _)| *t);

    let mut body = String::new();
    let mut snapped = false;
    let mut used_channels: Vec<u8> = Vec::new();
    for (index, track) in part.tracks.iter().enumerate() {
        let name = Some(track.name.as_str());
        let at1 = Position::bar(1);
        if track.channel == 10 {
            return Err(unwritable(
                at1,
                name,
                "a track on channel 10 is a drum part".to_owned(),
                "write drum parts as a drum grid, with duet drums read",
            ));
        }
        if let Some(control) = track.controls.first() {
            return Err(unwritable(
                control.at(),
                name,
                "controls (cc, bend and program changes) have no ABC form here".to_owned(),
                "remove the track's controls first",
            ));
        }
        if track.name.contains('"') {
            return Err(unwritable(
                at1,
                name,
                "a track name holding a double quote".to_owned(),
                "rename the track",
            ));
        }
        let default_channel = default_channel(&used_channels);
        used_channels.push(track.channel);
        let default_name = part.title.clone().unwrap_or_else(|| "Tune".to_owned());
        if multi || track.name != default_name {
            // A % would start a comment: \% is a literal percent (§2.2.5).
            let _ = writeln!(
                body,
                "V:{} name=\"{}\"",
                index + 1,
                track.name.replace('%', "\\%")
            );
        }
        if let Some(program) = track.program {
            let _ = writeln!(body, "%%MIDI program {program}");
        }
        if Some(track.channel) != default_channel {
            let _ = writeln!(body, "%%MIDI channel {}", track.channel);
        }

        // Chord groups, in the track's own order.
        let mut timed: Vec<(u64, u64, usize)> = Vec::new();
        for (i, note) in track.notes.iter().enumerate() {
            let start = map.tick(note.at)?;
            let len = ticks_from_beats(note.beats, part.ticks_per_quarter)?;
            if len == 0 {
                return Err(unwritable(
                    note.at,
                    name,
                    format!("the {} at {} has no length", note.pitch, note.at),
                    "give it a length, or remove it",
                ));
            }
            timed.push((start, start + len, i));
        }
        timed.sort_by_key(|(start, _, _)| *start);
        let mut groups: Vec<Group> = Vec::new();
        for (start, end, i) in timed {
            let note = &track.notes[i];
            let vel = if is_level(note.vel) {
                note.vel
            } else if options.dynamics == Dynamics::Nearest {
                snapped = true;
                DYNAMICS
                    .iter()
                    .min_by_key(|(_, v)| v.abs_diff(note.vel))
                    .map_or(DEFAULT_VELOCITY, |(_, v)| *v)
            } else {
                return Err(unwritable(
                    note.at,
                    name,
                    format!(
                        "the {} at {} has velocity {}, which is no dynamic's level (20, 35, 50, \
                         65, 80, 95, 110, 125)",
                        note.pitch, note.at, note.vel
                    ),
                    "write with --dynamics nearest to snap velocities, which says so in the ABC",
                ));
            };
            match groups.last_mut() {
                Some(g) if g.start == start => {
                    if g.end != end {
                        let other = &track.notes[g.notes[0]];
                        return Err(unwritable(
                            note.at,
                            name,
                            format!(
                                "the {} at {} and the {} at {} start together but end apart",
                                other.pitch, other.at, note.pitch, note.at
                            ),
                            "splitting into voices is a v2 candidate; make them one length",
                        ));
                    }
                    if g.vel != vel {
                        return Err(unwritable(
                            note.at,
                            name,
                            format!("the chord at {} has notes of different velocities", note.at),
                            "give the chord's notes one velocity",
                        ));
                    }
                    g.notes.push(i);
                }
                last => {
                    if let Some(g) = last
                        && start < g.end
                    {
                        let other = &track.notes[g.notes[0]];
                        return Err(unwritable(
                            note.at,
                            name,
                            format!(
                                "the {} at {} starts while the {} at {} still sounds",
                                note.pitch, note.at, other.pitch, other.at
                            ),
                            "splitting into voices is a v2 candidate; shorten the earlier note",
                        ));
                    }
                    groups.push(Group {
                        start,
                        end,
                        notes: vec![i],
                        vel,
                    });
                }
            }
        }

        // Pieces: notes and the rests between them, cut at bar lines and at changes.
        let mut cuts: Vec<u64> = bars.iter().map(|b| b.start).collect();
        if !multi {
            cuts.extend(changes.iter().map(|(t, _, _)| *t));
        }
        cuts.push(end_tick);
        cuts.sort_unstable();
        cuts.dedup();
        let mut raw: Vec<Piece> = Vec::new();
        let mut t = 0u64;
        for (g, group) in groups.iter().enumerate() {
            if group.start > t {
                raw.push(Piece {
                    start: t,
                    end: group.start,
                    group: None,
                    tie: false,
                });
            }
            raw.push(Piece {
                start: group.start,
                end: group.end,
                group: Some(g),
                tie: false,
            });
            t = group.end;
        }
        if t < end_tick {
            raw.push(Piece {
                start: t,
                end: end_tick,
                group: None,
                tie: false,
            });
        }
        let mut pieces: Vec<Piece> = Vec::new();
        for piece in raw {
            let mut start = piece.start;
            for &cut in cuts.iter().filter(|c| **c > piece.start && **c < piece.end) {
                let is_bar = bars.iter().any(|b| b.start == cut);
                if piece.group.is_some() && !is_bar {
                    let n = &track.notes[groups[piece.group.unwrap_or(0)].notes[0]];
                    return Err(unwritable(
                        map.position(cut),
                        name,
                        format!(
                            "a tempo or key change falls inside the {} at {}",
                            n.pitch, n.at
                        ),
                        "move the change to where a note starts",
                    ));
                }
                pieces.push(Piece {
                    start,
                    end: cut,
                    group: piece.group,
                    tie: piece.group.is_some(),
                });
                start = cut;
            }
            pieces.push(Piece {
                start,
                end: piece.end,
                group: piece.group,
                tie: false,
            });
        }

        let mut writer = TrackWriter {
            track,
            groups,
            unit_ticks,
            whole_ticks,
            vel: DEFAULT_VELOCITY,
            key,
            bar_acc: [None; 7],
            map: &map,
        };
        let track_changes: &[Change] = if multi { &[] } else { &changes };
        let mut texts = Vec::new();
        let mut rest = pieces.as_slice();
        for bar in &bars {
            let bar_end = bar.start + bar.len;
            let n = rest.iter().take_while(|p| p.start < bar_end).count();
            let mut in_bar = rest[..n].to_vec();
            rest = &rest[n..];
            let in_changes: Vec<Change> = track_changes
                .iter()
                .filter(|(t, _, _)| *t >= bar.start && *t < bar_end)
                .cloned()
                .collect();
            texts.push(writer.bar(bar, &mut in_bar, &in_changes)?);
        }
        // Changes on the part's end go after its last bar line, where they still fall.
        let at_end: String = track_changes
            .iter()
            .filter(|(t, _, _)| *t == end_tick)
            .map(|(_, text, _)| text.as_str())
            .collect();
        for (i, chunk) in texts.chunks(4).enumerate() {
            let last = (i + 1) * 4 >= texts.len();
            let close = match (last, at_end.is_empty()) {
                (false, _) => "|".to_owned(),
                (true, true) => "|]".to_owned(),
                (true, false) => format!("|] {at_end}"),
            };
            let _ = writeln!(body, "{} {close}", chunk.join(" | "));
        }
    }

    let mut out = head;
    if snapped {
        out.push_str("% duet: velocities were snapped to the nearest dynamic's level\n");
    }
    out.push_str(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickup_meter_for;
    use duet_core::{Note, TimeChange, TimeMap};

    #[test]
    fn lengths_are_written_as_multiples_of_the_unit() {
        assert_eq!(length_text(240, 240), "");
        assert_eq!(length_text(480, 240), "2");
        assert_eq!(length_text(120, 240), "/");
        assert_eq!(length_text(60, 240), "/4");
        assert_eq!(length_text(360, 240), "3/2");
    }

    #[test]
    fn notes_are_named_on_the_letter_they_are_spelled_on() {
        let name = |s: &str| note_name(s.parse().unwrap());
        assert_eq!(name("C4"), "C");
        assert_eq!(name("C5"), "c");
        assert_eq!(name("D6"), "d'");
        assert_eq!(name("B3"), "B,");
        assert_eq!(name("B2"), "B,,");
        assert_eq!(name("B#3"), "B,", "B#3 is MIDI 60");
        assert_eq!(name("Cb5"), "c", "Cb5 is MIDI 71");
        assert_eq!(name("E#4"), "E");
    }

    /// A part whose first bar is `first`, then `meter` from the change at `change_tick`, with
    /// one quarter note at the start of bar 2.
    fn first_bar_part(first: TimeSig, meter: TimeSig, change_tick: u64) -> Part {
        let mut part = Part::default();
        part.time_sig = first;
        let mut map = TimeMap::new(part.ticks_per_quarter, first).unwrap();
        let at = map.change_at_tick(change_tick, meter).unwrap();
        part.time_changes.push(TimeChange {
            at,
            time_sig: meter,
        });
        let mut track = Track::new("melody", 1);
        track.notes.push(Note::new(
            Position::bar(2),
            duet_core::Pitch::new(60).unwrap(),
            1.0,
            80,
        ));
        part.tracks.push(track);
        part
    }

    #[test]
    fn off_level_velocities_are_the_ones_nearest_snaps() {
        let mut part = first_bar_part(
            TimeSig::new(4, 4).unwrap(),
            TimeSig::new(4, 4).unwrap(),
            1920,
        );
        assert_eq!(off_level_velocities(&part), 0, "80 is mf's level");
        let note = part.tracks[0].notes[0].clone();
        part.tracks[0].notes[0].vel = 81;
        assert_eq!(off_level_velocities(&part), 1);
        let nearest = WriteOptions::default().with_dynamics(Dynamics::Nearest);
        assert!(write(&part, &nearest).unwrap().contains("snapped"));
        part.tracks[0].notes[0] = note;
        assert!(!write(&part, &nearest).unwrap().contains("snapped"));
    }

    #[test]
    fn pickup_meter_for_agrees_with_the_writer() {
        let sig = |s: &str| s.parse::<TimeSig>().unwrap();
        let wrote_a_pickup = |part: &Part| {
            let abc = write(part, &WriteOptions::default()).unwrap();
            // A pickup is written under the second bar's meter, with no meter of its own.
            abc.contains("M:4/4") && !abc.contains("M:1/4") && !abc.contains("[M:")
        };

        // A one-beat first bar before 4/4 is written as a pickup, and the rule says so.
        let pickup = first_bar_part(sig("1/4"), sig("4/4"), 480);
        assert!(
            wrote_a_pickup(&pickup),
            "{}",
            write(&pickup, &WriteOptions::default()).unwrap()
        );
        assert_eq!(pickup_meter_for(480, 480, sig("4/4")), Some(sig("1/4")));

        // A first bar of a 128th, cut short by the change, is no time signature: the writer
        // writes no pickup (it refuses the cut bar), and the rule refuses it too.
        let cut = first_bar_part(sig("4/4"), sig("3/4"), 15);
        let written = write(&cut, &WriteOptions::default());
        assert!(
            matches!(&written, Err(Error::Unwritable { why, .. }) if why.contains("cut short")),
            "{written:?}"
        );
        assert_eq!(pickup_meter_for(15, 480, sig("3/4")), None);
    }
}
