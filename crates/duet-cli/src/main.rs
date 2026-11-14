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

/// Runs a command with config read from `dirs`. Tests pass their own, so a developer's
/// duet.toml never changes what they check.
fn run(cli: Cli, dirs: &Dirs) -> Result<()> {
    match cli.command {
        Command::Convert {
            input,
            output,
            from,
            to,
        } => {
            let from = from.unwrap_or_else(|| format_of(&input, Format::Text));
            let to = to.unwrap_or_else(|| format_of(&output, Format::Text));
            write(&read(&input, from)?, &output, to, Config::load(dirs)?.end)
        }
        Command::Song(command) => song::run(command, &Config::load(dirs)?),
        Command::Drums(command) => drums::run(command, &Config::load(dirs)?),
        Command::Config(ConfigCommand::Path) => stdout(&config::describe_paths(dirs)),
        Command::Config(ConfigCommand::Show) => stdout(&Config::load(dirs)?.show()),
    }
}

fn stdout(text: &str) -> Result<()> {
    use std::io::Write as _;
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| anyhow::anyhow!("writing standard output: {e}"))
}

fn main() -> ExitCode {
    match Dirs::find().and_then(|dirs| run(Cli::parse(), &dirs)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` prints the context chain on one line, like "take.mid: No such file".
            eprintln!("duet: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use clap::Parser;

    use crate::config::Dirs;
    use crate::{Cli, run};

    /// Runs `duet <args>` with no config at all: a fresh, empty user config directory and
    /// no project `.duet`, passed in rather than set in the process's environment, so a
    /// developer's own duet.toml, or a `.duet` above the checkout, cannot change a test.
    pub(crate) fn run_hermetic(args: &[&str]) -> anyhow::Result<()> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let user = std::env::temp_dir().join(format!(
            "duet-no-config-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let dirs = Dirs {
            user: Some((user, "the test's empty config directory")),
            project: None,
        };
        run(
            Cli::parse_from(std::iter::once("duet").chain(args.iter().copied())),
            &dirs,
        )
    }
}
