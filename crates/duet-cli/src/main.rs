//! `duet`: compose with an LLM in text both can read, and convert it to and from MIDI.
//!
//! Commands are grouped by domain (`duet song …`, `duet drums …`), plus `duet convert`, which goes
//! between any two formats because every format passes through the same `Song`.

#![forbid(unsafe_code)]

mod config;
mod drums;
mod io;
mod song;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use duet_core::Grid;

use crate::config::{Config, Dirs};
use crate::drums::{DrumsCommand, ReadOptions};
use crate::io::{AnyFormat, Format, read, write};
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
    /// Convert between formats. Formats come from the file names (.mid or .midi, .json,
    /// .toml, .drums, .drums.json, .drums.toml, anything else is notefmt text) unless
    /// --from or --to is given. A drum grid is read as `duet drums render` reads it, and
    /// written as `duet drums read` writes it, which needs --grid.
    Convert {
        /// The file to read, or - for standard input.
        input: PathBuf,
        /// The file to write, or - for standard output.
        output: PathBuf,
        #[arg(long, value_enum)]
        from: Option<AnyFormat>,
        #[arg(long, value_enum)]
        to: Option<AnyFormat>,
        /// Writing a drum grid: the step, such as 1/16 for sixteenths or 1/8t for triplet
        /// eighths. Required for a grid, and refused for anything else.
        #[arg(long)]
        grid: Option<Grid>,
        /// Writing a drum grid: the drum track's name, as for `duet drums read`.
        #[arg(long)]
        track: Option<String>,
        /// Writing a drum grid: how many bars it runs to, as for `duet drums read`.
        #[arg(long)]
        bars: Option<u32>,
        /// Writing a drum grid: snap each velocity to the text levels, as for
        /// `duet drums read`.
        #[arg(long)]
        snap_levels: bool,
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
            grid,
            track,
            bars,
            snap_levels,
        } => convert(
            &input,
            &output,
            from,
            to,
            GridFlags {
                grid,
                track,
                bars,
                snap_levels,
            },
            &Config::load(dirs)?,
        ),
        Command::Song(command) => song::run(command, &Config::load(dirs)?),
        Command::Drums(command) => drums::run(command, &Config::load(dirs)?),
        Command::Config(ConfigCommand::Path) => stdout(&config::describe_paths(dirs)),
        Command::Config(ConfigCommand::Show) => stdout(&Config::load(dirs)?.show()),
    }
}

/// `duet convert`'s grid flags, as given.
struct GridFlags {
    grid: Option<Grid>,
    track: Option<String>,
    bars: Option<u32>,
    snap_levels: bool,
}

/// Reads any format and writes any other. A grid is read by the code `duet drums render`
/// reads with, and written by the code `duet drums read` writes with, so they never
/// disagree. Writing a grid needs `--grid`, and the grid flags are refused for anything
/// else, so a flag is never silently ignored.
fn convert(
    input: &Path,
    output: &Path,
    from: Option<AnyFormat>,
    to: Option<AnyFormat>,
    flags: GridFlags,
    config: &Config,
) -> Result<()> {
    let text = AnyFormat::Song(Format::Text);
    let from = from.unwrap_or_else(|| AnyFormat::of(input, text));
    let to = to.unwrap_or_else(|| AnyFormat::of(output, text));
    let song = match from {
        AnyFormat::Song(format) => read(input, format)?,
        AnyFormat::Grid(form) => drums::read_grid(input, form, config)?.to_song()?,
    };
    let GridFlags {
        grid,
        track,
        bars,
        snap_levels,
    } = flags;
    match to {
        AnyFormat::Song(format) => {
            let given: Vec<&str> = [
                (grid.is_some(), "--grid"),
                (track.is_some(), "--track"),
                (bars.is_some(), "--bars"),
                (snap_levels, "--snap-levels"),
            ]
            .into_iter()
            .filter_map(|(set, flag)| set.then_some(flag))
            .collect();
            if !given.is_empty() {
                bail!(
                    "{} {} for writing a drum grid, and {} is not one; a grid is .drums, \
                     .drums.json or .drums.toml, or --to drums, drums-json or drums-toml",
                    given.join(", "),
                    if given.len() == 1 { "is" } else { "are" },
                    output.display()
                );
            }
            write(&song, output, format, config.end)
        }
        AnyFormat::Grid(form) => {
            let Some(grid) = grid else {
                bail!(
                    "writing a drum grid needs --grid, the step, such as --grid 1/16; duet \
                     never guesses it"
                );
            };
            let options = ReadOptions {
                grid,
                track,
                bars,
                snap_levels,
            };
            drums::write_grid(
                &drums::grid_of_song(&song, &options, form, config)?,
                output,
                form,
            )
        }
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
