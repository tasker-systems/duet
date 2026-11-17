//! Musical time: bar.beat.tick positions, time signatures, keys, and the map from
//! positions to absolute MIDI ticks.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::pitch::Pitch;

macro_rules! string_serde {
    ($ty:ty) => {
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let s = <String as serde::Deserialize>::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}
pub(crate) use string_serde;

/// A position as bar, beat and tick, all counted the way a DAW shows them.
///
/// `bar` and `beat` start at 1. A beat is one unit of the time signature's denominator
/// (a quarter in 4/4, an eighth in 6/8). `tick` is the offset into that beat in the
/// file's own ticks, so `1.2.240` at 480 ticks per quarter is an eighth after beat 2.
/// Written as `bar.beat.tick`; `bar.beat` alone means tick 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub bar: u32,
    pub beat: u32,
    pub tick: u32,
}

impl Position {
    pub fn new(bar: u32, beat: u32, tick: u32) -> Self {
        Position { bar, beat, tick }
    }

    /// The start of a bar.
    pub fn bar(bar: u32) -> Self {
        Position::new(bar, 1, 0)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.bar, self.beat, self.tick)
    }
}

impl FromStr for Position {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bad = || Error::invalid(format!("'{s}' is not a position like 3.2.240"));
        let parts: Vec<&str> = s.split('.').collect();
        if !(2..=3).contains(&parts.len()) {
            return Err(bad());
        }
        let num = |p: &str| p.parse::<u32>().map_err(|_| bad());
        let bar = num(parts[0])?;
        let beat = num(parts[1])?;
        let tick = if parts.len() == 3 { num(parts[2])? } else { 0 };
        if bar == 0 || beat == 0 {
            return Err(Error::invalid(format!(
                "position '{s}': bars and beats start at 1"
            )));
        }
        Ok(Position { bar, beat, tick })
    }
}

string_serde!(Position);

/// A time signature such as 4/4 or 6/8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeSig {
    pub num: u8,
    pub den: u8,
}

impl TimeSig {
    pub fn new(num: u8, den: u8) -> Result<Self> {
        if num == 0 {
            return Err(Error::invalid("a time signature needs at least one beat"));
        }
        if !den.is_power_of_two() || den > 64 {
            return Err(Error::invalid(format!(
                "time signature denominator {den} is not 1, 2, 4, 8, 16, 32 or 64"
            )));
        }
        Ok(TimeSig { num, den })
    }

    /// The denominator as the power of two MIDI stores (4 is 2, 8 is 3).
    pub fn den_power(self) -> u8 {
        self.den.trailing_zeros() as u8
    }
}

impl Default for TimeSig {
    fn default() -> Self {
        TimeSig { num: 4, den: 4 }
    }
}

impl fmt::Display for TimeSig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl FromStr for TimeSig {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bad = || Error::invalid(format!("'{s}' is not a time signature like 4/4"));
        let (num, den) = s.split_once('/').ok_or_else(bad)?;
        TimeSig::new(
            num.trim().parse().map_err(|_| bad())?,
            den.trim().parse().map_err(|_| bad())?,
        )
    }
}

string_serde!(TimeSig);

/// Tonic names around the circle of fifths, from Fb (8 flats from C) to B# (12 sharps).
/// A key's tonic is the name `sharps + mode offset` steps from C.
const FIFTHS: [&str; 21] = [
    "Fb", "Cb", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#", "G#",
    "D#", "A#", "E#", "B#",
];
/// The index of C in [`FIFTHS`].
const FIFTHS_C: i8 = 8;

/// A key's mode: major, minor, or one of the five other diatonic modes.
///
/// Ionian is major and aeolian is minor, so they have no variants of their own: `C ionian`
/// reads as `C major`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
}

