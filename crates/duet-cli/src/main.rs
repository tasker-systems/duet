//! `duet`: compose with an LLM in text both can read, and convert it to and from MIDI.
//!
//! Commands are grouped by domain (`duet part …`, `duet drums …`), plus `duet convert`, which goes
//! between any two formats because every format passes through the same `Part`.
//!
//! **This crate is a face.** Every command that does work is an action in `duet-actions`: clap
//! parses the action's own input type, and [`run`] hands it to the action with a
//! [`Context`]. Inside a project the documents the action wrote are then appended to the
//! project's journal as `doc.written` events; outside one there is no journal. Only reading the
//! configuration and the journal stays here.

#![forbid(unsafe_code)]

mod journal;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use duet_actions::abc::AbcCommand;
use duet_actions::capture::CaptureCommand;
use duet_actions::config::{self, Config, Dirs};
use duet_actions::drums::DrumsCommand;
use duet_actions::io::{stdout, tool};
use duet_actions::part::PartCommand;
use duet_actions::project::{InitInput, ProjectCommand};
use duet_actions::song::SongCommand;
use duet_actions::take::TakeCommand;
use duet_actions::{Context, Project, abc, actor, capture, convert, drums, fresh_id, part, play};
use duet_actions::{project, record, song, take, view};
use duet_bus::Writer;

/// Compose with an LLM in text both can read, and convert it to and from MIDI.
#[derive(Parser)]
#[command(name = "duet", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make a project: .duet/project.duet, and .duet/duet.toml when there is none. A directory
    /// that holds a project already is refused, as is one inside another project unless
    /// --nested is given, and the home directory itself. A .duet/ that holds only config is
    /// adopted, its config kept as it is. Nothing else is written: no song, no other directory,
    /// no git file.
    Init(InitInput),
    /// Read the project a command runs in: the nearest directory up whose .duet/ holds
    /// project.duet, or the one DUET_PROJECT names.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Work on a project's takes: add a recording, find one again, list them, or forget one.
    /// A take is copied into takes/TAKE/, or named where it is in .duet/takes.local.toml,
    /// and is always checked against its digest.
    #[command(subcommand)]
    Take(TakeCommand),
    /// Convert between formats. A .duet is a document, read by its envelope: a part, or a
    /// drum grid. .mid (.midi, .smf) is MIDI and .abc is ABC. No other name means a format:
    /// --from and --to name one, standard input is a document when it starts with {, and
    /// standard output is a document. A view's text is not a format: `duet view` writes it and
    /// `duet apply` reads it. A .duet output keeps what was read: a grid is written as a
    /// drum-grid document, as is one made from a part with --grid. A grid is written from a
    /// part as `duet drums read` writes it, which needs --grid. Grid to grid is a straight
    /// copy, or with --grid goes through a part at that step.
    Convert(convert::ConvertInput),
    /// Render a document as a text view: notes of a part, a grid of a drum grid, or a grid of
    /// a part's drum track with --as grid. The first line names the view and the document it
    /// was rendered from, so `duet apply` can read it back.
    View(view::ViewArgs),
    /// Read an edited view back: --onto the document it was rendered from, keeping what the
    /// view does not show, or -o a new document from a lossless view.
    Apply(view::ApplyArgs),
    /// Work on a part: show it, or quantize it.
    #[command(subcommand)]
    Part(PartCommand),
    /// Work on a song in a project: make one, show it, or move its status. A song is its
    /// folder, `songs/<song>/`, holding song.duet (the song and its composition) and parts/.
    #[command(subcommand)]
    Song(SongCommand),
    /// Work on drum grids: render one as a part, read one from a part's drum track,
    /// snap its velocities, or list the lanes.
    #[command(subcommand)]
    Drums(DrumsCommand),
    /// Work on ABC: check a file against the subset this version reads, render a tune,
    /// or write a part as ABC.
    #[command(subcommand)]
    Abc(AbcCommand),
    /// Capture a recorded take: the exact performance as a .duet part document, a quantized
    /// score (.abc, or a .grid.duet drum-grid document) and the analysis as a .capture.duet
    /// document. What is not given is estimated, and stated on standard error and in the files.
    #[command(subcommand)]
    Capture(CaptureCommand),
    /// Play a part: through the built-in synth with a General MIDI SoundFont, or to a
    /// MIDI port with --port.
    Play(play::PlayArgs),
    /// Record a take from the microphone (the default input, or one named with --device) to
    /// a FLAC file, until Ctrl-C or for --seconds. `duet capture` reads it like any take.
    Record(record::RecordArgs),
    /// Show duet's configuration: where its duet.toml files are, and what they set.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Read a project's journal, journal.duet.jsonl: the shared, ordered account of what the
    /// owner and Claude did and asked of each other.
    #[command(subcommand)]
    Journal(journal::JournalCommand),
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

