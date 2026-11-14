//! The song model. These types serialize to JSON and TOML with serde, and to the
//! one-note-per-line text format in [`crate::text`].

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pitch::Pitch;
use crate::time::{
    Key, Position, TimeMap, TimeSig, beats_from_ticks, bpm_from_micros, micros_from_bpm,
    ticks_from_beats,
};

/// A whole song: its tempo, meter and key, and its tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    /// The song's name: the MIDI sequence name, from the first track of a format 1 file
    /// when that track holds no notes, or from the only track of a format 0 file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The file's resolution. Tick offsets in positions are counted in these ticks.
    pub ticks_per_quarter: u16,
    /// The tempo at the start, in quarter notes per minute.
    pub tempo_bpm: f64,
    /// The time signature at the start.
    pub time_sig: TimeSig,
    /// The key at the start, if the file names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_changes: Vec<TempoChange>,
    /// Time signature changes. Each one's position is read against the changes before it;
    /// one that falls mid-bar cuts that bar short.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_changes: Vec<TimeChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_changes: Vec<KeyChange>,
    /// Where the song ends, if it says: a bar.beat.tick position such as `9.1.0` (the bar
    /// line before bar 9), or one inside a bar, for a song that stops mid-bar. Nothing in
    /// the song may lie past it: a note may end on it, and no note may end after it.
    ///
    /// `None` means the song names no end, and its [default end](Song::default_end)
    /// applies: the end of the last bar holding an event. Every writer states the end it
    /// uses ([`Song::stated_end`]), so a written file always says where it ends, whatever
    /// default computed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Position>,
    pub tracks: Vec<Track>,
}