impl Mode {
    /// The mode's name as notefmt writes it.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Major => "major",
            Mode::Minor => "minor",
            Mode::Dorian => "dorian",
            Mode::Phrygian => "phrygian",
            Mode::Lydian => "lydian",
            Mode::Mixolydian => "mixolydian",
            Mode::Locrian => "locrian",
        }
    }

    /// The mode a word names, matched as the ABC 2.1 standard matches modes (§3.1.14): on
    /// its first three letters, ignoring case, with `m` alone for minor. So `Dor`, `dorian`
    /// and `DORIAN` are all dorian, and `ionian` and `aeolian` are major and minor.
    pub fn from_word(word: &str) -> Option<Mode> {
        let lower = word.to_ascii_lowercase();
        if lower == "m" {
            return Some(Mode::Minor);
        }
        if lower.len() < 3 || !lower.is_char_boundary(3) {
            return None;
        }
        Some(match &lower[..3] {
            "maj" | "ion" => Mode::Major,
            "min" | "aeo" => Mode::Minor,
            "dor" => Mode::Dorian,
            "phr" => Mode::Phrygian,
            "lyd" => Mode::Lydian,
            "mix" => Mode::Mixolydian,
            "loc" => Mode::Locrian,
            _ => return None,
        })
    }

    /// How many steps round the circle of fifths the tonic lies from the major key with
    /// the same signature: D dorian is two steps from C.
    fn fifths_from_major(self) -> i8 {
        match self {
            Mode::Lydian => -1,
            Mode::Major => 0,
            Mode::Mixolydian => 1,
            Mode::Dorian => 2,
            Mode::Minor => 3,
            Mode::Phrygian => 4,
            Mode::Locrian => 5,
        }
    }

    /// The scale's steps above the tonic, in semitones. Minor is spelled as harmonic
    /// minor, so its leading tone is raised; the other modes are their natural scales.
    fn spelling_steps(self) -> [u8; 7] {
        match self {
            Mode::Major => [0, 2, 4, 5, 7, 9, 11],
            Mode::Minor => [0, 2, 3, 5, 7, 8, 11],
            Mode::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Mode::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Mode::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Mode::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Mode::Locrian => [0, 1, 3, 5, 6, 8, 10],
        }
    }
}

/// A key signature, stored the way MIDI does (sharps, positive, or flats, negative), and
/// the key's mode. Written as `D minor`, `Bb major` or `D dorian`.
///
/// MIDI's key signature holds only major or minor. A key in another mode is written to
/// MIDI as its signature plus a `duet:key D dorian` text event, which `duet-midi` reads
/// back; other programs see the signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub sharps: i8,
    pub mode: Mode,
}

impl Key {
    /// A major (`minor` false) or minor key with this many sharps (or flats, negative).
    pub fn new(sharps: i8, minor: bool) -> Result<Self> {
        Key::with_mode(sharps, if minor { Mode::Minor } else { Mode::Major })
    }

    /// A key in any mode, with this many sharps (or flats, negative) in its signature.
    pub fn with_mode(sharps: i8, mode: Mode) -> Result<Self> {
        if !(-7..=7).contains(&sharps) {
            return Err(Error::invalid(format!(
                "a key has between 7 flats and 7 sharps, not {sharps}"
            )));
        }
        Ok(Key { sharps, mode })
    }

    /// Whether MIDI's key signature marks this key as minor. Only the minor mode is; the
    /// other modes are written as their signature with the major flag.
    pub fn is_minor(self) -> bool {
        self.mode == Mode::Minor
    }

