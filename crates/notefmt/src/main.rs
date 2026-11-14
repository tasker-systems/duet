use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

type BoxError = Box<dyn std::error::Error>;

fn read(path: &Path, format: Format) -> Result<Song, BoxError> {
    let bytes = if path == Path::new("-") {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;
        buf
    } else {
        fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?
    };
    let text = || String::from_utf8(bytes.clone()).map_err(|_| "input is not UTF-8 text");
    let mut song: Song = match format {
        Format::Midi => Song::from_midi(&bytes)?,
        Format::Text => Song::from_text(&text()?)?,
        Format::Json => serde_json::from_str(&text()?)?,
        Format::Toml => toml::from_str(&text()?)?,
    };
    // Hand-edited JSON or TOML may hold lengths or positions in a loose form.
    song.normalize()?;
    Ok(song)
}

fn write(song: &Song, path: &Path, format: Format) -> Result<(), BoxError> {
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
        io::stdout().write_all(&bytes)?;
    } else {
        fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn run(cli: Cli) -> Result<(), BoxError> {
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
            eprintln!("notefmt: {e}");
            ExitCode::FAILURE
        }
    }
}