impl Default for Song {
    fn default() -> Self {
        Song {
            title: None,
            ticks_per_quarter: 480,
            tempo_bpm: 120.0,
            time_sig: TimeSig::default(),
            key: None,
            tempo_changes: Vec::new(),
            time_changes: Vec::new(),
            key_changes: Vec::new(),
            end: None,
            tracks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempoChange {
    pub at: Position,
    pub bpm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeChange {
    pub at: Position,
    pub time_sig: TimeSig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyChange {
    pub at: Position,
    pub key: Key,
}

/// One instrument part on one MIDI channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    /// The MIDI channel, 1 to 16 as DAWs number them.
    pub channel: u8,
    /// The General MIDI program (0 to 127) set at the start, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    pub notes: Vec<Note>,
    /// Controllers, pitch bends and later program changes, in the order they happen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
}

impl Track {
    pub fn new(name: impl Into<String>, channel: u8) -> Self {
        Track {
            name: name.into(),
            channel,
            program: None,
            notes: Vec::new(),
            controls: Vec::new(),
        }
    }
}

/// One note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Where the note starts, as bar.beat.tick.
    pub at: Position,
    pub pitch: Pitch,
    /// The length in quarter notes, whatever the time signature.
    pub beats: f64,
    /// Velocity, 1 to 127.
    pub vel: u8,
}

/// A channel event other than a note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Control {
    /// A control change, such as 64 for the sustain pedal or 1 for the mod wheel.
    Cc {
        at: Position,
        controller: u8,
        value: u8,
    },
    /// Pitch bend, from -8192 to 8191, with 0 as no bend.
    Bend { at: Position, value: i16 },
    /// A program change after the start of the track.
    Program { at: Position, program: u8 },
}

impl Control {
    pub fn at(&self) -> Position {
        match self {
            Control::Cc { at, .. } | Control::Bend { at, .. } | Control::Program { at, .. } => *at,
        }
    }
}

impl Song {
    /// Puts the song in the form reading MIDI would give: lengths rounded to whole
    /// ticks and written as their shortest decimal, tempos rounded to what MIDI stores,
    /// and positions written in canonical bar.beat.tick form (so `1.1.960` in 4/4 at 480
    /// ticks per quarter becomes `1.3.0`). The order of notes and controls is kept.
    pub fn normalize(&mut self) -> Result<()> {
        let tpq = self.ticks_per_quarter;
        let bpm = |bpm: f64| micros_from_bpm(bpm).map(bpm_from_micros);
        self.tempo_bpm = bpm(self.tempo_bpm)?;

        let mut map = TimeMap::new(tpq, self.time_sig)?;
        for change in &mut self.time_changes {
            let tick = map.tick(change.at)?;
            change.at = map.change_at_tick(tick, change.time_sig)?;
        }
        let canonical = |at| map.tick(at).map(|tick| map.position(tick));
        for change in &mut self.tempo_changes {
            change.at = canonical(change.at)?;
            change.bpm = bpm(change.bpm)?;
        }
        for change in &mut self.key_changes {
            change.at = canonical(change.at)?;
        }
        if let Some(end) = &mut self.end {
            *end = canonical(*end)?;
        }
        for track in &mut self.tracks {
            for note in &mut track.notes {
                note.at = canonical(note.at)?;
                note.beats = beats_from_ticks(ticks_from_beats(note.beats, tpq)?, tpq);
            }
            for control in &mut track.controls {
                let at = canonical(control.at())?;
                match control {
                    Control::Cc { at: a, .. }
                    | Control::Bend { at: a, .. }
                    | Control::Program { at: a, .. } => *a = at,
                }
            }
        }
        self.check_end()
    }

    /// Checks that nothing lies past the song's [end](Song::end), when it names one.
    ///
    /// A note may end exactly on the end, and a control or a tempo, meter or key change
    /// may sit on it; anything later is refused, naming it and the end. Nothing is ever
    /// cut to fit: move the end, or shorten or move what lies past it.
    pub fn check_end(&self) -> Result<()> {
        let Some(end) = self.end else {
            return Ok(());
        };
        let map = self.time_map()?;
        let end_tick = map.tick(end)?;
        let end = map.position(end_tick);
        let past = |what: String| {
            Err(Error::invalid(format!(
                "{what} lies past the song's end at {end}; move the end, or shorten or move \
                 what lies past it"
            )))
        };
        for c in &self.time_changes {
            if map.tick(c.at)? > end_tick {
                return past(format!("the time {} change at {}", c.time_sig, c.at));
            }
        }
        for c in &self.tempo_changes {
            if map.tick(c.at)? > end_tick {
                return past(format!("the tempo {} change at {}", c.bpm, c.at));
            }
        }
        for c in &self.key_changes {
            if map.tick(c.at)? > end_tick {
                return past(format!("the key {} change at {}", c.key, c.at));
            }
        }
        for track in &self.tracks {
            for note in &track.notes {
                let note_end =
                    map.tick(note.at)? + ticks_from_beats(note.beats, self.ticks_per_quarter)?;
                if note_end > end_tick {
                    return past(format!(
                        "track '{}': the {} at {}, which ends at {},",
                        track.name,
                        note.pitch,
                        note.at,
                        map.position(note_end)
                    ));
                }
            }
            for control in &track.controls {
                if map.tick(control.at())? > end_tick {
                    let what = match control {
                        Control::Cc { controller, .. } => format!("cc{controller}"),
                        Control::Bend { .. } => "bend".to_owned(),
                        Control::Program { .. } => "prog".to_owned(),
                    };
                    return past(format!(
                        "track '{}': the {what} at {}",
                        track.name,
                        control.at()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The end the song names, or else its [default end](Song::default_end): the end
    /// every writer states.
    pub fn stated_end(&self) -> Result<Position> {
        match self.end {
            Some(end) => Ok(end),
            None => self.default_end(),
        }
    }

    /// The end the song has when it names none: the bar line after the last bar that
    /// holds a note or a control. A note holds every bar it sounds in, so one that ends
    /// exactly on a bar line does not hold the bar after it; a control, or a note of no
    /// length, holds the bar it sits in. A song with no notes or controls ends at `1.1.0`.
    ///
    /// Tempo, meter and key changes do not hold a bar, but the default never comes before
    /// one: a change later than the last bar played moves the end to the first bar line
    /// at or after it.
    pub fn default_end(&self) -> Result<Position> {
        let map = self.time_map()?;
        let tpq = self.ticks_per_quarter;
        let mut last_bar = 0;
        let changes = self
            .time_changes
            .iter()
            .map(|c| c.at)
            .chain(self.tempo_changes.iter().map(|c| c.at))
            .chain(self.key_changes.iter().map(|c| c.at));
        for at in changes {
            // The bar line at or after the change: the bar before it, when it is on one.
            let at = map.position(map.tick(at)?);
            let before = if at.beat == 1 && at.tick == 0 {
                at.bar - 1
            } else {
                at.bar
            };
            last_bar = last_bar.max(before);
        }
        for track in &self.tracks {
            for note in &track.notes {
                let start = map.tick(note.at)?;
                let len = ticks_from_beats(note.beats, tpq)?;
                let held = if len == 0 { start } else { start + len - 1 };
                last_bar = last_bar.max(map.position(held).bar);
            }
            for control in &track.controls {
                last_bar = last_bar.max(map.position(map.tick(control.at())?).bar);
            }
        }
        Ok(Position::bar(last_bar + 1))
    }

    /// The map from positions to ticks for this song's time signatures.
    pub fn time_map(&self) -> Result<TimeMap> {
        let mut map = TimeMap::new(self.ticks_per_quarter, self.time_sig)?;
        for change in &self.time_changes {
            map.change_at(change.at, change.time_sig)?;
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_end(body: &str) -> String {
        let song = Song::from_text(&format!("time 4/4\n{body}")).expect("a song");
        song.default_end().expect("an end").to_string()
    }

    #[test]
    fn the_default_end_is_the_bar_line_after_the_last_bar_holding_an_event() {
        let track = "\n[track \"P\" ch1]\n";
        assert_eq!(default_end(""), "1.1.0", "nothing played");
        assert_eq!(default_end(&format!("{track}1.1.0 C4 1 80\n")), "2.1.0");
        assert_eq!(
            default_end(&format!("{track}1.3.0 C4 7 80\n")),
            "4.1.0",
            "a note holds every bar it sounds in"
        );
        assert_eq!(
            default_end(&format!("{track}1.3.0 C4 2 80\n")),
            "2.1.0",
            "a note ending on a bar line does not hold the bar after it"
        );
        assert_eq!(default_end(&format!("{track}2.1.0 C4 0 80\n")), "3.1.0");
        assert_eq!(default_end(&format!("{track}5.4.0 cc64 0\n")), "6.1.0");
        assert_eq!(
            default_end(&format!("tempo 90 at 9.1.0\n{track}1.1.0 C4 1 80\n")),
            "9.1.0",
            "a tempo change holds no bar, but the end never comes before it"
        );
        assert_eq!(
            default_end(&format!("key D minor at 4.2.0\n{track}1.1.0 C4 1 80\n")),
            "5.1.0"
        );
        assert_eq!(
            default_end(&format!("tempo 90 at 1.3.0\n{track}5.1.0 C4 1 80\n")),
            "6.1.0"
        );
    }
}