    /// The key's tonic, such as `D` or `Bb`.
    pub fn tonic(self) -> &'static str {
        FIFTHS[(self.sharps + self.mode.fifths_from_major() + FIFTHS_C) as usize]
    }

    /// The accidental this key's scale spells a pitch class (0 to 11, C = 0) with, if it is
    /// a note of the scale: harmonic minor for minor, so the leading tone is raised, and the
    /// natural scale for every other mode. -1 is a flat, 1 a sharp, 2 a double sharp (the
    /// leading tone of D# minor is C##).
    fn scale_accidental(self, pitch_class: u8) -> Option<i32> {
        const NATURAL: [u8; 7] = [0, 2, 4, 5, 7, 9, 11]; // C D E F G A B
        let tonic = self.tonic();
        let letter = match tonic.as_bytes()[0] {
            b'C' => 0,
            b'D' => 1,
            b'E' => 2,
            b'F' => 3,
            b'G' => 4,
            b'A' => 5,
            _ => 6,
        };
        let shift: i32 = tonic[1..]
            .chars()
            .map(|c| if c == '#' { 1 } else { -1 })
            .sum();
        let tonic_pc = (i32::from(NATURAL[letter]) + shift).rem_euclid(12);
        self.mode
            .spelling_steps()
            .iter()
            .enumerate()
            .find_map(|(degree, step)| {
                let pc = (tonic_pc + i32::from(*step)).rem_euclid(12);
                (pc == i32::from(pitch_class % 12)).then(|| {
                    let natural = i32::from(NATURAL[(letter + degree) % 7]);
                    (pc - natural + 18).rem_euclid(12) - 6
                })
            })
    }

    /// Whether a black key (pitch class 0 to 11, C = 0) should be spelled as a flat in this
    /// key. Notes of the scale (harmonic minor for minor, so the leading tone is raised,
    /// and the natural scale for every other mode) are spelled as the scale spells them, so
    /// D minor has Bb and C#. Other black keys follow the key signature: flats in flat
    /// keys, sharps otherwise.
    pub fn spells_flat(self, pitch_class: u8) -> bool {
        match self.scale_accidental(pitch_class) {
            Some(accidental) => accidental < 0,
            None => self.sharps < 0,
        }
    }

    /// The note spelled as this key spells it. A note of the scale is spelled as the scale
    /// spells it, so Eb minor has Cb, F# major has E#, F# minor's leading tone is E# and D#
    /// minor's is C##. Another black key follows the signature, flat in flat keys and sharp
    /// otherwise, and another white key is its natural letter, so the C of Eb dorian is C,
    /// written with a natural under the signature. The MIDI note never changes.
    pub fn spell(self, pitch: Pitch) -> Pitch {
        let pc = pitch.midi() % 12;
        self.scale_accidental(pc)
            .and_then(|accidental| pitch.spelled(i8::try_from(accidental).ok()?))
            .unwrap_or_else(|| pitch.spelled_flat(self.sharps < 0))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.tonic(), self.mode.name())
    }
}

impl FromStr for Key {
    type Err = Error;

    /// Parses a tonic and a mode: `D minor`, `Dm`, `Bb major`, plain `Bb` (major),
    /// `D dorian`, `F#Mix`. Modes are matched on their first three letters, ignoring case,
    /// as [`Mode::from_word`] says.
    fn from_str(s: &str) -> Result<Self> {
        let bad = || {
            Error::invalid(format!(
                "'{s}' is not a key like D minor, Bb major or D dorian"
            ))
        };
        let s = s.trim();
        let mut chars = s.chars();
        let letter = chars.next().ok_or_else(bad)?.to_ascii_uppercase();
        let rest = chars.as_str();
        // A "b" straight after the letter is a flat: no mode word starts with "b".
        let accidental_len = rest
            .chars()
            .take_while(|c| *c == '#' || *c == 'b')
            .map(char::len_utf8)
            .sum::<usize>();
        let (accidental, mode) = rest.split_at(accidental_len);
        let tonic = format!("{letter}{accidental}");
        let mode = match mode.trim() {
            "" => Mode::Major,
            word => Mode::from_word(word).ok_or_else(bad)?,
        };
        let index = FIFTHS.iter().position(|n| *n == tonic).ok_or_else(bad)? as i8;
        let sharps = index - FIFTHS_C - mode.fifths_from_major();
        if !(-7..=7).contains(&sharps) {
            return Err(Error::invalid(format!(
                "'{s}' has no standard key signature: it needs {} {}",
                sharps.abs(),
                if sharps < 0 { "flats" } else { "sharps" }
            )));
        }
        Key::with_mode(sharps, mode)
    }
}

string_serde!(Key);

/// One stretch of the song in a single time signature.
#[derive(Debug, Clone, Copy)]
struct Segment {
    start_tick: u64,
    start_bar: u32,
    sig: TimeSig,
}

/// Converts between absolute ticks and bar.beat.tick positions.
///
/// A time signature change starts a new bar where it falls. If it falls in the middle of
/// a bar, that bar is cut short and the next bar number begins at the change, which is
/// how DAWs treat it too.
#[derive(Debug, Clone)]
pub struct TimeMap {
    tpq: u32,
    segments: Vec<Segment>,
}

impl TimeMap {
    pub fn new(ticks_per_quarter: u16, initial: TimeSig) -> Result<Self> {
        if ticks_per_quarter == 0 {
            return Err(Error::invalid("ticks per quarter must be above 0"));
        }
        let map = TimeMap {
            tpq: u32::from(ticks_per_quarter),
            segments: vec![Segment {
                start_tick: 0,
                start_bar: 1,
                sig: initial,
            }],
        };
        map.beat_ticks(initial)?;
        Ok(map)
    }

