//! Reading and writing songs in every format, chosen by file extension.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;
use duet_core::{EndRule, Song};
use duet_midi::Midi;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Midi,
    Text,
    Json,
    Toml,
}

/// The format a path's extension names: .mid, .midi or .smf is MIDI, .json and .toml are
/// themselves, and anything else is notefmt text. `-` (standard input or output) has no
/// extension, so it takes `default`.
pub fn format_of(path: &Path, default: Format) -> Format {
    if path == Path::new("-") {
        return default;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mid" | "midi" | "smf") => Format::Midi,
        Some("json") => Format::Json,
        Some("toml") => Format::Toml,
        _ => Format::Text,
    }
}

pub fn read(path: &Path, format: Format) -> Result<Song> {
    let bytes = if path == Path::new("-") {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .context("reading standard input")?;
        buf
    } else {
        fs::read(path).with_context(|| path.display().to_string())?
    };
    let text = || String::from_utf8(bytes.clone()).context("input is not UTF-8 text");
    // MIDI and text come back normalized. Hand-edited JSON or TOML may hold lengths or
    // positions in a loose form, so those are normalized here.
    let song = match format {
        Format::Midi => Song::from_midi(&bytes)?,
        Format::Text => Song::from_text(&text()?)?,
        Format::Json => normalized(serde_json::from_str(&text()?)?)?,
        Format::Toml => normalized(toml::from_str(&text()?)?)?,
    };
    Ok(song)
}

fn normalized(mut song: Song) -> duet_core::Result<Song> {
    song.normalize()?;
    Ok(song)
}

/// The song with its end stated: the one it names, or else the configured default's.
pub fn with_end(song: &Song, end: EndRule) -> Result<Song> {
    let mut song = song.clone();
    if song.end.is_none() {
        song.end = Some(song.default_end_by(end)?);
    }
    Ok(song)
}

/// Writes the song in a format. Every file written says where the song ends, whatever
/// default computed it: text and MIDI state it themselves, and JSON and TOML carry it as
/// the `end` field.
pub fn write(song: &Song, path: &Path, format: Format, end: EndRule) -> Result<()> {
    let song = with_end(song, end)?;
    let song = &song;
    let bytes = match format {
        Format::Midi => song.to_midi()?,
        Format::Text => song.to_text().into_bytes(),
        Format::Json => {
            let mut s = serde_json::to_string_pretty(song)?;
            s.push('\n');
            s.into_bytes()
        }
        Format::Toml => toml::to_string(song)?.into_bytes(),
    };
    if path == Path::new("-") {
        io::stdout()
            .write_all(&bytes)
            .context("writing standard output")?;
    } else {
        fs::write(path, bytes).with_context(|| path.display().to_string())?;
    }
    Ok(())
}

/// Whether two paths name the same existing file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use duet_core::{Note, Position, Song, Track};

    use super::{Format, read};

    #[test]
    fn read_normalizes_json_and_toml() {
        // A loose position and a length that is not whole ticks, as a hand edit might leave.
        let mut song = Song::default();
        let mut track = Track::new("P", 1);
        track.notes.push(Note::new(
            Position::new(1, 1, 960),
            "C4".parse().expect("C4 is a pitch"),
            0.3334,
            80,
        ));
        song.tracks.push(track);

        let dir = std::env::temp_dir().join(format!("duet-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let json = dir.join("song.json");
        let toml_path = dir.join("song.toml");
        std::fs::write(&json, serde_json::to_string(&song).expect("json")).expect("write");
        std::fs::write(&toml_path, toml::to_string(&song).expect("toml")).expect("write");

        for (path, format) in [(&json, Format::Json), (&toml_path, Format::Toml)] {
            let read = read(path, format).expect("read");
            let note = &read.tracks[0].notes[0];
            assert_eq!((note.at.to_string(), note.beats), ("1.3.0".into(), 0.333));
        }
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