/// Runs a command with config read from `dirs`, in `project` if it is in one. Tests pass their
/// own, so a developer's duet.toml or project never changes what they check.
fn run(cli: Cli, dirs: &Dirs, project: Option<Project>) -> Result<()> {
    // The commands that read no configuration: a broken duet.toml leaves them working, as it
    // always has.
    let loads = !matches!(
        cli.command,
        Command::Init(_)
            | Command::Project(_)
            | Command::Song(_)
            | Command::Take(_)
            | Command::Record(_)
            | Command::Config(_)
            | Command::Journal(_)
    );
    // `init` makes a project, and runs in none: it journals nothing (duet_actions::project).
    let project = if matches!(cli.command, Command::Init(_)) {
        None
    } else {
        project
    };
    let config = if loads {
        Config::load(dirs)?
    } else {
        Config::built_in()
    };
    let ctx = Context::new(project, config);
    let ran = match cli.command {
        Command::Init(input) => project::init(&ctx, input),
        Command::Project(command) => project::run(&ctx, command),
        Command::Take(command) => take::run(&ctx, command),
        Command::Convert(input) => convert::run(&ctx, input),
        Command::Part(command) => part::run(&ctx, command),
        Command::Song(command) => song::run(&ctx, command),
        Command::Drums(command) => drums::run(&ctx, command),
        Command::Abc(command) => abc::run(&ctx, command),
        Command::Capture(command) => capture::run(&ctx, command),
        Command::Play(args) => play::run(&ctx, args),
        Command::Record(args) => record::run(&ctx, args),
        Command::View(args) => view::run_view(&ctx, args),
        Command::Apply(args) => view::run_apply(&ctx, args),
        Command::Config(ConfigCommand::Path) => stdout(&config::describe_paths(dirs)),
        Command::Config(ConfigCommand::Show) => stdout(&Config::load(dirs)?.show()),
        Command::Journal(command) => journal::run(command),
    };
    // What the action wrote is journalled even when it failed after writing it, so no write is
    // left out of the account; the action's own error is the one reported.
    let journalled = journal_written(&ctx, dirs, loads);
    ran.and(journalled)
}

/// Appends a `doc.written` for each document the action wrote, inside a project, then each other
/// event it emitted (a `take.added`), all caused by one command id, as `by` decision 8 names. The segment size is the configuration's
/// `[journal] segment-size`.
fn journal_written(ctx: &Context, dirs: &Dirs, loaded: bool) -> Result<()> {
    let (Some(project), written, emitted) = (ctx.project(), ctx.written(), ctx.emitted()) else {
        return Ok(());
    };
    if written.is_empty() && emitted.is_empty() {
        return Ok(());
    }
    let segment_size = if loaded {
        ctx.config().segment_size
    } else {
        Config::load(dirs)?.segment_size
    };
    let mut writer =
        Writer::new(&project.root, tool(), &project.name, actor("cli")).segment_size(segment_size);
    let cause = fresh_id()?;
    for doc in written {
        writer.append(doc, Some(cause.clone()))?;
    }
    for event in emitted {
        writer.append(event, Some(cause.clone()))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let found = Dirs::find().and_then(|dirs| {
        let cwd = std::env::current_dir()?;
        Ok((dirs, Project::find(&cwd)?))
    });
    match found.and_then(|(dirs, project)| run(Cli::parse(), &dirs, project)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` prints the context chain on one line, like "take.mid: No such file".
            eprintln!("duet: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
