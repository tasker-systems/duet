use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// The letters' names, C = 0.
const LETTERS: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];
/// The pitch class of each natural letter, C = 0.
const NATURAL: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

/// A MIDI note number with a spelling, written in scientific pitch notation.
///
/// Middle C (MIDI 60) is `C4`, so the lowest note, MIDI 0, is `C-1`. The spelling is a
/// letter and its alteration: a double flat, a flat, none, a sharp or a double sharp, written
/// `bb`, `b`, ``, `#` and `##`. A black key is spelled with a sharp unless it is set to a flat,
/// and a white key is its natural letter unless it is spelled otherwise, as E#, Cb or F##.
/// Equality includes the spelling, so `A#3` and `Bb3` are the same MIDI note but not equal
/// `Pitch` values, nor are `B3` and `Cb4`, or `G4` and `F##4`. The octave is the letter's, so
/// `Cb4` is MIDI 59, a semitone below `C4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pitch {
    midi: u8,
    /// The letter's alteration, from -2 to 2.
    alter: i8,
}

/// Pitches order by MIDI note, and the same note by its spelling, sharpest first: `A#3`
/// before `Bb3`, and `B3` before `Cb4`.
impl Ord for Pitch {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.midi
            .cmp(&other.midi)
            .then(other.alter.cmp(&self.alter))
    }
}

impl PartialOrd for Pitch {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn is_black(pitch_class: u8) -> bool {
    matches!(pitch_class, 1 | 3 | 6 | 8 | 10)
}

/// The letter (C = 0) of a natural pitch class, if it is one.
fn natural_letter(pitch_class: i32) -> Option<u8> {
    NATURAL
        .iter()
        .position(|n| i32::from(*n) == pitch_class.rem_euclid(12))
        .map(|l| l as u8)
}

impl Pitch {
    /// A pitch from a MIDI note number (0 to 127), spelled with sharps.
    pub fn new(midi: u8) -> Result<Self> {
        if midi > 127 {
            return Err(Error::invalid(format!("MIDI note {midi} is above 127")));
        }
        Ok(Pitch {
            midi,
            alter: i8::from(is_black(midi % 12)),
        })
    }

    /// The same note, spelled with flats (`true`) or sharps (`false`). A black key takes the
    /// flat or the sharp; a white key is spelled as its natural letter either way.
    pub fn spelled_flat(self, flat: bool) -> Self {
        let alter = match (is_black(self.midi % 12), flat) {
            (false, _) => 0,
            (true, true) => -1,
            (true, false) => 1,
        };
        Pitch {
            midi: self.midi,
            alter,
        }
    }

    /// The same note spelled with this alteration of its letter, from -2 (a double flat) to
    /// 2 (a double sharp). `None` when no letter lies that far from the note, as for a double
    /// flat on MIDI 61 (two semitones above it is D#, no letter), or past a double accidental.
    pub fn spelled(self, alter: i8) -> Option<Self> {
        if !(-2..=2).contains(&alter) {
            return None;
        }
        natural_letter(i32::from(self.midi) - i32::from(alter)).map(|_| Pitch {
            midi: self.midi,
            alter,
        })
    }

    /// The MIDI note number.
    pub fn midi(self) -> u8 {
        self.midi
    }

    /// Whether this pitch is spelled with a flat.
    pub fn is_flat(self) -> bool {
        self.alter < 0
    }

    /// The alteration of the letter: -2 for a double flat, -1 for a flat, 0 for a natural, 1
    /// for a sharp and 2 for a double sharp.
    pub fn alteration(self) -> i8 {
        self.alter
    }

    /// The letter the pitch is spelled on, from C = 0 to B = 6: Cb is on C, not B.
    pub fn letter(self) -> u8 {
        natural_letter(i32::from(self.midi) - i32::from(self.alter))
            .expect("a pitch's alteration always leaves a natural letter")
    }

    /// The octave of the letter, so `Cb4` is in octave 4 and `B#3` in octave 3.
    pub fn octave(self) -> i32 {
        (i32::from(self.midi) - i32::from(self.alter)).div_euclid(12) - 1
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let accidental = match self.alter {
            -2 => "bb",
            -1 => "b",
            0 => "",
            1 => "#",
            _ => "##",
        };
        write!(
            f,
            "{}{}{}",
            LETTERS[usize::from(self.letter())],
            accidental,
            self.octave()
        )
    }
}

impl FromStr for Pitch {
    type Err = Error;