    fn beat_ticks(&self, sig: TimeSig) -> Result<u64> {
        let whole = self.tpq * 4;
        if !whole.is_multiple_of(u32::from(sig.den)) {
            return Err(Error::invalid(format!(
                "{} ticks per quarter cannot divide into beats of 1/{}",
                self.tpq, sig.den
            )));
        }
        Ok(u64::from(whole / u32::from(sig.den)))
    }

    /// The position of an absolute tick.
    pub fn position(&self, tick: u64) -> Position {
        let seg = self
            .segments
            .iter()
            .rev()
            .find(|s| s.start_tick <= tick)
            .expect("the first segment starts at tick 0");
        // Every segment's signature was checked by beat_ticks when it was added.
        let beat_ticks = self.beat_ticks(seg.sig).unwrap_or(1);
        let bar_ticks = beat_ticks * u64::from(seg.sig.num);
        let offset = tick - seg.start_tick;
        let bars = offset / bar_ticks;
        let in_bar = offset % bar_ticks;
        Position {
            bar: seg.start_bar + bars as u32,
            beat: (in_bar / beat_ticks) as u32 + 1,
            tick: (in_bar % beat_ticks) as u32,
        }
    }

    /// The time signature in force at an absolute tick.
    pub fn sig_at(&self, tick: u64) -> TimeSig {
        self.segments
            .iter()
            .rev()
            .find(|s| s.start_tick <= tick)
            .expect("the first segment starts at tick 0")
            .sig
    }

    /// The absolute tick of a position. Tick offsets past the end of a beat, and beats
    /// past the end of a bar, simply carry on into the next ones.
    pub fn tick(&self, pos: Position) -> Result<u64> {
        if pos.bar == 0 || pos.beat == 0 {
            return Err(Error::invalid(format!(
                "position {pos}: bars and beats start at 1"
            )));
        }
        let seg = self
            .segments
            .iter()
            .rev()
            .find(|s| s.start_bar <= pos.bar)
            .expect("the first segment starts at bar 1");
        let beat_ticks = self.beat_ticks(seg.sig)?;
        let bar_ticks = beat_ticks * u64::from(seg.sig.num);
        Ok(seg.start_tick
            + u64::from(pos.bar - seg.start_bar) * bar_ticks
            + u64::from(pos.beat - 1) * beat_ticks
            + u64::from(pos.tick))
    }

    /// Adds a time signature change at an absolute tick, and returns where it falls
    /// (its position just before the change takes effect). Changes must come in order.
    pub fn change_at_tick(&mut self, tick: u64, sig: TimeSig) -> Result<Position> {
        self.beat_ticks(sig)?;
        let last = *self.segments.last().expect("never empty");
        if tick < last.start_tick {
            return Err(Error::invalid(format!(
                "time signature change at tick {tick} comes before an earlier change"
            )));
        }
        let at = self.position(tick);
        if tick == last.start_tick {
            self.segments.last_mut().expect("never empty").sig = sig;
        } else {
            let start_bar = if at.beat == 1 && at.tick == 0 {
                at.bar
            } else {
                at.bar + 1
            };
            self.segments.push(Segment {
                start_tick: tick,
                start_bar,
                sig,
            });
        }
        Ok(at)
    }

    /// Adds a time signature change at a position, read against the changes before it.
    pub fn change_at(&mut self, at: Position, sig: TimeSig) -> Result<u64> {
        let tick = self.tick(at)?;
        self.change_at_tick(tick, sig)?;
        Ok(tick)
    }
}

/// The shortest decimal near `value` (up to 9 places) that `fits` accepts.
fn shortest_decimal(value: f64, fits: impl Fn(f64) -> bool) -> f64 {
    for places in 0..=9 {
        let candidate: f64 = format!("{value:.places$}").parse().unwrap_or(value);
        if fits(candidate) {
            return candidate;
        }
    }
    value
}

/// A length in ticks as quarter notes, using the shortest decimal that converts back to
/// exactly the same number of ticks.
pub fn beats_from_ticks(ticks: u64, tpq: u16) -> f64 {
    let tpq = f64::from(tpq);
    shortest_decimal(ticks as f64 / tpq, |b| (b * tpq).round() == ticks as f64)
}

