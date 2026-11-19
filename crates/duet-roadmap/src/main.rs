//! `duet-roadmap snapshot`: draw duet's roadmap from the Temper records.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Parser, Subcommand};
use duet_roadmap::{Mode, TemperCli, draw_snapshot};

/// Draws duet's roadmap from the Temper records.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read the open goals and their work, and create a new dated `roadmap-snapshot` research
    /// resource in +making-music/duet-dev. Nothing else is written, and no snapshot is ever
    /// updated. If any read fails or comes back partial, it says which, exits non-zero and writes
    /// nothing.
    Snapshot {
        /// Print the snapshot instead of writing it.
        #[arg(long)]
        dry_run: bool,
        /// The `temper` to run, if not the one on PATH.
        #[arg(long, value_name = "PATH", default_value = "temper")]
        temper: PathBuf,
    },
}

fn main() -> ExitCode {
    let Command::Snapshot { dry_run, temper } = Cli::parse().command;
    let mode = if dry_run { Mode::DryRun } else { Mode::Write };
    let mut temper = TemperCli::new(temper);
    match draw_snapshot(&mut temper, &mut SystemTime::now, mode) {
        Ok(drawn) => {
            match drawn.created {
                Some(id) => println!("created {id}: {}", drawn.title),
                None => print!("{}", drawn.body),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("duet-roadmap: {e}");
            if !e.may_have_written() {
                eprintln!("duet-roadmap: no snapshot was drawn, and nothing was written");
            }
            ExitCode::FAILURE
        }
    }
}
