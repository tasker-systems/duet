//! Reading and writing songs in every format, chosen by file extension.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;
use clap::builder::PossibleValue;
use duet_core::{EndRule, Song};
use duet_midi::Midi;

use crate::drums::GridForm;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
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

/// Every format `duet convert` goes between: a song's, or a drum grid's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnyFormat {
    Song(Format),
    Grid(GridForm),
}

impl AnyFormat {
    /// The format a path's name gives: `.drums` is .drums text, a `.json` or `.toml` whose
    /// name ends `.drums.json` or `.drums.toml` is a grid's exact form, and anything else is
    /// a song's format by [`format_of`]. `-` has no name, so it takes `default`.
    pub fn of(path: &Path, default: AnyFormat) -> AnyFormat {
        if path == Path::new("-") {
            return default;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if name.ends_with(".drums") {
            AnyFormat::Grid(GridForm::Text)
        } else if name.ends_with(".drums.json") {
            AnyFormat::Grid(GridForm::Json)
        } else if name.ends_with(".drums.toml") {
            AnyFormat::Grid(GridForm::Toml)
        } else {
            AnyFormat::Song(format_of(path, Format::Text))
        }
    }
}

/// The song format of a path a command reads or writes as a song: `given` when the user
/// named one, or else the path's name, by [`AnyFormat::of`]. A name that is a drum grid's is
/// refused rather than misread, so a file name means the same thing to every command.
pub fn song_format(
    path: &Path,
    given: Option<Format>,
    default: Format,
    what: &str,
) -> Result<Format> {
    if let Some(format) = given {
        return Ok(format);
    }
    match AnyFormat::of(path, AnyFormat::Song(default)) {
        AnyFormat::Song(format) => Ok(format),
        AnyFormat::Grid(_) => anyhow::bail!(
            "{} is a drum grid's name (.drums, .drums.json or .drums.toml), and {what} a \
             song; duet convert goes between a grid and a song",
            path.display()
        ),
    }
}

/// The form of a path a command reads or writes as a drum grid, by its name: `.drums`,
/// `.drums.json` or `.drums.toml`, or `-` for .drums text. Any other name is a song's, and
/// is refused rather than misread.
pub fn grid_form(path: &Path, what: &str) -> Result<GridForm> {
    match AnyFormat::of(path, AnyFormat::Grid(GridForm::Text)) {
        AnyFormat::Grid(form) => Ok(form),
        AnyFormat::Song(_) => anyhow::bail!(
            "{} is not a drum grid's name, and {what} a grid; name it .drums, .drums.json or \
             .drums.toml, or - for .drums text",
            path.display()
        ),
    }
}

/// The song formats by their own names, then the grid forms as `drums`, `drums-json` and
/// `drums-toml`. Built from [`Format`]'s own list, so a song format added there is a
/// convert format too.
impl ValueEnum for AnyFormat {
    fn value_variants<'a>() -> &'a [Self] {
        static ALL: std::sync::LazyLock<Vec<AnyFormat>> = std::sync::LazyLock::new(|| {
            Format::value_variants()
                .iter()
                .copied()
                .map(AnyFormat::Song)
                .chain(
                    [GridForm::Text, GridForm::Json, GridForm::Toml]
                        .into_iter()
                        .map(AnyFormat::Grid),
                )
                .collect()
        });
        &ALL
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            AnyFormat::Song(format) => format.to_possible_value(),
            AnyFormat::Grid(GridForm::Text) => Some(PossibleValue::new("drums")),
            AnyFormat::Grid(GridForm::Json) => Some(PossibleValue::new("drums-json")),
            AnyFormat::Grid(GridForm::Toml) => Some(PossibleValue::new("drums-toml")),
        }
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

    use std::path::Path;

    use super::{AnyFormat, Format, read};
    use crate::drums::GridForm;

    #[test]
    fn a_name_ending_drums_is_a_grid_and_a_plain_json_or_toml_is_a_song() {
        let text = AnyFormat::Song(Format::Text);
        let of = |name: &str| AnyFormat::of(Path::new(name), text);
        assert_eq!(of("beat.drums"), AnyFormat::Grid(GridForm::Text));
        assert_eq!(of("dir/Beat.DRUMS.JSON"), AnyFormat::Grid(GridForm::Json));
        assert_eq!(of("beat.drums.toml"), AnyFormat::Grid(GridForm::Toml));
        assert_eq!(of("song.json"), AnyFormat::Song(Format::Json));
        assert_eq!(of("song.toml"), AnyFormat::Song(Format::Toml));
        assert_eq!(of("drums.json"), AnyFormat::Song(Format::Json));
        assert_eq!(of("take.mid"), AnyFormat::Song(Format::Midi));
        assert_eq!(of("take.txt"), text);
        assert_eq!(of("-"), text);
    }

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
