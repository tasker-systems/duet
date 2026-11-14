//! `duet`: compose with an LLM in text both can read, and convert it to and from MIDI.
//!
//! Commands are grouped by domain (`duet song …`, `duet drums …`), plus `duet convert`, which goes
//! between any two formats because every format passes through the same `Song`.

#![forbid(unsafe_code)]

mod drums;
mod io;
mod song;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::drums::DrumsCommand;
use crate::io::{Format, format_of, read, write};
use crate::song::SongCommand;

/// Compose with an LLM in text both can read, and convert it to and from MIDI.
#[derive(Parser)]
#[command(name = "duet", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert between formats. Formats come from the file extensions (.mid or .midi,
    /// .json, .toml, anything else is notefmt text) unless --from or --to is given.
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
    /// Work on a song: show it, or quantize it.
    #[command(subcommand)]
    Song(SongCommand),
    /// Work on drum grids: render one as a song, read one from a song's drum track,
    /// snap its velocities, or list the lanes.
    #[command(subcommand)]
    Drums(DrumsCommand),
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
        Command::Song(command) => song::run(command),
        Command::Drums(command) => drums::run(command),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` prints the context chain on one line, like "take.mid: No such file".
            eprintln!("duet: {e:#}");
            ExitCode::FAILURE
        }
    }
}
