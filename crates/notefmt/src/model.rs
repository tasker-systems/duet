//! The song model. These types serialize to JSON and TOML with serde, and to the
//! one-note-per-line text format in [`crate::text`].

use serde::{Deserialize, Serialize};

use crate::pitch::Pitch;
use crate::time::{Key, Position, TimeSig};

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
