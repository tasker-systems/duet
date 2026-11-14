use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

const SHARP_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLAT_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// A MIDI note number with a spelling preference, written in scientific pitch notation.
///
/// Middle C (MIDI 60) is `C4`, so the lowest note, MIDI 0, is `C-1`. Black keys are
/// spelled with sharps unless `flat` is set; white keys ignore it. Equality includes the
/// spelling, so `A#3` and `Bb3` are the same MIDI note but not equal `Pitch` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pitch {
    midi: u8,
    flat: bool,
}

fn is_black(pitch_class: u8) -> bool {
    matches!(pitch_class, 1 | 3 | 6 | 8 | 10)
}

impl Pitch {
    /// A pitch from a MIDI note number (0 to 127), spelled with sharps.
    pub fn new(midi: u8) -> Result<Self> {
        if midi > 127 {
            return Err(Error::invalid(format!("MIDI note {midi} is above 127")));
        }
        Ok(Pitch { midi, flat: false })
    }

    /// The same note, spelled with flats (`true`) or sharps (`false`).
    pub fn spelled_flat(self, flat: bool) -> Self {
        Pitch {
            midi: self.midi,
            flat: flat && is_black(self.midi % 12),
        }
    }

    /// The MIDI note number.
    pub fn midi(self) -> u8 {
        self.midi
    }

    /// Whether this pitch is spelled with a flat.
    pub fn is_flat(self) -> bool {
        self.flat
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = if self.flat { &FLAT_NAMES } else { &SHARP_NAMES };
        let octave = i32::from(self.midi / 12) - 1;
        write!(f, "{}{}", names[usize::from(self.midi % 12)], octave)
    }
}

impl FromStr for Pitch {
    type Err = Error;

    /// Parses names like `C4`, `F#3`, `Bb2`, `c#-1` or `Cb4` (which is B3).
    fn from_str(s: &str) -> Result<Self> {
        let bad = || Error::invalid(format!("'{s}' is not a pitch like C4, F#3 or Bb2"));
        let mut chars = s.chars();
        let letter = chars.next().ok_or_else(bad)?;
        let base: i32 = match letter.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return Err(bad()),
        };
        let rest = chars.as_str();
        let octave_start = rest
            .find(|c: char| c == '-' || c.is_ascii_digit())
            .ok_or_else(bad)?;
        let (accidentals, octave) = rest.split_at(octave_start);
        let mut shift = 0;
        let mut flat = false;
        for c in accidentals.chars() {
            match c {
                '#' | '♯' => shift += 1,
                'b' | '♭' => {
                    shift -= 1;
                    flat = true;
                }
                _ => return Err(bad()),
            }
        }
        let octave: i32 = octave.parse().map_err(|_| bad())?;
        let midi = (octave + 1) * 12 + base + shift;
        if !(0..=127).contains(&midi) {
            return Err(Error::invalid(format!(
                "pitch '{s}' is outside the MIDI range C-1 to G9"
            )));
        }
        Ok(Pitch::new(midi as u8)?.spelled_flat(flat))
    }
}

impl Serialize for Pitch {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Pitch {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for midi in 0..=127u8 {
            for flat in [false, true] {
                let p = Pitch::new(midi).unwrap().spelled_flat(flat);
                assert_eq!(p.to_string().parse::<Pitch>().unwrap(), p);
            }
        }
    }

    #[test]
    fn known_names() {
        assert_eq!("C4".parse::<Pitch>().unwrap().midi(), 60);
        assert_eq!("A4".parse::<Pitch>().unwrap().midi(), 69);
        assert_eq!("C-1".parse::<Pitch>().unwrap().midi(), 0);
        assert_eq!("G9".parse::<Pitch>().unwrap().midi(), 127);
        assert_eq!("Cb4".parse::<Pitch>().unwrap().to_string(), "B3");
        assert_eq!("Bb2".parse::<Pitch>().unwrap().to_string(), "Bb2");
        assert_eq!("f#3".parse::<Pitch>().unwrap().to_string(), "F#3");
        assert!("H4".parse::<Pitch>().is_err());
        assert!("G#9".parse::<Pitch>().is_err());
        assert!("C".parse::<Pitch>().is_err());
    }
}
