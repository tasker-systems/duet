//! Reading and writing a subset of ABC 2.1 as duet [`Song`](duet_core::Song)s.
//!
//! ABC is the shared score format: a tune people and LLMs can read and discuss. A
//! [`Song`](duet_core::Song) is the exact performance every duet format converts through.
//! This crate reads the part of ABC that a song can hold exactly, and writes a song back
//! as ABC, without quantizing it.
//!
//! ```
//! use duet_abc::{Book, ReadOptions, WriteOptions, write};
//!
//! let abc = "X:1\nT:Scale\nM:4/4\nL:1/8\nQ:1/4=96\nK:D dorian\nD2E2 F2G2 | A8 |]\n";
//! let book = Book::parse(abc)?;
//! let reading = book.tune(1).expect("tune 1").to_song(&ReadOptions::default())?;
//! assert_eq!(reading.song.key.unwrap().to_string(), "D dorian");
//! let written = write(&reading.song, &WriteOptions::default())?;
//! let again = Book::parse(&written)?.tunes()[0].to_song(&ReadOptions::default())?;
//! assert_eq!(again.song, reading.song);
//! # Ok::<(), duet_abc::Error>(())
//! ```
//!
//! # The subset
//!
//! **Read and carried into the song:** the header fields `X:`, `T:`, `M:`, `L:`, `Q:`,
//! `K:` (with a mode) and `V:`; notes with octaves, accidentals and lengths; rests,
//! multi-bar rests, bar lines, ties, broken rhythm, tuplets up to `(9`, chords; repeats
//! and endings, unfolded; inline `[M:]`, `[L:]`, `[Q:]` and `[K:]` in a single-voice tune;
//! voices; `%%MIDI program` and `%%MIDI channel`; the dynamics `!ppp!` to `!fff!`; several
//! tunes per file.
//!
//! **Read with a notice, and not carried:** slurs, information fields such as `C:` and
//! `N:`, annotations, voice properties other than the name, other `%%` directives, and the
//! spacer `y`. [`check`] lists every one with its line.
//!
//! **Refused by name, with line and column:** every construct in [`REFUSED`], such as
//! grace notes, chord symbols, lyrics and parts. Nothing outside the subset is dropped
//! silently.
//!
//! # Where this crate comes from
//!
//! It was written from the ABC 2.1 standard (abcnotation.com/wiki/abc:standard:v2.1),
//! and from abcMIDI's *documentation* of how abc2midi plays what the standard leaves open
//! (its `abcguide.txt` and `readme.txt`). No abcMIDI source file was opened while writing
//! it, and none is copied here. abc2midi is used only as a separate program: a test run
//! on a developer's machine compares this crate's reading of a corpus of our own tunes with
//! abc2midi's, and committed recordings of those readings are checked in CI. No abcMIDI
//! code is linked, and no CI job runs abc2midi.
//!
//! # Where the standard and abc2midi part
//!
//! Where they disagree, this crate refuses the construct rather than pick one: a bare
//! `Q:120` (unit lengths per minute in the standard, quarters in abc2midi) and a tune with
//! no `M:` (free meter in the standard, 4/4 in abc2midi). The bar-length check is this
//! crate's own: abc2midi plays a short bar as written, without a warning.
//!
//! **One place this crate parts from both.** A `:|` with no `|:` goes back to the start of
//! the tune in the standard (§4.8) and in abc2midi, which replays a pickup there and warns
//! of the short bar that makes. A song cannot hold a short bar in its middle, so this
//! crate goes back to the first full bar after the pickup. abc2midi's warning means the
//! corpus cannot record the case; a test pins this crate's reading instead. Write `|:`
//! after the pickup and all three agree.
//!
//! A `||` inside a repeat is a bar line like any other: a later `:|` goes back past it, as
//! abc2midi plays it. The standard also allows going back only to the `||`.

#![forbid(unsafe_code)]

mod error;
mod lex;
mod parse;
mod ratio;
mod read;
mod subset;
mod write;

pub use error::{Error, Result};
pub use lex::Span;
pub use parse::{Book, Tune};
pub use read::{Notice, ReadOptions, Reading, pickup_meter_for};
pub use subset::{REFUSED, Refusal};
pub use write::{Dynamics, WriteOptions, write};

/// Reads every tune in a file, and returns the notation-only constructs they held, each
/// with its line. Fails on the first tune that is not valid ABC, or that holds a construct
/// outside the subset, naming the construct and its line.
///
/// The file header's notices are listed once, though its lines apply to every tune.
pub fn check(text: &str) -> Result<Vec<Notice>> {
    let book = Book::parse(text)?;
    let mut notices = Vec::new();
    for reading in book.read_all(&ReadOptions::default()) {
        notices.extend(reading?.notices);
    }
    Ok(notices)
}