    /// Parses names like `C4`, `F#3`, `Bb2`, `c#-1`, `Cb4` (MIDI 59, spelled as Cb), `F##4`
    /// (MIDI 67) or `Bbb3` (MIDI 69). The spelling is kept as written. Up to two sharps or two
    /// flats are read; three, or sharps and flats together, are refused, since no spelling
    /// keeps them.
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
        let mut sharps = 0;
        let mut flats = 0;
        for c in accidentals.chars() {
            match c {
                '#' | '♯' => sharps += 1,
                'b' | '♭' => flats += 1,
                _ => return Err(bad()),
            }
        }
        if (sharps > 0 && flats > 0) || sharps > 2 || flats > 2 {
            return Err(Error::invalid(format!(
                "'{s}' has no spelling: a pitch takes up to two sharps or two flats, not both"
            )));
        }
        let shift = sharps - flats;
        let octave: i32 = octave.parse().map_err(|_| bad())?;
        let midi = (octave + 1) * 12 + base + shift;
        if !(0..=127).contains(&midi) {
            return Err(Error::invalid(format!(
                "pitch '{s}' is outside the MIDI range C-1 to G9"
            )));
        }
        Ok(Pitch {
            midi: midi as u8,
            alter: shift as i8,
        })
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
            for alter in -2..=2 {
                if let Some(p) = Pitch::new(midi).unwrap().spelled(alter) {
                    assert_eq!(p.to_string().parse::<Pitch>().unwrap(), p, "{p}");
                    assert_eq!(p.alteration(), alter);
                }
            }
        }
    }

    #[test]
    fn white_keys_can_be_spelled_on_the_next_letter() {
        let p = |midi: u8, alter: i8| Pitch::new(midi).unwrap().spelled(alter);
        assert_eq!(p(71, -1).unwrap().to_string(), "Cb5");
        assert_eq!(p(65, 1).unwrap().to_string(), "E#4");
        assert_eq!(p(64, -1).unwrap().to_string(), "Fb4");
        assert_eq!(p(60, 1).unwrap().to_string(), "B#3");
        assert_eq!(p(0, 1).unwrap().to_string(), "B#-2");
        assert_eq!(p(62, -2).unwrap().to_string(), "Ebb4");
        assert_eq!(p(62, 2).unwrap().to_string(), "C##4");
        assert_eq!(p(61, -2), None); // no letter is two semitones above C#
        assert_eq!(p(60, 3), None);
        assert_eq!(p(61, 0), None);
        // Spelled flat or sharp, a white key is its natural letter.
        assert_eq!(Pitch::new(71).unwrap().spelled_flat(true).to_string(), "B4");
    }

    #[test]
    fn known_names() {
        assert_eq!("C4".parse::<Pitch>().unwrap().midi(), 60);
        assert_eq!("A4".parse::<Pitch>().unwrap().midi(), 69);
        assert_eq!("C-1".parse::<Pitch>().unwrap().midi(), 0);
        assert_eq!("G9".parse::<Pitch>().unwrap().midi(), 127);
        assert_eq!("Cb4".parse::<Pitch>().unwrap().midi(), 59);
        assert_eq!("Cb4".parse::<Pitch>().unwrap().to_string(), "Cb4");
        assert_eq!("E#4".parse::<Pitch>().unwrap().midi(), 65);
        assert_eq!("E#4".parse::<Pitch>().unwrap().to_string(), "E#4");
        assert_eq!("B#3".parse::<Pitch>().unwrap().to_string(), "B#3");
        assert_eq!("Fb4".parse::<Pitch>().unwrap().to_string(), "Fb4");
        assert_eq!("Bbb3".parse::<Pitch>().unwrap().midi(), 57);
        assert_eq!("Bbb3".parse::<Pitch>().unwrap().to_string(), "Bbb3");
        assert_eq!("F##4".parse::<Pitch>().unwrap().midi(), 67);
        assert_eq!("F##4".parse::<Pitch>().unwrap().to_string(), "F##4");
        assert_eq!("C♯♯4".parse::<Pitch>().unwrap().to_string(), "C##4");
        for bad in ["C###4", "Cbbb4", "C#b4", "Cb#4"] {
            assert!(bad.parse::<Pitch>().is_err(), "{bad}");
        }
        assert_eq!("Bb2".parse::<Pitch>().unwrap().to_string(), "Bb2");
        assert_eq!("f#3".parse::<Pitch>().unwrap().to_string(), "F#3");
        assert!("H4".parse::<Pitch>().is_err());
        assert!("G#9".parse::<Pitch>().is_err());
        assert!("C".parse::<Pitch>().is_err());
    }
}
