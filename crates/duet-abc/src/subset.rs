//! The v1 subset's edges: every construct refused by name, and every construct accepted
//! as notation only.
//!
//! A refused construct is valid ABC that this version does not read. Each has a stable
//! name, which is the `construct` of [`Error::Unsupported`](crate::Error::Unsupported), and
//! a reason. A notation-only construct is read and dropped: it changes nothing a `Song`
//! holds, and [`check`](crate::check) lists every one with its line.

use crate::error::Error;
use crate::lex::Span;

/// A construct this version refuses, with why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refusal {
    /// The construct's stable name.
    pub construct: &'static str,
    /// Why this version does not read it.
    pub why: &'static str,
}

macro_rules! refusals {
    ($($id:ident = $name:literal : $why:literal;)*) => {
        $(pub(crate) const $id: &str = $name;)*
        /// Every construct refused by name, in the order of the design's table.
        pub const REFUSED: &[Refusal] = &[$(Refusal { construct: $name, why: $why },)*];
    };
}

refusals! {
    GRACE = "grace notes":
        "they have no notated length, and abc2midi takes their time from the next note";
    DECORATION = "decoration":
        "each changes playback in abc2midi; dynamics are the one family read";
    CRESCENDO = "crescendo / diminuendo":
        "abc2midi ramps the velocity; a v2 candidate";
    CHORD_SYMBOL = "chord symbol":
        "abc2midi turns chord symbols into accompaniment notes";
    LYRICS_ALIGNED = "lyrics (w:)":
        "a Song has no lyrics yet";
    LYRICS_AFTER = "lyrics (W:)":
        "a Song has no lyrics yet";
    HORNPIPE = "R:hornpipe":
        "abc2midi plays it swung";
    PARTS = "parts (P:)":
        "parts need their own unfolding, and abc2midi turns its repeat repair off with them";
    NESTED_REPEAT = "nested repeat":
        "abc2midi does not support it";
    MULTI_VOICE_REPEAT = "repeat without a start in a multi-voice tune":
        "abc2midi does not support it";
    FREE_METER = "free meter (M:none, or no M:)":
        "a Song has no free meter; the standard reads a tune with no M: as free meter (§3.1.6)";
    UNIT_TEMPO = "Q: unit-length tempo":
        "deprecated: the standard reads it as unit lengths per minute (§10.1), abc2midi as quarters";
    PROPAGATE = "%%propagate-accidentals <not|octave>":
        "this version reads only the standard's default, pitch (§11.3)";
    PICKUP = "pickup of a length no meter can hold":
        "a pickup is a short first bar, so its length must be a time signature";
    KEY_NONE = "K:none":
        "a v2 candidate";
    KEY_HP = "K:HP":
        "a v2 candidate";
    KEY_EXPLICIT = "K:<key> <explicit accidentals>":
        "a v2 candidate; the standard lets software mark the notes instead (§3.1.14)";
    CLEF = "clef, transpose or octave property":
        "each changes pitch on playback";
    MULTI_VOICE_KEY = "[K:] inside a multi-voice tune":
        "it is per-voice in ABC, but song-wide in a Song";
    MACRO = "macro (m:)":
        "text substitution, a v2 candidate";
    USER_SYMBOL = "user symbol (U:)":
        "text substitution, a v2 candidate";
    MIDI_OTHER = "%%MIDI <other>":
        "each changes playback in its own way; only %%MIDI program n and %%MIDI channel n are read";
    MIXED_CHORD = "chord of mixed lengths":
        "abc2midi takes the first note's length, and the writer has no form for it";
    BIG_TUPLET = "tuplet p>9":
        "it cannot be exact in ticks";
    ODD_LENGTH = "length that is not a whole tick at 480 ticks per quarter":
        "it cannot be exact in ticks";
    ENHARMONIC = "double sharp or flat <^^/__>":
        "a Pitch spells at most one sharp or flat; E#, B#, Cb and Fb are read";
    TEMPO_TEXT = "Q: text only":
        "it has no number";
    OLD_CHORD = "old chord syntax +CEG+":
        "it is deprecated, and clashes with +deco+";
    MULTI_VOICE_CHANGE = "[M:] or [Q:] inside a multi-voice tune":
        "a Song's meter and tempo are song-wide; the standard repeats inline fields per voice (§7.3)";
    OVERLAY = "voice overlay (&)":
        "a Song track holds one line of notes or chords; splitting into voices is a v2 candidate";
    SYMBOL_LINE = "symbol line (s:)":
        "it carries decorations and chord symbols, which are refused";
    COMPLEX_METER = "complex meter (M:(2+3+2)/8)":
        "a v2 candidate; write the meter's total, such as M:7/8";
    TRIPLE_REPEAT = "repeat played more than twice (|::, ::|, or a third ending)":
        "abc2midi plays a third pass; the standard ties more than two endings to P: (§4.10), a v2 candidate";
    LATE_PROGRAM = "%%MIDI program or channel after a voice's first note":
        "a Song track has one program and channel from its start";
}

/// Notation-only constructs: accepted with a notice, and not carried into the song.
pub(crate) const SLUR: &str = "slur";
pub(crate) const INFO_FIELD: &str = "information field";
pub(crate) const ANNOTATION: &str = "annotation";
pub(crate) const VOICE_PROPERTY: &str = "voice property";
pub(crate) const DIRECTIVE: &str = "%% directive";
pub(crate) const SPACER: &str = "spacer (y)";
pub(crate) const BEAMING: &str = "beaming (spaces between notes)";
pub(crate) const STAFF_PROPERTY: &str = "staff property";
pub(crate) const RESERVED: &str = "reserved character";

/// The refusal for `construct` at `span`, with the reason from [`REFUSED`].
pub(crate) fn refuse(span: Span, construct: &'static str, found: impl Into<String>) -> Error {
    let why = REFUSED
        .iter()
        .find(|r| r.construct == construct)
        .map_or("", |r| r.why);
    Error::Unsupported {
        line: span.line,
        col: span.col,
        construct,
        found: found.into(),
        hint: why.to_owned(),
    }
}

/// [`refuse`] with a hint of its own in place of the table's reason.
pub(crate) fn refuse_with(
    span: Span,
    construct: &'static str,
    found: impl Into<String>,
    hint: impl Into<String>,
) -> Error {
    Error::Unsupported {
        line: span.line,
        col: span.col,
        construct,
        found: found.into(),
        hint: hint.into(),
    }
}
