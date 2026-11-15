//! `duet`: compose with an LLM in text both can read, and convert it to and from MIDI.
//!
//! Commands are grouped by domain (`duet song …`, `duet drums …`), plus `duet convert`, which goes
//! between any two formats because every format passes through the same `Song`.

#![forbid(unsafe_code)]

mod abc;
mod capture;
mod config;
mod drums;
mod io;
mod play;
mod song;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use duet_core::Grid;

use crate::abc::AbcCommand;
use crate::capture::CaptureCommand;
use crate::config::{Config, Dirs};
use crate::drums::{DrumsCommand, ReadOptions};
use crate::io::{AnyFormat, Format, GridForm, Loaded, read_any, song_format, write};
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
    /// Convert between formats. A .duet is a document, read by its envelope: a song, or a
    /// drum grid. .mid (.midi, .smf) is MIDI and .abc is ABC. No other name means a format:
    /// --from and --to name notefmt text (text) and a drum grid's text (drums), and standard
    /// input is a document when it starts with {. A .duet output keeps what was read: a grid
    /// is written as a drum-grid document, as is one made from a song with --grid. A grid is
    /// read as `duet drums render` reads it, and written from a song as `duet drums read`
    /// writes it, which needs --grid. Grid to grid is a straight copy, or with --grid goes
    /// through a song at that step.
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
        /// eighths. Required to write a grid from a song; from a grid, it re-steps the grid
        /// through a song, which drops unplayed lanes. Refused for any other output.
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
    /// Work on ABC: check a file against the subset this version reads, render a tune,
    /// or write a song as ABC.
    #[command(subcommand)]
    Abc(AbcCommand),
    /// Capture a recorded take: the exact performance as a .duet song document, a quantized
    /// score (.abc or .drums) and the analysis as a .capture.duet document. What is not given is estimated, and
    /// stated on standard error and in the files.
    #[command(subcommand)]
    Capture(CaptureCommand),
    /// Play a song: through the built-in synth with a General MIDI SoundFont, or to a
    /// MIDI port with --port.
    Play(play::PlayArgs),
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
        Command::Abc(command) => abc::run(command, &Config::load(dirs)?),
        Command::Capture(command) => capture::run(command, &Config::load(dirs)?),
        Command::Play(args) => play::run(args, &Config::load(dirs)?),
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
/// reads with, and written from a song by the code `duet drums read` writes with, so they
/// never disagree. Writing a grid from a song needs `--grid`. A grid written from a grid
/// with no `--grid` is copied as it is; with `--grid`, it goes through a song at that step.
/// A grid flag that would do nothing is refused, so a flag is never silently ignored.
///
/// A `.duet` output keeps what was read: a grid read (a drum-grid document, or a grid's text
/// with `--from drums`) is written as a drum-grid document, as is a grid made from a song with
/// `--grid`; a song is written as a song document. `duet drums render` makes a song of a grid.
fn convert(
    input: &Path,
    output: &Path,
    from: Option<AnyFormat>,
    to: Option<AnyFormat>,
    flags: GridFlags,
    config: &Config,
) -> Result<()> {
    let GridFlags {
        grid,
        track,
        bars,
        snap_levels,
    } = flags;
    let any_grid_flag = grid.is_some() || track.is_some() || bars.is_some() || snap_levels;
    let read = match from {
        Some(AnyFormat::Grid) => Loaded::Grid(drums::read_grid(
            input,
            Some(GridForm::Text),
            "convert reads",
            config,
        )?),
        Some(AnyFormat::Song(format)) => read_any(input, format, "convert reads")?,
        None => {
            let format = song_format(input, None, Format::Text, "convert reads")?;
            read_any(input, format, "convert reads")?
        }
    };
    // The grid form to write, when a grid is written.
    let to_grid = match to {
        Some(AnyFormat::Grid) => Some(GridForm::Text),
        given => {
            let given = given.and_then(|f| match f {
                AnyFormat::Song(format) => Some(format),
                AnyFormat::Grid => None,
            });
            let format = song_format(output, given, Format::Text, "convert writes")?;
            let grid_in = matches!(read, Loaded::Grid(_));
            (format == Format::Duet && (grid_in || any_grid_flag)).then_some(GridForm::Document)
        }
    };
    // Grid to grid with no step named: a straight copy, read and written as `drums snap`
    // does, so nothing a song cannot hold (an unplayed lane, the grid's own levels, in a
    // document any velocity) is lost.
    if let (Loaded::Grid(source), Some(form), None) = (&read, to_grid, grid) {
        let given: Vec<&str> = [(track.is_some(), "--track"), (bars.is_some(), "--bars")]
            .into_iter()
            .filter_map(|(set, flag)| set.then_some(flag))
            .collect();
        if !given.is_empty() {
            bail!(
                "{} {} for reading a song's drum track, and need --grid; without it, a grid \
                 is copied as it is",
                given.join(", "),
                if given.len() == 1 { "is" } else { "are" },
            );
        }
        let copy = if snap_levels {
            source.snapped_to_levels()
        } else {
            source.clone()
        };
        return drums::write_grid(&copy, output, Some(form), "convert writes");
    }
    let song = match read {
        Loaded::Song(song) => song,
        Loaded::Grid(grid) => grid.to_song()?,
    };
    let Some(form) = to_grid else {
        if any_grid_flag {
            let given: Vec<&str> = [
                (grid.is_some(), "--grid"),
                (track.is_some(), "--track"),
                (bars.is_some(), "--bars"),
                (snap_levels, "--snap-levels"),
            ]
            .into_iter()
            .filter_map(|(set, flag)| set.then_some(flag))
            .collect();
            bail!(
                "{} {} for writing a drum grid, and {} is not one; a grid is written as a \
                 .duet, or as its text with --to drums",
                given.join(", "),
                if given.len() == 1 { "is" } else { "are" },
                output.display()
            );
        }
        let format = match to {
            Some(AnyFormat::Song(format)) => format,
            _ => song_format(output, None, Format::Text, "convert writes")?,
        };
        return write(&song, output, format, config.end);
    };
    let Some(grid) = grid else {
        bail!(
            "writing a drum grid from a song needs --grid, the step, such as --grid 1/16; duet \
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
        &drums::grid_of_song(&song, &options, config)?,
        output,
        Some(form),
        "convert writes",
    )
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