/// A length in quarter notes as ticks, rounded to the nearest tick.
pub fn ticks_from_beats(beats: f64, tpq: u16) -> Result<u64> {
    if !beats.is_finite() || beats < 0.0 {
        return Err(Error::invalid(format!(
            "length {beats} is not zero or a positive number of beats"
        )));
    }
    Ok((beats * f64::from(tpq)).round() as u64)
}

/// A tempo in microseconds per quarter as beats per minute, using the shortest decimal
/// that converts back to exactly the same microseconds.
pub fn bpm_from_micros(micros: u32) -> f64 {
    shortest_decimal(60_000_000.0 / f64::from(micros), |bpm| {
        (60_000_000.0 / bpm).round() == f64::from(micros)
    })
}

/// A tempo in beats per minute as microseconds per quarter, as MIDI stores it.
pub fn micros_from_bpm(bpm: f64) -> Result<u32> {
    if !bpm.is_finite() || bpm <= 0.0 {
        return Err(Error::invalid(format!("tempo {bpm} is not a positive bpm")));
    }
    let micros = (60_000_000.0 / bpm).round();
    if !(1.0..=16_777_215.0).contains(&micros) {
        return Err(Error::invalid(format!(
            "tempo {bpm} bpm is outside what MIDI can store"
        )));
    }
    Ok(micros as u32)
}

