#![forbid(unsafe_code)]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use notefmt::Song;

/// Convert MIDI to a readable note format (text, JSON or TOML) and back.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert between formats. Formats come from the file extensions (.mid or .midi,
    /// .json, .toml, anything else is text) unless --from or --to is given.
    Convert {
        /// The file to read, or - for standard input.
        input: PathBuf,
        /// The file to write, or - for standard output.
        output: PathBuf,
        #[arg(long, value_enum)]
        from: Option<Format>,
        #[arg(long, value_enum)]
        to: Option<Format>,
    },
    /// Print a file as text (or another format with --to).
    Show {
        /// The file to read, or - for standard input.
        input: PathBuf,
        #[arg(long, value_enum)]
        from: Option<Format>,
        #[arg(long, value_enum, default_value = "text")]
        to: Format,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Midi,
    Text,
    Json,
    Toml,
}

fn format_of(path: &Path, default: Format) -> Format {
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

fn read(path: &Path, format: Format) -> Result<Song> {
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

fn normalized(mut song: Song) -> notefmt::Result<Song> {
    song.normalize()?;
    Ok(song)
}

fn write(song: &Song, path: &Path, format: Format) -> Result<()> {
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

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Convert {
            input,
            output,
            from,
            to,
        } => {
            let from = from.unwrap_or_else(|| format_of(&input, Format::Text));
            let to = to.unwrap_or_else(|| format_of(&output, Format::Text));
            write(&read(&input, from)?, &output, to)
        }
        Command::Show { input, from, to } => {
            let from = from.unwrap_or_else(|| format_of(&input, Format::Midi));
            write(&read(&input, from)?, Path::new("-"), to)
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` prints the context chain on one line, like "take.mid: No such file".
            eprintln!("notefmt: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use notefmt::{Note, Position, Track};

    use super::{Format, Song, read};

    #[test]
    fn read_normalizes_json_and_toml() {
        // A loose position and a length that is not whole ticks, as a hand edit might leave.
        let mut song = Song::default();
        let mut track = Track::new("P", 1);
        track.notes.push(Note {
            at: Position::new(1, 1, 960),
            pitch: "C4".parse().expect("C4 is a pitch"),
            beats: 0.3334,
            vel: 80,
        });
        song.tracks.push(track);

        let dir = std::env::temp_dir().join(format!("notefmt-read-{}", std::process::id()));
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
