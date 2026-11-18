//! JSON Schemas for the five types that serialize as strings by hand.
//!
//! A derived schema would describe their fields, which are never written: each is a string,
//! with the pattern its `FromStr` reads. The pattern says what reads as text; the reader also
//! checks what a pattern cannot, such as a key's signature having at most 7 sharps or flats,
//! and a number fitting its type.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::{Grid, Key, Pitch, Position, TimeSig};

/// One `impl JsonSchema` giving `type: string` with `pattern` and `description`.
macro_rules! string_schema {
    ($ty:ty, $name:literal, $pattern:expr, $description:literal) => {
        impl JsonSchema for $ty {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed($name)
            }

            fn schema_id() -> Cow<'static, str> {
                Cow::Borrowed(concat!("duet_core::", $name))
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": $pattern,
                    "description": $description,
                })
            }
        }
    };
}

/// A bar and a beat, each from 1, and an optional tick offset.
pub(crate) const POSITION: &str = r"^0*[1-9][0-9]*\.0*[1-9][0-9]*(\.[0-9]+)?$";
/// Beats over a power of two up to 64, with spaces allowed around each number.
pub(crate) const TIME_SIG: &str = r"^\s*0*[1-9][0-9]*\s*/\s*0*(1|2|4|8|16|32|64)\s*$";
/// A tonic letter, sharps or flats, and an optional mode word, read by its first three letters
/// in any case (`m` alone is minor).
pub(crate) const KEY: &str = concat!(
    r"^\s*[A-Ga-g][#b]*(\s*([Mm]|([Mm][Aa][Jj]|[Ii][Oo][Nn]|[Mm][Ii][Nn]|[Aa][Ee][Oo]|",
    r"[Dd][Oo][Rr]|[Pp][Hh][Rr]|[Ll][Yy][Dd]|[Mm][Ii][Xx]|[Ll][Oo][Cc]).*))?\s*$"
);
/// `1/` and a note value, with `t` for a triplet.
pub(crate) const GRID: &str = r"^\s*1/0*[1-9][0-9]*t?\s*$";
/// A letter, sharps or flats, and an octave from -1.
pub(crate) const PITCH: &str = r"^[A-Ga-g]([#♯]{0,2}|[b♭]{0,2})-?[0-9]+$";

string_schema!(
    Position,
    "Position",
    POSITION,
    "A position as bar.beat.tick, such as 3.2.240: bar and beat from 1, the tick an offset into the beat in the part's ticks. bar.beat alone is tick 0."
);
string_schema!(
    TimeSig,
    "TimeSig",
    TIME_SIG,
    "A time signature such as 4/4 or 6/8: the denominator is 1, 2, 4, 8, 16, 32 or 64."
);
string_schema!(
    Key,
    "Key",
    KEY,
    "A key such as D minor, Bb major or D dorian, with at most 7 sharps or flats."
);
string_schema!(
    Grid,
    "Grid",
    GRID,
    "A quantize grid such as 1/16, or 1/8t for triplet eighths."
);
string_schema!(
    Pitch,
    "Pitch",
    PITCH,
    "A pitch in scientific notation, such as C4 (MIDI 60), F#3, Bb2, Cb5 (MIDI 71) or F##4 (MIDI 67), from C-1 to G9: a letter, up to two sharps or two flats, and the letter's octave. Its spelling is kept."
);