/// Parses a length in quarter notes, as a decimal (`0.5`) or a fraction (`1/3`).
pub fn parse_beats(s: &str) -> Result<f64> {
    let bad = || Error::invalid(format!("'{s}' is not a length like 1, 0.5 or 1/3"));
    let value = match s.split_once('/') {
        Some((n, d)) => {
            let n: f64 = n.parse().map_err(|_| bad())?;
            let d: f64 = d.parse().map_err(|_| bad())?;
            if d == 0.0 {
                return Err(bad());
            }
            n / d
        }
        None => s.parse().map_err(|_| bad())?,
    };
    if !value.is_finite() || value < 0.0 {
        return Err(bad());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_trip_through_ticks() {
        let mut map = TimeMap::new(480, TimeSig::new(4, 4).unwrap()).unwrap();
        // 6/8 from bar 3, then 3/4 in the middle of a 6/8 bar.
        map.change_at(Position::bar(3), TimeSig::new(6, 8).unwrap())
            .unwrap();
        map.change_at(Position::new(4, 4, 0), TimeSig::new(3, 4).unwrap())
            .unwrap();
        for tick in (0..20_000).step_by(7) {
            assert_eq!(map.tick(map.position(tick)).unwrap(), tick);
        }
        assert_eq!(map.position(0), Position::bar(1));
        assert_eq!(map.position(480 * 4 + 240), Position::new(2, 1, 240));
        assert_eq!(map.position(480 * 8), Position::bar(3));
        assert_eq!(map.position(480 * 8 + 240), Position::new(3, 2, 0));
        // Bar 4 is cut short after three eighths, and bar 5 starts in 3/4.
        let cut = 480 * 8 + 240 * 6 + 240 * 3;
        assert_eq!(map.position(cut), Position::bar(5));
        assert_eq!(map.position(cut - 1), Position::new(4, 3, 239));
    }

    #[test]
    fn decimals_are_exact() {
        for tpq in [96u16, 384, 480, 960] {
            for ticks in 0..2000u64 {
                let beats = beats_from_ticks(ticks, tpq);
                assert_eq!(ticks_from_beats(beats, tpq).unwrap(), ticks);
                let text: f64 = beats.to_string().parse().unwrap();
                assert_eq!(text, beats);
            }
        }
        assert_eq!(beats_from_ticks(240, 480), 0.5);
        assert_eq!(beats_from_ticks(160, 480), 0.333);
        for micros in (200_000..1_500_000).step_by(997) {
            assert_eq!(micros_from_bpm(bpm_from_micros(micros)).unwrap(), micros);
        }
        assert_eq!(bpm_from_micros(652_174), 92.0);
    }

    #[test]
    fn keys() {
        assert_eq!(
            "D minor".parse::<Key>().unwrap(),
            Key::new(-1, true).unwrap()
        );
        assert_eq!("Dm".parse::<Key>().unwrap(), Key::new(-1, true).unwrap());
        assert_eq!("Bb".parse::<Key>().unwrap(), Key::new(-2, false).unwrap());
        assert_eq!(
            "F# major".parse::<Key>().unwrap(),
            Key::new(6, false).unwrap()
        );
        assert!("D# major".parse::<Key>().is_err());
        for sharps in -7..=7 {
            for minor in [false, true] {
                let key = Key::new(sharps, minor).unwrap();
                assert_eq!(key.to_string().parse::<Key>().unwrap(), key);
            }
        }
    }

    #[test]
    fn spelling_follows_the_key() {
        let spell = |key: &str| -> Vec<bool> {
            let key: Key = key.parse().unwrap();
            [1u8, 3, 6, 8, 10]
                .iter()
                .map(|pc| key.spells_flat(*pc))
                .collect()
        };
        // C#/Db, D#/Eb, F#/Gb, G#/Ab, A#/Bb
        assert_eq!(spell("D minor"), [false, true, true, true, true]); // C#, then flats
        assert_eq!(spell("A minor"), [false; 5]); // G#, then sharps
        assert_eq!(spell("C minor"), [true; 5]); // Eb, Ab, Bb, then flats
        assert_eq!(spell("Bb major"), [true; 5]);
        assert_eq!(spell("E major"), [false; 5]);
        assert_eq!(spell("F major"), [true; 5]); // Bb, then flats
    }

    #[test]
    fn six_and_seven_accidental_keys_spell_their_scales() {
        let scale = |key: &str, midi: &[u8]| -> Vec<String> {
            let key: Key = key.parse().unwrap();
            midi.iter()
                .map(|m| key.spell(Pitch::new(*m).unwrap()).to_string())
                .collect()
        };
        // Eb natural minor, its harmonic leading tone, and the dorian C: a natural.
        assert_eq!(
            scale("Eb minor", &[63, 65, 66, 68, 70, 71, 73, 74, 72]),
            ["Eb4", "F4", "Gb4", "Ab4", "Bb4", "Cb5", "Db5", "D5", "C5"]
        );
        assert_eq!(
            scale("Gb major", &[66, 68, 70, 71, 73, 75, 77]),
            ["Gb4", "Ab4", "Bb4", "Cb5", "Db5", "Eb5", "F5"]
        );
        assert_eq!(
            scale("F# major", &[66, 68, 70, 71, 73, 75, 77]),
            ["F#4", "G#4", "A#4", "B4", "C#5", "D#5", "E#5"]
        );
        // D# minor's leading tone is C##, and G# minor's F##.
        assert_eq!(
            scale("D# minor", &[63, 65, 66, 68, 70, 71, 74]),
            ["D#4", "E#4", "F#4", "G#4", "A#4", "B4", "C##5"]
        );
        assert_eq!(
            scale("G# minor", &[68, 70, 71, 73, 75, 76, 79]),
            ["G#4", "A#4", "B4", "C#5", "D#5", "E5", "F##5"]
        );
        // And the leading tones of F# minor and C# minor are E# and B#.
        assert_eq!(scale("F# minor", &[77]), ["E#5"]);
        assert_eq!(scale("C# minor", &[72]), ["B#4"]);
        assert_eq!(
            scale("C# major", &[61, 63, 65, 66, 68, 70, 72]),
            ["C#4", "D#4", "E#4", "F#4", "G#4", "A#4", "B#4"]
        );
        assert_eq!(
            scale("Cb major", &[59, 61, 63, 64, 66, 68, 70]),
            ["Cb4", "Db4", "Eb4", "Fb4", "Gb4", "Ab4", "Bb4"]
        );
        // Spelling never changes the note.
        for sharps in -7..=7 {
            for minor in [false, true] {
                let key = Key::new(sharps, minor).unwrap();
                for midi in 0..=127 {
                    assert_eq!(key.spell(Pitch::new(midi).unwrap()).midi(), midi);
                }
            }
        }
    }

    #[test]
    fn fractions() {
        assert_eq!(parse_beats("1/3").unwrap(), 1.0 / 3.0);
        assert_eq!(
            ticks_from_beats(parse_beats("1/3").unwrap(), 480).unwrap(),
            160
        );
        assert!(parse_beats("-1").is_err());
        assert!(parse_beats("1/0").is_err());
    }
}
