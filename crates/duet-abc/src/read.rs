//! ABC → `Part`.
//!
//! Written from the ABC 2.1 standard, and from abcMIDI's documentation of how abc2midi
//! plays what the standard leaves open. The rules are the design's §3:
//!
//! - **Lengths** are exact fractions of a whole note, at 480 ticks per quarter. The unit is
//!   `L:`, or else the standard's default from `M:` (§3.1.7). Broken rhythm is the dotted
//!   reading (§4.4), and tuplets take the standard's default `q` (§4.13).
//! - **Bars** are checked against the meter. The one exception is a pickup: a first bar
//!   shorter than the meter, which becomes a short first bar of its own time signature.
//! - **Pitch:** an accidental lasts to the bar line, for its letter in every octave (the
//!   standard's default, `%%propagate-accidentals pitch`, §11.3). A tie carries its first
//!   note's pitch across a bar line.
//! - **Repeats** are unfolded, endings played on their passes.
//! - **Voices** become tracks, in order of first appearance, on channels 1, 2, 3 …
//!   skipping 10, unless `%%MIDI channel` says otherwise.
//! - **Velocity** is a level per dynamic, `!ppp!` 20 to `!fff!` 125, and 95 without one.

use std::fmt;

use duet_core::time::beats_from_ticks;
use duet_core::{
    Key, KeyChange, Note, Part, Pitch, TempoChange, TimeChange, TimeMap, TimeSig, Track,
};

use crate::error::{Error, Result};
use crate::lex::{Acc, BarTok, LineKind, NoteTok, Span, Tok};
use crate::parse::{Line, Tune};
use crate::ratio::Ratio;
use crate::subset::{self, refuse, refuse_with};

/// Ticks in a whole note: 480 ticks per quarter, as abc2midi writes.
pub(crate) const TICKS_PER_WHOLE: u64 = 1920;
pub(crate) const TPQ: u16 = 480;

/// How to read a tune. There are no choices yet; the type keeps room for them.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ReadOptions {}

/// A tune read as a part, with every notation-only construct it held.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Reading {
    pub part: Part,
    pub notices: Vec<Notice>,
}

/// A construct that was read but not carried into the part, because it is notation only:
/// a slur, an annotation, an information field and the like.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notice {
    pub line: usize,
    pub col: usize,
    /// The kind of construct, such as `slur` or `annotation`.
    pub construct: &'static str,
    /// The text that was found.
    pub found: String,
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, col {}: {} ({}) is notation only, and is not carried into the part",
            self.line, self.col, self.found, self.construct
        )
    }
}

/// The level each dynamic sets: abc2midi's own levels with beat accents off.
pub(crate) const DYNAMICS: [(&str, u8); 8] = [
    ("ppp", 20),
    ("pp", 35),
    ("p", 50),
    ("mp", 65),
    ("mf", 80),
    ("f", 95),
    ("ff", 110),
    ("fff", 125),
];
/// The channel a voice with no `%%MIDI channel` takes, given the channels the voices
/// before it are on: the lowest one none of them uses, skipping 10. This is how abc2midi
/// gives channels out, run with an explicit channel before, between and after.
pub(crate) fn default_channel(used: &[u8]) -> Option<u8> {
    (1..=16u8).find(|c| *c != 10 && !used.contains(c))
}

/// The level with no dynamic: `!f!`.
pub(crate) const DEFAULT_VELOCITY: u8 = 95;

/// Semitones above C of each natural letter, C to B.
pub(crate) const NATURAL: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Letters (C = 0) in the order a key signature adds sharps: F C G D A E B.
const SHARP_ORDER: [usize; 7] = [3, 0, 4, 1, 5, 2, 6];

/// The alteration a key signature gives each letter, C to B.
pub(crate) fn key_alterations(key: Key) -> [i8; 7] {
    let mut alt = [0i8; 7];
    let n = usize::from(key.sharps.unsigned_abs());
    if key.sharps > 0 {
        for &letter in &SHARP_ORDER[..n] {
            alt[letter] = 1;
        }
    } else {
        for &letter in SHARP_ORDER.iter().rev().take(n) {
            alt[letter] = -1;
        }
    }
    alt
}

/// A note of an event, its pitch worked out.
#[derive(Debug, Clone)]
struct EvNote {
    midi: u8,
    /// The letter's alteration, as written or as the key and bar give it.
    alter: i8,
    letter: u8,
    octave: i32,
    tie: bool,
    span: Span,
}

#[derive(Debug, Clone)]
struct Event {
    /// Empty for a rest.
    notes: Vec<EvNote>,
    /// In whole notes.
    len: Ratio,
    span: Span,
    text: String,
}

#[derive(Debug, Clone)]
struct BarItem {
    ends: usize,
    starts: usize,
    closing: bool,
    span: Span,
    text: String,
}

#[derive(Debug, Clone)]
enum Item {
    Event(Event),
    Bar(BarItem),
    Ending(Vec<u32>, Span),
    BarRest(u32, Span),
    Key(Key),
    Tempo(f64),
    Meter(TimeSig, Span),
    Dynamic(u8),
}

/// A chord being read: where it opened, its notes with their own lengths, and its text.
type OpenChord = (Span, Vec<(EvNote, Ratio)>, String);

#[derive(Debug, Clone)]
struct Voice {
    id: String,
    name: Option<String>,
    channel: Option<u8>,
    program: Option<u8>,
    items: Vec<Item>,
    has_music: bool,
    unit: Option<Ratio>,
    key: Key,
    meter: TimeSig,
    bar_acc: [Option<i8>; 7],
    /// Index of the last event, while nothing but spaces follows it.
    last_event: Option<usize>,
    pending_broken: Option<Ratio>,
    tuplet: Option<(Ratio, u32)>,
    /// The tied notes of the last event, waiting for the notes they tie to.
    tied: Vec<EvNote>,
    chord: Option<OpenChord>,
}

impl Voice {
    fn new(id: String) -> Voice {
        Voice {
            id,
            name: None,
            channel: None,
            program: None,
            items: Vec::new(),
            has_music: false,
            unit: None,
            key: Key {
                sharps: 0,
                mode: duet_core::Mode::Major,
            },
            meter: TimeSig::default(),
            bar_acc: [None; 7],
            last_event: None,
            pending_broken: None,
            tuplet: None,
            tied: Vec::new(),
            chord: None,
        }
    }
}

