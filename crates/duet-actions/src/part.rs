//! `duet part`: work on a part in the notefmt hub, whatever format it is stored in.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Subcommand;
use duet_core::Grid;

use crate::Context;
use crate::io::{
    Format, FormatParser, part_format, read_with_from, same_file, with_end, write_bytes, write_from,
};
use clap::Args;
use schemars::JsonSchema;

#[derive(Debug, Subcommand)]
pub enum PartCommand {
    /// Print a part as notefmt text (or in another format with --to).
    Show(ShowInput),
    /// Write a copy with every note snapped to a grid. The input is never changed.
    Quantize(QuantizeInput),
}

/// Print a part as notefmt text (or in another format with --to).
#[derive(Debug, Args, JsonSchema)]
pub struct ShowInput {
    /// The part to read: a .duet document, MIDI or ABC by its name, or - for standard
    /// input (a document if it starts with {, MIDI, or ABC with --from abc).
    pub input: PathBuf,
    #[arg(long, value_parser = FormatParser)]
    pub from: Option<Format>,
    /// Print the part in this format instead of as notefmt text.
    #[arg(long, value_parser = FormatParser)]
    pub to: Option<Format>,
}

/// Write a copy with every note snapped to a grid. The input is never changed.
#[derive(Debug, Args, JsonSchema)]
pub struct QuantizeInput {
    /// The part to read, by its name or on standard input as for `part show`.
    pub input: PathBuf,
    /// The part to write, by its name, or - for a part document on standard output
    /// (another format with --to). Must not be the input.
    pub output: PathBuf,
    /// The grid, such as 1/16 for sixteenths or 1/8t for triplet eighths.
    #[arg(long)]
    pub grid: Grid,
    #[arg(long, value_parser = FormatParser)]
    pub from: Option<Format>,
    #[arg(long, value_parser = FormatParser)]
    pub to: Option<Format>,
}

pub fn run(ctx: &Context, command: PartCommand) -> Result<()> {
    let config = ctx.config();
    match command {
        PartCommand::Show(ShowInput { input, from, to }) => {
            let from = part_format(&input, from, "part show reads")?;
            let (part, origin) = read_with_from(&input, from, "part show reads")?;
            match to {
                Some(to) => {
                    let source = origin.source();
                    write_from(ctx, &part, Path::new("-"), to, &origin, source)
                }
                None => write_bytes(
                    ctx,
                    Path::new("-"),
                    with_end(&part, config.end)?.to_text().as_bytes(),
                ),
            }
        }
        PartCommand::Quantize(QuantizeInput {
            input,
            output,
            grid,
            from,
            to,
        }) => {
            if input != Path::new("-") && same_file(&input, &output) {
                anyhow::bail!(
                    "{} is the input; quantize writes a new file and never changes its input",
                    output.display()
                );
            }
            let from = part_format(&input, from, "quantize reads")?;
            let to = part_format(&output, to, "quantize writes")?;
            // Quantizing keeps where the part came from: the notices still name what the
            // import did not carry. It is an operation, so the part it writes is generated,
            // naming the document it read.
            let (part, origin) = read_with_from(&input, from, "quantize reads")?;
            let source = origin.generated("part quantize");
            write_from(ctx, &part.quantized(grid)?, &output, to, &origin, source)
        }
    }
}
