//! `duet abc`: ABC tunes, read through the subset duet-abc reads and refused by name
//! outside it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use duet_abc::{Book, Dynamics, ReadOptions, Tune, WriteOptions};
use duet_core::Song;

use crate::config::Config;
use crate::io::{Format, read, same_file, song_format, with_end, write};

#[derive(Subcommand)]
pub enum AbcCommand {
    /// Read every tune in a file, and list what each holds that is notation only (and
    /// not carried into a song), or the construct it is refused for, with its line.
    Check {
        /// The ABC file to read, or - for standard input.
        input: PathBuf,
    },
    /// Render one tune as a song: MIDI, notefmt text, JSON or TOML, by the output's
    /// extension (.mid or .midi, .json, .toml, anything else is notefmt text).
    Render {
        /// The ABC file to read, or - for standard input.
        input: PathBuf,
        /// The file to write, or - for standard output. Must not be the input.
        output: PathBuf,
        /// The tune's X: number. Needed only when the file holds more than one tune.
        #[arg(long)]
        tune: Option<u32>,
        #[arg(long, value_enum)]
        to: Option<Format>,
    },
    /// Write a song as ABC. Nothing is quantized: a note ABC cannot write where it is
    /// is refused by name.
    Write {
        /// The song to read (MIDI, notefmt text, JSON or TOML), or - for standard input.
        input: PathBuf,
        /// The ABC file to write, or - for standard output. Must not be the input.
        output: PathBuf,
        /// The unit note length (L:), such as 1/4 for a slow vocal line. Without it, the
        /// standard's default for the meter.
        #[arg(long)]
        unit: Option<String>,
        /// What to do with a velocity that is not a dynamic's level: refuse it (the
        /// default), or snap it to the nearest, which the ABC then says in a comment.
        #[arg(long, value_enum, default_value = "refuse")]
        dynamics: DynamicsArg,
        #[arg(long, value_enum)]
        from: Option<Format>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DynamicsArg {
    Refuse,
    Nearest,
}

fn text_of(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("reading standard input")?;
        Ok(buf)
    } else {
        fs::read_to_string(path).with_context(|| path.display().to_string())
    }
}

/// Reads a tune, and says on standard error how many notation-only constructs it held,
/// so nothing is dropped silently.
pub fn reading(tune: &Tune) -> Result<Song> {
    let reading = tune.to_song(&ReadOptions::default())?;
    let n = reading.notices.len();
    if n > 0 {
        eprintln!(
            "duet: X:{} held {n} notation-only construct{}, not carried into the song; \
             duet abc check lists them",
            tune.x(),
            if n == 1 { "" } else { "s" }
        );
    }
    Ok(reading.song)
}

fn unit_of(text: &str) -> Result<(u32, u32)> {
    let bad = || anyhow::anyhow!("'{text}' is not a unit length like 1/8 or 1/4");
    let (n, d) = text.split_once('/').unwrap_or((text, "1"));
    let n: u32 = n.trim().parse().map_err(|_| bad())?;
    let d: u32 = d.trim().parse().map_err(|_| bad())?;
    if n == 0 || d == 0 {
        return Err(bad());
    }
    Ok((n, d))
}

pub fn run(command: AbcCommand, config: &Config) -> Result<()> {
    match command {
        AbcCommand::Check { input } => {
            let book = Book::parse(&text_of(&input)?)?;
            let mut out = String::new();
            let mut refused = 0;
            let readings = book.read_all(&ReadOptions::default());
            for (tune, reading) in book.tunes().iter().zip(readings) {
                let name = match tune.title() {
                    Some(t) => format!("X:{} {t}", tune.x()),
                    None => format!("X:{}", tune.x()),
                };
                match reading {
                    Ok(reading) => {
                        out.push_str(&format!(
                            "{name}: read, with {} notation-only construct{}\n",
                            reading.notices.len(),
                            if reading.notices.len() == 1 { "" } else { "s" }
                        ));
                        for notice in &reading.notices {
                            out.push_str(&format!("  {notice}\n"));
                        }
                    }
                    Err(e) => {
                        refused += 1;
                        out.push_str(&format!("{name}: not read: {e}\n"));
                    }
                }
            }
            crate::stdout(&out)?;
            if refused > 0 {
                bail!(
                    "{refused} of {} tune{} not read",
                    book.tunes().len(),
                    if book.tunes().len() == 1 { "" } else { "s" }
                );
            }
            Ok(())
        }
        AbcCommand::Render {
            input,
            output,
            tune,
            to,
        } => {
            if input != Path::new("-") && same_file(&input, &output) {
                bail!(
                    "{} is the input; render writes a new file",
                    output.display()
                );
            }
            let book = Book::parse(&text_of(&input)?)?;
            let tune = match tune {
                Some(x) => book
                    .tune(x)
                    .with_context(|| format!("{} holds no tune X:{x}", input.display()))?,
                None if book.tunes().len() == 1 => &book.tunes()[0],
                None => bail!(
                    "{} holds {} tunes; pick one with --tune <X>",
                    input.display(),
                    book.tunes().len()
                ),
            };
            let song = reading(tune)?;
            let to = song_format(&output, to, Format::Midi, "duet abc render writes")?;
            write(&song, &output, to, config.end)
        }
        AbcCommand::Write {
            input,
            output,
            unit,
            dynamics,
            from,
        } => {
            if input != Path::new("-") && same_file(&input, &output) {
                bail!("{} is the input; write makes a new file", output.display());
            }
            let from = song_format(&input, from, Format::Text, "duet abc write reads")?;
            let song = with_end(&read(&input, from)?, config.end)?;
            let mut options = WriteOptions::default().with_dynamics(match dynamics {
                DynamicsArg::Refuse => Dynamics::Refuse,
                DynamicsArg::Nearest => Dynamics::Nearest,
            });
            if let Some(unit) = unit {
                let (n, d) = unit_of(&unit)?;
                options = options.with_unit(n, d);
            }
            let abc = duet_abc::write(&song, &options)?;
            if output == Path::new("-") {
                crate::stdout(&abc)
            } else {
                fs::write(&output, abc).with_context(|| output.display().to_string())
            }
        }
    }
}