/// The state of reading one tune. The file header's lines are applied to a `Reader` once,
/// when the file is split, and each tune starts from a copy of it.
#[derive(Debug, Clone)]
pub(crate) struct Reader {
    notices: Vec<Notice>,
    title: Option<String>,
    meter: Option<TimeSig>,
    unit: Option<Ratio>,
    tempo: Option<f64>,
    key: Option<Key>,
    in_body: bool,
    voices: Vec<Voice>,
    current: Option<usize>,
    /// The first part-wide change written in the body, refused if the tune has several
    /// voices: (span, construct, text).
    body_change: Option<(Span, &'static str, String)>,
    last_field: Option<char>,
    beaming_noted: bool,
    header_program: Option<u8>,
    header_channel: Option<u8>,
    x_line: usize,
}

/// A MIDI note spelled with the alteration of the letter it was written on: `c` under
/// `K:Ebm` is Cb5, `^E` is E#, `^^F` is F##. Every note the reader makes is a letter and at
/// most two sharps or flats, so its spelling always exists.
fn spelled(midi: u8, alter: i8) -> Result<Pitch> {
    let pitch = Pitch::new(midi)?;
    Ok(pitch
        .spelled(alter)
        .unwrap_or_else(|| pitch.spelled_flat(alter < 0)))
}

fn parse_err(span: Span, msg: impl Into<String>) -> Error {
    Error::parse(span.line, span.col, msg)
}

/// `M:` (§3.1.6).
fn parse_meter(span: Span, value: &str) -> Result<TimeSig> {
    let v = value.trim();
    match v {
        "C" => return Ok(TimeSig::new(4, 4)?),
        "C|" => return Ok(TimeSig::new(2, 2)?),
        _ => {}
    }
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return Err(refuse(span, subset::FREE_METER, format!("M:{v}")));
    }
    if v.contains('+') || v.contains('(') {
        return Err(refuse(span, subset::COMPLEX_METER, format!("M:{v}")));
    }
    let bad = || parse_err(span, format!("'M:{v}' is not a meter like 6/8, C or C|"));
    let (num, den) = v.split_once('/').ok_or_else(bad)?;
    let num: u8 = num.trim().parse().map_err(|_| bad())?;
    let den: u8 = den.trim().parse().map_err(|_| bad())?;
    TimeSig::new(num, den).map_err(|e| parse_err(span, format!("'M:{v}': {e}")))
}

/// `L:` (§3.1.7).
fn parse_unit(span: Span, value: &str) -> Result<Ratio> {
    let v = value.trim();
    let bad = || parse_err(span, format!("'L:{v}' is not a unit length like 1/8"));
    let (num, den) = match v.split_once('/') {
        Some((n, d)) => (n.trim().parse::<u64>(), d.trim().parse::<u64>()),
        None => (v.parse::<u64>(), Ok(1)),
    };
    let (num, den) = (num.map_err(|_| bad())?, den.map_err(|_| bad())?);
    if num == 0 || den == 0 {
        return Err(bad());
    }
    Ok(Ratio::new(num, den))
}

/// `Q:` (§3.1.8), as quarter notes per minute.
fn parse_tempo(span: Span, value: &str) -> Result<f64> {
    // Remove any quoted text, before or after the tempo.
    let mut rest = String::new();
    let mut in_quote = false;
    let mut had_text = false;
    for c in value.chars() {
        if c == '"' {
            in_quote = !in_quote;
            had_text = true;
        } else if !in_quote {
            rest.push(c);
        }
    }
    let rest = rest.trim();
    let found = format!("Q:{}", value.trim());
    if rest.is_empty() {
        return Err(if had_text {
            refuse(span, subset::TEMPO_TEXT, found)
        } else {
            parse_err(span, "'Q:' needs a tempo such as 1/4=120")
        });
    }
    let Some((beats, bpm)) = rest.split_once('=') else {
        return Err(refuse_with(
            span,
            subset::UNIT_TEMPO,
            found,
            "the standard reads it as unit lengths per minute, abc2midi as quarters; write \
             Q:1/4=<n>",
        ));
    };
    // Q:C=120 and its kin, Q:C3=40: the unit length, so many to the minute (§10.1).
    let beats_text = beats.trim();
    if beats_text.starts_with(['C', 'c']) && beats_text[1..].chars().all(|c| c.is_ascii_digit()) {
        return Err(refuse_with(
            span,
            subset::UNIT_TEMPO,
            found,
            "the standard reads it as unit lengths per minute, abc2midi as quarters; write \
             Q:1/4=<n>",
        ));
    }
    let bad = || parse_err(span, format!("'{found}' is not a tempo like Q:1/4=120"));
    let bpm: u64 = bpm.trim().parse().map_err(|_| bad())?;
    let mut total = Ratio::ZERO;
    let units: Vec<&str> = beats.split_whitespace().collect();
    if units.is_empty() || units.len() > 4 || bpm == 0 {
        return Err(bad());
    }
    for unit in units {
        let (n, d) = unit.split_once('/').ok_or_else(bad)?;
        let n: u64 = n.parse().map_err(|_| bad())?;
        let d: u64 = d.parse().map_err(|_| bad())?;
        if n == 0 || d == 0 {
            return Err(bad());
        }
        total = total + Ratio::new(n, d);
    }
    Ok(bpm as f64 * total.as_f64() * 4.0)
}

fn is_clef_word(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    ["treble", "bass", "alto", "tenor", "perc", "none"]
        .iter()
        .any(|clef| {
            w.starts_with(clef)
                && w[clef.len()..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || "+-8".contains(c))
        })
}

/// Splits `name="Tenor I" clef=bass` into words, keeping quoted values whole.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in text.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

impl Reader {
    fn notice(&mut self, span: Span, construct: &'static str, found: impl Into<String>) {
        self.notices.push(Notice {
            line: span.line,
            col: span.col,
            construct,
            found: found.into(),
        });
    }

    /// `K:` (§3.1.14).
    fn parse_key(&mut self, span: Span, value: &str) -> Result<Key> {
        let v = value.trim();
        let found = format!("K:{v}");
        let ws = words(v);
        let Some(first) = ws.first() else {
            return Err(refuse(span, subset::KEY_NONE, found));
        };
        if first.eq_ignore_ascii_case("none") {
            return Err(refuse(span, subset::KEY_NONE, found));
        }
        if first == "HP" || first == "Hp" {
            return Err(refuse(span, subset::KEY_HP, found));
        }
        if !first.starts_with(|c: char| matches!(c, 'A'..='G')) {
            // A clef with no key, such as K:bass, or anything else.
            self.key_properties(span, &found, &ws)?;
            return Err(parse_err(span, format!("'{found}' names no key")));
        }
        // The tonic and mode may be one word (`F#Mix`, `Dm`) or two (`D dorian`).
        let tonic_len = 1 + first[1..]
            .chars()
            .take_while(|c| *c == '#' || *c == 'b')
            .count();
        let mut mode = first[tonic_len..].to_owned();
        let mut rest = &ws[1..];
        if mode.is_empty()
            && let Some(word) = rest.first()
            && duet_core::Mode::from_word(word).is_some()
        {
            mode = word.clone();
            rest = &rest[1..];
        }
        if !mode.is_empty() && duet_core::Mode::from_word(&mode).is_none() {
            return Err(parse_err(
                span,
                format!("'{mode}' in '{found}' is not a mode"),
            ));
        }
        let key: Key = format!("{} {mode}", &first[..tonic_len])
            .parse()
            .map_err(|e: duet_core::Error| parse_err(span, format!("'{found}': {e}")))?;
        self.key_properties(span, &found, rest)?;
        Ok(key)
    }

