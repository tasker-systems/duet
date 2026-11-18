//! `duet abc`: ABC tunes, read through the subset duet-abc reads and refused by name
//! outside it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Subcommand, ValueEnum};
use duet_abc::{Book, Dynamics, ReadOptions, Tune, WriteOptions};
use duet_core::Part;
use duet_schemas::{From, Notice, Source};

use crate::Context;
use crate::io::{Format, FormatParser, Origin, part_format, read, same_file, with_end, write_from};
use clap::Args;
use schemars::JsonSchema;

#[derive(Debug, Subcommand)]
pub enum AbcCommand {
    /// Read every tune in a file, and list what each holds that is notation only (and
    /// not carried into a part), or the construct it is refused for, with its line.
    Check(CheckInput),
    /// Render one tune as a part: a document (.duet) or MIDI (.mid or .midi) by the output's
    /// name. Standard output is MIDI unless --to says. `duet view` shows a part as text.
    Render(RenderInput),
    /// Write a part as ABC. Nothing is quantized: a note ABC cannot write where it is
    /// is refused by name.
    Write(WriteInput),
}

/// Read every tune in a file, and list what each holds that is notation only (and
/// not carried into a part), or the construct it is refused for, with its line.
#[derive(Debug, Args, JsonSchema)]
pub struct CheckInput {
    /// The ABC file to read, or - for standard input.
    pub input: PathBuf,
}

/// Render one tune as a part: a document (.duet) or MIDI (.mid or .midi) by the output's
/// name. Standard output is MIDI unless --to says. `duet view` shows a part as text.
#[derive(Debug, Args, JsonSchema)]
pub struct RenderInput {
    /// The ABC file to read, or - for standard input.
    pub input: PathBuf,
    /// The file to write, or - for standard output. Must not be the input.
    pub output: PathBuf,
    /// The tune's X: number. Needed only when the file holds more than one tune.
    #[arg(long)]
    pub tune: Option<u32>,
    #[arg(long, value_parser = FormatParser)]
    pub to: Option<Format>,
}

/// Write a part as ABC. Nothing is quantized: a note ABC cannot write where it is
/// is refused by name.
#[derive(Debug, Args, JsonSchema)]
pub struct WriteInput {
    /// The part to read: a .duet document or MIDI by its name, or - for standard input (a
    /// document if it starts with {, else MIDI).
    pub input: PathBuf,
    /// The ABC file to write, or - for standard output. Must not be the input.
    pub output: PathBuf,
    /// The unit note length (L:), such as 1/4 for a slow vocal line. Without it, the
    /// standard's default for the meter.
    #[arg(long)]
    pub unit: Option<String>,
    /// What to do with a velocity that is not a dynamic's level: refuse it (the
    /// default), or snap it to the nearest, which the ABC then says in a comment.
    #[arg(long, value_enum, default_value = "refuse")]
    pub dynamics: DynamicsArg,
    #[arg(long, value_parser = FormatParser)]
    pub from: Option<Format>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
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
/// so nothing is dropped silently. They come back as the part's provenance, `from`: the
/// format, the tune's `X:` number, and each notice at its line and column, as a `.duet`
/// written from the part records them. A refused construct fails the read, so none is ever
/// recorded.
pub fn reading(tune: &Tune) -> Result<(Part, From)> {
    let reading = tune.to_part(&ReadOptions::default())?;
    let n = reading.notices.len();
    if n > 0 {
        eprintln!(
            "duet: X:{} held {n} notation-only construct{}, not carried into the part; \
             duet abc check lists them",
            tune.x(),
            if n == 1 { "" } else { "s" }
        );
    }
    let mut from = From::new("abc");
    from.tune = Some(tune.x().to_string());
    from.notices = reading.notices.iter().map(notice).collect();
    Ok((reading.part, from))
}

/// duet-abc's notice, as the envelope records it.
fn notice(n: &duet_abc::Notice) -> Notice {
    let at = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Notice::new(at(n.line), at(n.col), n.construct, n.found.clone())
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

pub fn run(ctx: &Context, command: AbcCommand) -> Result<()> {
    let config = ctx.config();
    match command {
        AbcCommand::Check(CheckInput { input }) => {
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
            crate::io::stdout(&out)?;
            if refused > 0 {
                bail!(
                    "{refused} of {} tune{} not read",
                    book.tunes().len(),
                    if book.tunes().len() == 1 { "" } else { "s" }
                );
            }
            Ok(())
        }
        AbcCommand::Render(RenderInput {
            input,
            output,
            tune,
            to,
        }) => {
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
            let (part, from) = reading(tune)?;
            // Standard output is MIDI here unless --to says, as it always was.
            let to = to.or((output == Path::new("-")).then_some(Format::Midi));
            let to = part_format(&output, to, "duet abc render writes")?;
            write_from(
                ctx,
                &part,
                &output,
                to,
                &Origin::imported(from),
                Source::hand(),
            )
        }
        AbcCommand::Write(WriteInput {
            input,
            output,
            unit,
            dynamics,
            from,
        }) => {
            if input != Path::new("-") && same_file(&input, &output) {
                bail!("{} is the input; write makes a new file", output.display());
            }
            let from = part_format(&input, from, "duet abc write reads")?;
            let part = with_end(&read(&input, from, "duet abc write reads")?, config.end)?;
            let mut options = WriteOptions::default().with_dynamics(match dynamics {
                DynamicsArg::Refuse => Dynamics::Refuse,
                DynamicsArg::Nearest => Dynamics::Nearest,
            });
            if let Some(unit) = unit {
                let (n, d) = unit_of(&unit)?;
                options = options.with_unit(n, d);
            }
            let abc = duet_abc::write(&part, &options)?;
            if output == Path::new("-") {
                crate::io::stdout(&abc)
            } else {
                ctx.write(&output, abc.as_bytes())
            }
        }
    }
}
