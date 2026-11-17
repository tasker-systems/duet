//! `duet journal show | check`: a project's journal, read as every duet reads it.
//!
//! Both only read. Neither takes the journal's lock: a reader never sees a torn last line as an
//! entry, and nothing it does changes the file.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use duet_bus::{JOURNAL, Replay, Report};
use duet_schemas::journal::Body;

#[derive(Subcommand)]
pub enum JournalCommand {
    /// Print every entry, one per line: its seq, when, who and through which face, its event
    /// and version, and its body. An entry of an event this duet does not read is shown as
    /// kept, and one that replay skips says so.
    Show {
        /// The journal. Default: journal.duet.jsonl in the working directory.
        #[arg(default_value = JOURNAL)]
        journal: PathBuf,
    },
    /// Check the journal: list every entry of an event this duet does not read, which is kept
    /// as it is; every transition replay skips, such as a second answer to one ask, the first
    /// in file order standing; and every id that appears twice. Exits 1 when anything is
    /// wrong. An entry this duet does not read, and a torn last line, are stated, not wrong.
    Check {
        /// The journal. Default: journal.duet.jsonl in the working directory.
        #[arg(default_value = JOURNAL)]
        journal: PathBuf,
    },
}

pub fn run(command: JournalCommand) -> Result<()> {
    match command {
        JournalCommand::Show { journal } => {
            let read = duet_bus::read(&journal)?;
            let replay = Replay::of(&read.lines);
            let refused: HashMap<u64, _> = replay.refused.iter().map(|r| (r.seq, r)).collect();
            let mut out = String::new();
            for (seq, line) in (1..).zip(&read.lines) {
                let body = match &line.body {
                    Body::Read(event) => serde_json::to_string(event)?,
                    _ => serde_json::from_str::<serde_json::Value>(&line.text)?["body"].to_string(),
                };
                write!(
                    out,
                    "{seq:>4}  {}  {}/{}  {} {}  {body}",
                    line.at, line.by.who, line.by.face, line.event, line.v
                )?;
                if let Body::Skipped(why) = &line.body {
                    write!(out, "  (kept, not read: {why})")?;
                }
                if let Some(r) = refused.get(&seq) {
                    write!(out, "  (skipped on replay: {})", r.refusal)?;
                }
                out.push('\n');
            }
            if read.torn.is_some() {
                out.push_str("      (a torn last line: a write that never finished)\n");
            }
            crate::stdout(&out)
        }
        JournalCommand::Check { journal } => {
            let report = Report::of(&duet_bus::read(&journal)?);
            crate::stdout(&format!("{report}\n"))?;
            if report.is_wrong() {
                bail!("{}: the check found something wrong", journal.display());
            }
            Ok(())
        }
    }
}