    /// What follows the key in `K:`, or the id in `V:`: accidentals, clefs and properties.
    fn key_properties(&mut self, span: Span, found: &str, ws: &[String]) -> Result<()> {
        for word in ws {
            let lower = word.to_ascii_lowercase();
            if lower.starts_with(['^', '_', '=']) || lower == "exp" {
                return Err(refuse(span, subset::KEY_EXPLICIT, found.to_owned()));
            }
            let prop = lower.split_once('=').map(|(k, _)| k.to_owned());
            match prop.as_deref() {
                Some("clef" | "transpose" | "t" | "octave") => {
                    return Err(refuse(span, subset::CLEF, word.clone()));
                }
                Some("middle" | "m" | "stafflines") => {
                    self.notice(span, subset::STAFF_PROPERTY, word.clone());
                }
                Some(_) => self.notice(span, subset::VOICE_PROPERTY, word.clone()),
                None if is_clef_word(word) => {
                    return Err(refuse(span, subset::CLEF, word.clone()));
                }
                None => {
                    return Err(parse_err(
                        span,
                        format!("'{word}' in '{found}' is not a key property"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The voice music goes to, created if the tune has none yet.
    fn music_voice(&mut self) -> usize {
        if let Some(i) = self.current {
            return i;
        }
        if self.voices.is_empty() {
            let voice = self.new_voice(String::new());
            self.voices.push(voice);
        }
        self.current = Some(0);
        0
    }

    fn new_voice(&self, id: String) -> Voice {
        let mut voice = Voice::new(id);
        if self.voices.is_empty() {
            voice.program = self.header_program;
            voice.channel = self.header_channel;
        }
        if self.in_body {
            voice.key = self.key.unwrap_or(voice.key);
            voice.meter = self.meter.unwrap_or_default();
        }
        voice
    }

    /// `V:` (§7), in the header or the body, or inline.
    fn voice_field(&mut self, span: Span, value: &str) -> Result<()> {
        let ws = words(value);
        let Some(id) = ws.first() else {
            return Err(parse_err(span, "'V:' needs a voice id"));
        };
        // §7: only the first 20 characters of an id tell voices apart.
        let id: String = id.chars().take(20).collect();
        let index = match self.voices.iter().position(|v| v.id == id) {
            Some(i) => i,
            None => {
                let voice = self.new_voice(id);
                self.voices.push(voice);
                self.voices.len() - 1
            }
        };
        self.current = Some(index);
        for word in &ws[1..] {
            let lower = word.to_ascii_lowercase();
            match lower.split_once('=').map(|(k, _)| k) {
                Some("name" | "nm") => {
                    let value = &word[word.find('=').map_or(0, |i| i + 1)..];
                    self.voices[index].name = Some(unquote(value));
                }
                Some("clef" | "transpose" | "t" | "octave") => {
                    return Err(refuse(span, subset::CLEF, word.clone()));
                }
                Some("middle" | "m" | "stafflines") => {
                    self.notice(span, subset::STAFF_PROPERTY, word.clone());
                }
                Some(_) => self.notice(span, subset::VOICE_PROPERTY, word.clone()),
                None if is_clef_word(word) => {
                    return Err(refuse(span, subset::CLEF, word.clone()));
                }
                None => self.notice(span, subset::VOICE_PROPERTY, word.clone()),
            }
        }
        Ok(())
    }

    /// A `%%` directive, or an `I:` field, which is the same thing (§3.1.17).
    fn directive(&mut self, span: Span, text: &str) -> Result<()> {
        let ws: Vec<&str> = text.split_whitespace().collect();
        let found = format!("%%{text}");
        match ws.as_slice() {
            ["MIDI", "program", n] | ["MIDI", "channel", n] => {
                let is_program = ws[1] == "program";
                let value: u8 = n
                    .parse()
                    .ok()
                    .filter(|v| {
                        if is_program {
                            *v <= 127
                        } else {
                            (1..=16).contains(v)
                        }
                    })
                    .ok_or_else(|| {
                        parse_err(
                            span,
                            if is_program {
                                format!("'{found}': a program is 0 to 127")
                            } else {
                                format!("'{found}': a channel is 1 to 16")
                            },
                        )
                    })?;
                match self.current {
                    Some(i) => {
                        let voice = &mut self.voices[i];
                        if voice.has_music {
                            return Err(refuse(span, subset::LATE_PROGRAM, found));
                        }
                        if is_program {
                            voice.program = Some(value);
                        } else {
                            voice.channel = Some(value);
                        }
                    }
                    None if is_program => self.header_program = Some(value),
                    None => self.header_channel = Some(value),
                }
            }
            ["MIDI", ..] => {
                return Err(refuse(span, subset::MIDI_OTHER, found));
            }
            ["propagate-accidentals", "pitch"] => {}
            ["propagate-accidentals", ..] => {
                return Err(refuse(span, subset::PROPAGATE, found));
            }
            _ => self.notice(span, subset::DIRECTIVE, found),
        }
        Ok(())
    }

    /// A field line, or an inline field in music code.
    fn field(&mut self, span: Span, letter: char, value: &str, inline: bool) -> Result<()> {
        let found = if inline {
            format!("[{letter}:{value}]")
        } else {
            format!("{letter}:{value}")
        };
        let previous = self.last_field.replace(letter);
        match letter {
            'T' if !self.in_body && self.title.is_none() => self.title = Some(value.to_owned()),
            'M' => {
                let sig = parse_meter(span, value)?;
                if self.in_body {
                    self.body_change
                        .get_or_insert((span, subset::MULTI_VOICE_CHANGE, found));
                    let v = self.music_voice();
                    self.voices[v].meter = sig;
                    self.voices[v].items.push(Item::Meter(sig, span));
                } else {
                    self.meter = Some(sig);
                }
            }
            'L' => {
                let unit = parse_unit(span, value)?;
                if self.in_body {
                    let v = self.music_voice();
                    self.voices[v].unit = Some(unit);
                } else {
                    self.unit = Some(unit);
                }
            }
            'Q' => {
                let bpm = parse_tempo(span, value)?;
                if self.in_body {
                    self.body_change
                        .get_or_insert((span, subset::MULTI_VOICE_CHANGE, found));
                    let v = self.music_voice();
                    self.voices[v].items.push(Item::Tempo(bpm));
                } else {
                    self.tempo = Some(bpm);
                }
            }
            'K' => {
                let key = self.parse_key(span, value)?;
                if self.in_body {
                    self.body_change
                        .get_or_insert((span, subset::MULTI_VOICE_KEY, found));
                    let v = self.music_voice();
                    self.voices[v].key = key;
                    self.voices[v].items.push(Item::Key(key));
                } else {
                    self.key = Some(key);
                    self.start_body(span)?;
                }
            }
            'V' => self.voice_field(span, value)?,
            'P' => return Err(refuse(span, subset::PARTS, found)),
            'w' => return Err(refuse(span, subset::LYRICS_ALIGNED, found)),
            'W' => return Err(refuse(span, subset::LYRICS_AFTER, found)),
            'm' => return Err(refuse(span, subset::MACRO, found)),
            'U' => return Err(refuse(span, subset::USER_SYMBOL, found)),
            's' => return Err(refuse(span, subset::SYMBOL_LINE, found)),
            'R' if value.trim().to_ascii_lowercase().starts_with("hornpipe") => {
                return Err(refuse(span, subset::HORNPIPE, found));
            }
            'r' => {}
            'I' => self.directive(span, value)?,
            'X' => return Err(parse_err(span, "a second X: field inside a tune")),
            '+' if previous == Some('T') && !self.in_body => {
                // §3.3: a continued string field gains a space between its halves.
                if let Some(title) = &mut self.title {
                    title.push(' ');
                    title.push_str(value);
                }
                self.last_field = Some('T');
            }
            _ => self.notice(span, subset::INFO_FIELD, found),
        }
        Ok(())
    }

    fn start_body(&mut self, span: Span) -> Result<()> {
        let Some(meter) = self.meter else {
            return Err(refuse(span, subset::FREE_METER, "no M: field"));
        };
        self.in_body = true;
        let key = self.key.unwrap_or(Key {
            sharps: 0,
            mode: duet_core::Mode::Major,
        });
        for voice in &mut self.voices {
            voice.key = key;
            voice.meter = meter;
        }
        // Music after the header goes to the first voice the header declared, if any.
        if !self.voices.is_empty() {
            self.current = Some(0);
        }
        Ok(())
    }

    /// The unit length in force for a voice: its own `L:`, the header's, or the default
    /// from the header's meter (§3.1.7).
    fn unit_of(&self, v: usize) -> Ratio {
        self.voices[v].unit.or(self.unit).unwrap_or_else(|| {
            let m = self.meter.unwrap_or_default();
            if u32::from(m.num) * 4 < 3 * u32::from(m.den) {
                Ratio::new(1, 16)
            } else {
                Ratio::new(1, 8)
            }
        })
    }

    fn note(&mut self, v: usize, span: Span, n: &NoteTok) -> Result<EvNote> {
        let voice = &mut self.voices[v];
        let letter = usize::from(n.letter);
        let explicit = match n.acc {
            Some(Acc::DoubleFlat) => Some(-2),
            Some(Acc::DoubleSharp) => Some(2),
            Some(Acc::Sharp) => Some(1),
            Some(Acc::Flat) => Some(-1),
            Some(Acc::Natural) => Some(0),
            None => None,
        };
        let (midi, alter) = if let Some(alter) = explicit {
            voice.bar_acc[letter] = Some(alter);
            (None, alter)
        } else if let Some(t) = voice
            .tied
            .iter()
            .find(|t| t.letter == n.letter && t.octave == n.octave)
        {
            // A tie carries the first note's pitch, accidental included, across a bar line.
            (Some(t.midi), t.alter)
        } else {
            let alter = voice.bar_acc[letter].unwrap_or(key_alterations(voice.key)[letter]);
            (None, alter)
        };
        let midi = match midi {
            Some(m) => i32::from(m),
            None => {
                let alter = explicit
                    .or(voice.bar_acc[letter])
                    .unwrap_or(key_alterations(voice.key)[letter]);
                60 + i32::from(NATURAL[letter]) + 12 * n.octave + i32::from(alter)
            }
        };
        let midi = u8::try_from(midi)
            .ok()
            .filter(|m| *m <= 127)
            .ok_or_else(|| parse_err(span, format!("'{}' is outside the MIDI range", n.text)))?;
        Ok(EvNote {
            midi,
            alter,
            letter: n.letter,
            octave: n.octave,
            tie: false,
            span,
        })
    }

    /// Adds an event (a note, chord or rest) to a voice, applying broken rhythm and any
    /// tuplet in force.
    fn event(&mut self, v: usize, span: Span, notes: Vec<EvNote>, written: Ratio, text: String) {
        let unit = self.unit_of(v);
        let voice = &mut self.voices[v];
        let mut len = written * unit;
        if let Some(factor) = voice.pending_broken.take() {
            len = len * factor;
        }
        if let Some((factor, left)) = voice.tuplet {
            len = len * factor;
            voice.tuplet = (left > 1).then_some((factor, left - 1));
        }
        // Notes tied inside a chord, as in [C-E], wait for the notes they tie to.
        voice.tied = notes.iter().filter(|n| n.tie).cloned().collect();
        voice.has_music = true;
        voice.items.push(Item::Event(Event {
            notes,
            len,
            span,
            text,
        }));
        voice.last_event = Some(voice.items.len() - 1);
    }

    fn music(&mut self, tokens: &[(Span, Tok)]) -> Result<()> {
        if !self.in_body {
            let span = tokens.first().map_or(Span { line: 0, col: 0 }, |(s, _)| *s);
            return Err(parse_err(
                span,
                "music before the K: field that ends the header",
            ));
        }
        for (span, tok) in tokens {
            let span = *span;
            let v = self.music_voice();
            // Broken rhythm attaches to the event just before it, across spaces only.
            if !matches!(
                tok,
                Tok::Space | Tok::Broken { .. } | Tok::Tie | Tok::Continuation
            ) {
                self.voices[v].last_event = None;
            }
            match tok {
                Tok::Note(n) => {
                    let note = self.note(v, span, n)?;
                    if let Some((_, notes, text)) = &mut self.voices[v].chord {
                        notes.push((note, n.len));
                        text.push_str(&n.text);
                    } else {
                        self.event(v, span, vec![note], n.len, n.text.clone());
                    }
                }
                Tok::Rest { len } => {
                    if self.voices[v].chord.is_some() {
                        return Err(parse_err(span, "a rest inside a chord"));
                    }
                    self.event(v, span, Vec::new(), *len, "z".to_owned());
                }
                Tok::BarRest { bars } => {
                    self.voices[v].has_music = true;
                    self.voices[v].items.push(Item::BarRest(*bars, span));
                }
                Tok::Spacer => self.notice(span, subset::SPACER, "y"),
                Tok::ChordStart => {
                    if self.voices[v].chord.is_some() {
                        return Err(parse_err(span, "a chord inside a chord"));
                    }
                    self.voices[v].chord = Some((span, Vec::new(), "[".to_owned()));
                }
                Tok::ChordEnd { len } => {
                    let Some((start, notes, mut text)) = self.voices[v].chord.take() else {
                        return Err(parse_err(span, "']' closes no chord"));
                    };
                    text.push(']');
                    let Some((_, inner)) = notes.first() else {
                        return Err(parse_err(start, "an empty chord"));
                    };
                    let inner = *inner;
                    if notes.iter().any(|(_, l)| *l != inner) {
                        return Err(refuse(start, subset::MIXED_CHORD, text));
                    }
                    let notes = notes.into_iter().map(|(n, _)| n).collect();
                    self.event(v, start, notes, inner * *len, text);
                }
                Tok::Tie => {
                    let voice = &mut self.voices[v];
                    if let Some((_, notes, _)) = &mut voice.chord {
                        match notes.last_mut() {
                            Some((n, _)) => n.tie = true,
                            None => return Err(parse_err(span, "a tie with no note before it")),
                        }
                    } else {
                        let Some(Item::Event(e)) = voice.items.last_mut() else {
                            return Err(parse_err(span, "a tie with no note before it"));
                        };
                        if e.notes.is_empty() {
                            return Err(parse_err(span, "a rest cannot be tied"));
                        }
                        for n in &mut e.notes {
                            n.tie = true;
                        }
                        voice.tied = e.notes.clone();
                    }
                }
                Tok::Broken { left_long, count } => {
                    let voice = &mut self.voices[v];
                    let Some(i) = voice.last_event else {
                        return Err(parse_err(span, "broken rhythm needs a note before it"));
                    };
                    let half = Ratio::new(1, 1 << count);
                    let long = Ratio::new((2 << count) - 1, 1 << count);
                    let (before, after) = if *left_long {
                        (long, half)
                    } else {
                        (half, long)
                    };
                    if let Item::Event(e) = &mut voice.items[i] {
                        e.len = e.len * before;
                    }
                    voice.pending_broken = Some(after);
                }
                Tok::Tuplet { p, q, r } => {
                    let p = *p;
                    if p > 9 {
                        return Err(refuse(span, subset::BIG_TUPLET, format!("({p}")));
                    }
                    if p < 2 {
                        return Err(parse_err(span, format!("'({p}' is not a tuplet")));
                    }
                    let meter = self.voices[v].meter;
                    let compound = meter.den == 8 && matches!(meter.num, 6 | 9 | 12);
                    // §4.13's table.
                    let default_q = match p {
                        2 | 4 | 8 => 3,
                        3 | 6 => 2,
                        _ if compound => 3,
                        _ => 2,
                    };
                    let q = q.unwrap_or(default_q);
                    let r = r.unwrap_or(p);
                    if q == 0 || r == 0 {
                        return Err(parse_err(span, "a tuplet's q and r are 1 or more"));
                    }
                    self.voices[v].tuplet = Some((Ratio::new(u64::from(q), u64::from(p)), r));
                }
                Tok::SlurStart => self.notice(span, subset::SLUR, "("),
                Tok::SlurEnd => self.notice(span, subset::SLUR, ")"),
                Tok::Bar(b) => self.bar(v, span, b)?,
                Tok::Ending(passes) => {
                    self.voices[v]
                        .items
                        .push(Item::Ending(passes.clone(), span));
                }
                Tok::InlineField { letter, value } => {
                    if *letter == 'V' {
                        self.voice_field(span, value)?;
                    } else {
                        self.field(span, *letter, value, true)?;
                    }
                }
                Tok::Decoration(name) => {
                    if let Some((_, vel)) = DYNAMICS.iter().find(|(d, _)| d == name) {
                        self.voices[v].items.push(Item::Dynamic(*vel));
                    } else if [
                        "crescendo(",
                        "crescendo)",
                        "<(",
                        "<)",
                        "diminuendo(",
                        "diminuendo)",
                        ">(",
                        ">)",
                    ]
                    .contains(&name.as_str())
                    {
                        return Err(refuse(span, subset::CRESCENDO, format!("!{name}!")));
                    } else {
                        let found = if name.chars().count() == 1 || name.starts_with('+') {
                            name.clone()
                        } else {
                            format!("!{name}!")
                        };
                        return Err(refuse_with(
                            span,
                            subset::DECORATION,
                            found,
                            "each changes playback in abc2midi; dynamics (!ppp! to !fff!) are \
                             the one family read",
                        ));
                    }
                }
                Tok::Annotation(text) => {
                    self.notice(span, subset::ANNOTATION, format!("\"{text}\""))
                }
                Tok::ChordSymbol(text) => {
                    return Err(refuse(span, subset::CHORD_SYMBOL, format!("\"{text}\"")));
                }
                Tok::Grace(text) => return Err(refuse(span, subset::GRACE, text.clone())),
                Tok::Overlay => return Err(refuse(span, subset::OVERLAY, "&")),
                Tok::OldChord(text) => return Err(refuse(span, subset::OLD_CHORD, text.clone())),
                Tok::Space => {
                    if !self.beaming_noted {
                        self.beaming_noted = true;
                        self.notice(span, subset::BEAMING, "a space");
                    }
                }
                Tok::Continuation => {}
                Tok::Reserved(c) => self.notice(span, subset::RESERVED, c.to_string()),
            }
        }
        Ok(())
    }

    fn bar(&mut self, v: usize, span: Span, b: &BarTok) -> Result<()> {
        let voice = &mut self.voices[v];
        if voice.chord.is_some() {
            return Err(parse_err(span, "a bar line inside a chord"));
        }
        voice.bar_acc = [None; 7];
        voice.items.push(Item::Bar(BarItem {
            ends: b.ends,
            starts: b.starts,
            closing: b.closing,
            span,
            text: b.text.clone(),
        }));
        Ok(())
    }
}

/// A note laid out in ticks.
struct RawNote {
    start: u64,
    len: u64,
    midi: u8,
    alter: i8,
    vel: u8,
}

/// One voice laid out: its notes, its first bar if that is a pickup, and its length.
struct Laid {
    notes: Vec<RawNote>,
    pickup: Option<(u64, Span)>,
    total: u64,
    meters: Vec<(u64, TimeSig)>,
    keys: Vec<(u64, Key)>,
    tempos: Vec<(u64, f64)>,
}

fn whole(ticks: u64) -> Ratio {
    Ratio::new(ticks, TICKS_PER_WHOLE)
}

fn bar_ticks(sig: TimeSig) -> u64 {
    TICKS_PER_WHOLE * u64::from(sig.num) / u64::from(sig.den)
}

/// The order a voice's items play in, with repeats unfolded and endings played on their
/// passes (§4.8 to §4.10). `restart` is where a repeat with no start goes back to: the
/// start of the tune, or the first bar after a pickup.
fn unfold(items: &[Item], restart: usize, multi_voice: bool) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start = restart;
    let mut pass = 1u32;
    let mut open = false;
    // Whether the music now playing is an ending (`[1`, `|2`) of the current repeat.
    let mut in_ending = false;
    let mut steps = 0usize;
    while i < items.len() {
        steps += 1;
        if steps > 4 * items.len() + 16 {
            return Err(Error::invalid("the tune's repeats never end"));
        }
        match &items[i] {
            Item::Bar(b) => {
                out.push(i);
                if b.ends > 1 || b.starts > 1 {
                    return Err(refuse(b.span, subset::TRIPLE_REPEAT, b.text.clone()));
                }
                if b.ends == 1 {
                    if in_ending && pass >= 2 {
                        // A :| closing the second ending asks for a third pass, which
                        // abc2midi plays; the standard ties more than two passes to P:.
                        return Err(refuse(b.span, subset::TRIPLE_REPEAT, b.text.clone()));
                    }
                    if !open && multi_voice {
                        return Err(refuse(b.span, subset::MULTI_VOICE_REPEAT, b.text.clone()));
                    }
                    if pass == 1 {
                        pass = 2;
                        in_ending = false;
                        i = start;
                        continue;
                    }
                    pass = 1;
                    open = false;
                    in_ending = false;
                    start = i + 1;
                }
                if b.starts == 1 {
                    if open {
                        return Err(refuse(b.span, subset::NESTED_REPEAT, b.text.clone()));
                    }
                    open = true;
                    in_ending = false;
                    start = i + 1;
                    pass = 1;
                } else if b.closing && b.ends == 0 && in_ending {
                    // A double or thick bar ends the last ending. Anywhere else it is a bar
                    // line like any other: a later :| still goes back past it, as abc2midi
                    // plays it.
                    pass = 1;
                    in_ending = false;
                }
                i += 1;
            }
            Item::Ending(passes, span) => {
                if passes.iter().all(|p| *p > 2) {
                    let list: Vec<String> = passes.iter().map(u32::to_string).collect();
                    return Err(refuse(
                        *span,
                        subset::TRIPLE_REPEAT,
                        format!("ending {}", list.join(",")),
                    ));
                }
                if passes.contains(&pass) {
                    in_ending = true;
                    i += 1;
                    continue;
                }
                // Skip this ending: to the next ending, or past the repeat that ends it.
                let mut j = i + 1;
                i = loop {
                    match items.get(j) {
                        None | Some(Item::Ending(..)) => break j,
                        Some(Item::Bar(b)) if b.ends > 0 => {
                            // The repeat this ending closes is done: the pass stays, so the
                            // next ending plays, and a later :| goes back no further.
                            open = false;
                            start = j + 1;
                            break j + 1;
                        }
                        Some(Item::Bar(b)) if b.closing => break j,
                        _ => j += 1,
                    }
                };
            }
            _ => {
                out.push(i);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Closes the bar from `bar_start` to `tick`, checking its length against the meter. A
/// first bar shorter than the meter is a pickup; an empty bar is no bar.
fn close_bar(
    tick: u64,
    bar_start: &mut u64,
    bars_closed: &mut u64,
    meter: TimeSig,
    span: Span,
    pickup: &mut Option<(u64, Span)>,
) -> Result<()> {
    let len = tick - *bar_start;
    if len == 0 {
        return Ok(());
    }
    let full = bar_ticks(meter);
    if *bars_closed == 0 && len < full {
        *pickup = Some((len, span));
    } else if len != full {
        return Err(parse_err(
            span,
            format!(
                "bar {} is {} of a whole note long, but a bar of {meter} is {}",
                *bars_closed + 1,
                whole(len),
                whole(full)
            ),
        ));
    }
    *bars_closed += 1;
    *bar_start = tick;
    Ok(())
}

/// Lays a voice's items out in ticks, checking each bar against the meter.
fn lay_out(items: &[Item], order: &[usize], meter: TimeSig, x_line: usize) -> Result<Laid> {
    let mut laid = Laid {
        notes: Vec::new(),
        pickup: None,
        total: 0,
        meters: Vec::new(),
        keys: Vec::new(),
        tempos: Vec::new(),
    };
    let mut tick = 0u64;
    let mut bar_start = 0u64;
    let mut bars_closed = 0u64;
    let mut meter = meter;
    let mut vel = DEFAULT_VELOCITY;
    // Tied notes waiting for the next event: (midi, index in laid.notes, span).
    let mut pending: Vec<(u8, usize, Span)> = Vec::new();

    for &i in order {
        match &items[i] {
            Item::Event(e) => {
                let len = e.len.ticks(TICKS_PER_WHOLE).ok_or_else(|| {
                    refuse_with(
                        e.span,
                        subset::ODD_LENGTH,
                        e.text.clone(),
                        format!("it is {} of a whole note", e.len),
                    )
                })?;
                let mut matched = vec![false; pending.len()];
                let mut next = Vec::new();
                for n in &e.notes {
                    let found = pending
                        .iter()
                        .enumerate()
                        .position(|(k, (midi, _, _))| *midi == n.midi && !matched[k]);
                    let index = match found {
                        Some(k) => {
                            matched[k] = true;
                            let index = pending[k].1;
                            laid.notes[index].len += len;
                            index
                        }
                        None => {
                            laid.notes.push(RawNote {
                                start: tick,
                                len,
                                midi: n.midi,
                                alter: n.alter,
                                vel,
                            });
                            laid.notes.len() - 1
                        }
                    };
                    if n.tie {
                        next.push((n.midi, index, n.span));
                    }
                }
                if let Some(k) = matched.iter().position(|m| !m) {
                    return Err(parse_err(
                        pending[k].2,
                        "this tie is followed by no note of the same pitch",
                    ));
                }
                pending = next;
                tick += len;
            }
            Item::Bar(b) => close_bar(
                tick,
                &mut bar_start,
                &mut bars_closed,
                meter,
                b.span,
                &mut laid.pickup,
            )?,
            Item::BarRest(bars, span) => {
                if tick != bar_start {
                    return Err(parse_err(*span, "a multi-bar rest must start a bar"));
                }
                if !pending.is_empty() {
                    return Err(parse_err(pending[0].2, "this tie is followed by a rest"));
                }
                tick = tick.saturating_add(u64::from(*bars).saturating_mul(bar_ticks(meter)));
                bars_closed = bars_closed.saturating_add(u64::from(*bars) - 1);
                bar_start = tick - bar_ticks(meter);
            }
            Item::Meter(sig, span) => {
                if tick != bar_start {
                    return Err(parse_err(*span, "a meter change inside a bar"));
                }
                meter = *sig;
                laid.meters.push((tick, *sig));
            }
            Item::Key(k) => laid.keys.push((tick, *k)),
            Item::Tempo(bpm) => laid.tempos.push((tick, *bpm)),
            Item::Dynamic(v) => vel = *v,
            Item::Ending(..) => {}
        }
    }
    let end = Span {
        line: x_line,
        col: 1,
    };
    if let Some((_, _, span)) = pending.first() {
        return Err(parse_err(*span, "this tie is followed by no note"));
    }
    close_bar(
        tick,
        &mut bar_start,
        &mut bars_closed,
        meter,
        end,
        &mut laid.pickup,
    )?;
    laid.total = tick;
    Ok(laid)
}

/// The time signature a pickup of `len` ticks becomes: its length over the meter's
/// denominator, or a finer one when the pickup needs it.
fn pickup_sig(len: u64, meter: TimeSig, span: Span) -> Result<TimeSig> {
    pickup_meter(whole(len), meter).ok_or_else(|| {
        refuse_with(
            span,
            subset::PICKUP,
            format!("a pickup of {} of a whole note", whole(len)),
            "a pickup is a short first bar, so its length must be a time signature",
        )
    })
}

/// The meter a pickup `len` long (in whole notes) gets under `meter`: its length over the
/// meter's denominator, or a finer one when it needs it. The writer uses the pickup form
/// only when this gives back the part's first bar.
pub(crate) fn pickup_meter(len: Ratio, meter: TimeSig) -> Option<TimeSig> {
    let den = len.den().max(u64::from(meter.den));
    let num = len.num() * den / len.den();
    let fits = den <= 64 && den.is_power_of_two() && (1..=255).contains(&num);
    fits.then(|| TimeSig::new(num as u8, den as u8).ok())
        .flatten()
}

/// The time signature a pickup `len_ticks` long gets before a first full bar of `meter`, at
/// `ticks_per_quarter`: its length over the meter's denominator, or over a finer one when it
/// needs it, so a quarter before 4/4 is 1/4 and a quarter before 6/8 is 2/8.
///
/// `None` when the length is no time signature: a length of zero, one that is not a whole
/// number of 64ths, or one that would need more than 255 beats.
///
/// This is the rule the writer applies: a part whose first bar has this signature, followed
/// by a change to `meter` at bar 2, is written as a pickup before the first bar line. Anything
/// that builds a part with a pickup uses it, so the two cannot drift apart.
pub fn pickup_meter_for(len_ticks: u64, ticks_per_quarter: u16, meter: TimeSig) -> Option<TimeSig> {
    pickup_meter(
        Ratio::new(len_ticks, 4 * u64::from(ticks_per_quarter)),
        meter,
    )
}

/// The key an ABC key name gives, read exactly as the value of a `K:` field is: `Eb`, `Bm`,
/// `F#m`, `D dorian`. Anything that names a key from outside, such as a command line, reads it
/// here, so it means what `K:` means in a tune.
///
/// Fails, as `K:` does, on a name that is no key (`K:none`, a clef alone), and on anything
/// after the key, such as a clef or a property: a name is only a key.
pub fn key_from_name(name: &str) -> Result<Key> {
    let mut reader = Reader::new();
    let at = Span { line: 1, col: 1 };
    let key = reader.parse_key(at, name).map_err(without_position)?;
    if let Some(notice) = reader.notices.first() {
        return Err(Error::invalid(format!(
            "'{name}' holds '{}' after the key; a key name is only a key",
            notice.found
        )));
    }
    Ok(key)
}

/// An error about a name given on its own, without the line and column that only mean
/// something in a file. The text is the error's own, so it says what the error says.
fn without_position(e: Error) -> Error {
    let (Error::Parse { line, col, .. } | Error::Unsupported { line, col, .. }) = e else {
        return e;
    };
    let text = e.to_string();
    let at = format!("line {line}, col {col}: ");
    Error::Invalid(text.strip_prefix(&at).unwrap_or(&text).to_owned())
}

impl Tune {
    /// Reads the tune as a part, with the notation-only constructs it held.
    ///
    /// A construct outside the subset is refused by name ([`Error::Unsupported`]), and a
    /// tune that is not valid ABC, or has a bar of the wrong length, fails with its line
    /// and column ([`Error::Parse`]). Nothing is moved, rounded or dropped to make it fit.
    pub fn to_part(&self, options: &ReadOptions) -> Result<Reading> {
        self.read(options, true)
    }

    /// Reads the tune, with the file header's notices or without them.
    pub(crate) fn read(
        &self,
        _options: &ReadOptions,
        file_header_notices: bool,
    ) -> Result<Reading> {
        let (mut r, header_notices) = match &*self.header {
            Ok(h) => (h.state.clone(), &h.notices),
            Err(e) => return Err(e.clone()),
        };
        if file_header_notices {
            r.notices = header_notices.clone();
        }
        r.x_line = self.line();
        for line in self.classified()? {
            r.line(&line)?;
        }
        if !r.in_body {
            return Err(Error::parse(
                r.x_line,
                1,
                "the tune's header never ends: a header ends with a K: field",
            ));
        }
        r.into_part()
    }
}

/// A file header, applied once: the state every tune in the file starts from, and the
/// notices its lines gave, kept apart so that a tune copies only the state.
#[derive(Debug)]
pub(crate) struct FileHeader {
    state: Reader,
    notices: Vec<Notice>,
}

/// Applies a file header's lines once, for every tune in the file to start from (§2.2.2).
pub(crate) fn read_file_header(lines: &[Line]) -> Result<FileHeader> {
    // The fields the standard allows in a file header (§3, its table's first column), plus
    // `+:` continuing one of them. A tune's own fields (T:, V:, K:, Q:, P:, the lyrics) are
    // refused there: each tune starts from this state, so it holds only small defaults.
    const ALLOWED: &str = "ABCDFGHILMmNORrSUZ+";
    let mut state = Reader::new();
    for line in lines {
        if let LineKind::Field { letter, .. } = &line.kind
            && !ALLOWED.contains(*letter)
        {
            return Err(Error::parse(
                line.number,
                1,
                format!(
                    "'{letter}:' is not allowed in a file header; it belongs in a tune's \
                     header (§3)"
                ),
            ));
        }
        state.line(line)?;
    }
    let notices = std::mem::take(&mut state.notices);
    Ok(FileHeader { state, notices })
}

impl Reader {
    fn new() -> Reader {
        Reader {
            notices: Vec::new(),
            title: None,
            meter: None,
            unit: None,
            tempo: None,
            key: None,
            in_body: false,
            voices: Vec::new(),
            current: None,
            body_change: None,
            last_field: None,
            beaming_noted: false,
            header_program: None,
            header_channel: None,
            x_line: 0,
        }
    }

    fn line(&mut self, line: &Line) -> Result<()> {
        let span = Span {
            line: line.number,
            col: 1,
        };
        match &line.kind {
            LineKind::Field { letter: 'X', .. } => {}
            LineKind::Field { letter, value } => self.field(span, *letter, value, false)?,
            LineKind::Directive(text) => self.directive(span, text)?,
            LineKind::Music(tokens) => self.music(tokens)?,
            LineKind::Nothing => {}
        }
        Ok(())
    }
}

impl Reader {
    fn into_part(self) -> Result<Reading> {
        let header_meter = self.meter.unwrap_or_default();
        let voices: Vec<&Voice> = self.voices.iter().filter(|v| v.has_music).collect();
        let multi_voice = voices.len() > 1;
        if multi_voice && let Some((span, construct, found)) = &self.body_change {
            return Err(refuse(*span, construct, found.clone()));
        }
        if let Some(v) = voices.iter().find(|v| v.chord.is_some()) {
            let span = v.chord.as_ref().map_or(Span { line: 0, col: 0 }, |c| c.0);
            return Err(parse_err(span, "this chord is never closed"));
        }

        let mut laid = Vec::new();
        for voice in &voices {
            // A repeat with no start goes back to the start, or past a pickup.
            let first_bar = voice.items.iter().position(|i| matches!(i, Item::Bar(_)));
            let opening = &voice.items[..first_bar.unwrap_or(0)];
            let before = opening.iter().fold(Ratio::ZERO, |sum, item| match item {
                Item::Event(e) => sum + e.len,
                _ => sum,
            });
            let full = whole(bar_ticks(header_meter));
            let has_pickup = first_bar.is_some()
                && !opening.iter().any(|i| matches!(i, Item::BarRest(..)))
                && before != Ratio::ZERO
                && before.as_f64() < full.as_f64();
            let restart = if has_pickup {
                first_bar.map_or(0, |b| b + 1)
            } else {
                0
            };
            let order = unfold(&voice.items, restart, multi_voice)?;
            laid.push(lay_out(&voice.items, &order, header_meter, self.x_line)?);
        }

        // Every voice must start the same way: with the same pickup, or none.
        let pickup = laid.first().and_then(|l| l.pickup);
        for (voice, l) in voices.iter().zip(&laid).skip(1) {
            if l.pickup.map(|p| p.0) != pickup.map(|p| p.0) {
                let span = l.pickup.or(pickup).map_or(
                    Span {
                        line: self.x_line,
                        col: 1,
                    },
                    |p| p.1,
                );
                return Err(parse_err(
                    span,
                    format!(
                        "voice '{}' starts with a first bar of a different length from the \
                         first voice's",
                        voice.id
                    ),
                ));
            }
        }

        let initial_sig = match pickup {
            Some((len, span)) => pickup_sig(len, header_meter, span)?,
            None => header_meter,
        };
        let mut map = TimeMap::new(TPQ, initial_sig)?;
        let mut time_changes: Vec<(u64, TimeChange)> = Vec::new();
        let mut add_meter = |map: &mut TimeMap, tick: u64, sig: TimeSig| -> Result<()> {
            let at = map.change_at_tick(tick, sig)?;
            match time_changes.last_mut() {
                Some((t, change)) if *t == tick => change.time_sig = sig,
                _ => time_changes.push((tick, TimeChange { at, time_sig: sig })),
            }
            Ok(())
        };
        if let Some((len, _)) = pickup {
            add_meter(&mut map, len, header_meter)?;
        }
        if let Some(first) = laid.first() {
            for &(tick, sig) in &first.meters {
                add_meter(&mut map, tick, sig)?;
            }
        }

        let mut part = Part::default();
        part.title = self.title.clone();
        part.ticks_per_quarter = TPQ;
        part.tempo_bpm = self.tempo.unwrap_or(120.0);
        part.time_sig = initial_sig;
        part.key = self.key;
        part.time_changes = time_changes.into_iter().map(|(_, c)| c).collect();
        if let Some(first) = laid.first() {
            part.key_changes = first
                .keys
                .iter()
                .map(|&(tick, key)| KeyChange {
                    at: map.position(tick),
                    key,
                })
                .collect();
            part.tempo_changes = first
                .tempos
                .iter()
                .map(|&(tick, bpm)| TempoChange {
                    at: map.position(tick),
                    bpm,
                })
                .collect();
        }

        let mut used: Vec<u8> = Vec::new();
        for (voice, l) in voices.iter().zip(&laid) {
            let name = match (&voice.name, voice.id.is_empty()) {
                (Some(name), _) => name.clone(),
                (None, false) => voice.id.clone(),
                (None, true) => self.title.clone().unwrap_or_else(|| "Tune".to_owned()),
            };
            let channel = match voice.channel {
                Some(c) => c,
                None => default_channel(&used).ok_or_else(|| {
                    Error::invalid("the tune has more voices than channels off the drum channel")
                })?,
            };
            used.push(channel);
            let mut track = Track::new(name, channel);
            track.program = voice.program;
            for n in &l.notes {
                track.notes.push(Note::new(
                    map.position(n.start),
                    spelled(n.midi, n.alter)?,
                    beats_from_ticks(n.len, TPQ),
                    n.vel,
                ));
            }
            part.tracks.push(track);
        }
        let total = laid.iter().map(|l| l.total).max().unwrap_or(0);
        part.end = Some(map.position(total));
        part.normalize()?;
        Ok(Reading {
            part,
            notices: self.notices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_name_reads_as_k_reads_it() {
        for (name, want) in [
            ("Eb", "Eb major"),
            ("Bm", "B minor"),
            ("F#m", "F# minor"),
            ("D dorian", "D dorian"),
            ("Amin", "A minor"),
        ] {
            assert_eq!(key_from_name(name).unwrap().to_string(), want, "{name}");
        }
    }

    #[test]
    fn a_name_that_is_no_key_is_refused_without_a_position() {
        for name in ["none", "bass", "", "Eb clef=bass", "Eb middle=c", "Hx"] {
            let err = key_from_name(name).unwrap_err().to_string();
            assert!(!err.contains("line 1"), "{name}: {err}");
        }
    }

    #[test]
    fn a_refused_key_says_what_the_refusal_says() {
        let in_a_tune = crate::Book::parse("X:1\nM:4/4\nK:none\nC|\n")
            .unwrap()
            .tunes()[0]
            .to_part(&ReadOptions::default())
            .unwrap_err()
            .to_string();
        let alone = key_from_name("none").unwrap_err().to_string();
        assert!(in_a_tune.ends_with(&alone), "{in_a_tune} / {alone}");
        assert!(alone.starts_with("K:none is refused"), "{alone}");
    }
}
