//! `duet`: compose with an LLM in text both can read, and convert it to and from MIDI.
//!
//! Commands are grouped by domain (`duet song …`, `duet drums …`), plus `duet convert`, which goes
//! between any two formats because every format passes through the same `Song`.

#![forbid(unsafe_code)]

mod config;
mod drums;
mod io;
mod song;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::{Config, Dirs};
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
    /// Show duet's configuration: where its duet.toml files are, and what they set.
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the directories duet reads duet.toml and duet.local.toml from, and each
    /// file in the order it is read, found or not.
    Path,
    /// Print the merged configuration as a duet.toml, each value with the file it came
    /// from.
    Show,
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
            write(&read(&input, from)?, &output, to, Config::load()?.end)
        }
        Command::Song(command) => song::run(command, &Config::load()?),
        Command::Drums(command) => drums::run(command, &Config::load()?),
        Command::Config(ConfigCommand::Path) => stdout(&config::describe_paths(&Dirs::find()?)),
        Command::Config(ConfigCommand::Show) => stdout(&Config::load()?.show()),
    }
}

fn stdout(text: &str) -> Result<()> {
    use std::io::Write as _;
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| anyhow::anyhow!("writing standard output: {e}"))
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
