//! `convert`: between any two formats, since every format passes through the same `Part`.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;
use duet_core::Grid;
use schemars::JsonSchema;

use crate::Context;
use crate::drums::{self, ReadOptions};
use crate::io::{self, Format, FormatParser, Loaded, Origin, part_format, read_any, write_from};

/// The input of `convert`.
#[derive(Debug, Args, JsonSchema)]
pub struct ConvertInput {
    /// The file to read, or - for standard input.
    pub input: PathBuf,
    /// The file to write, or - for standard output.
    pub output: PathBuf,
    #[arg(long, value_parser = FormatParser)]
    pub from: Option<Format>,
    #[arg(long, value_parser = FormatParser)]
    pub to: Option<Format>,
    /// Writing a drum grid: the step, such as 1/16 for sixteenths or 1/8t for triplet
    /// eighths. Required to write a grid from a part; from a grid, it re-steps the grid
    /// through a part, which drops unplayed lanes. Refused for any other output.
    #[arg(long)]
    pub grid: Option<Grid>,
    /// Writing a drum grid: the drum track's name, as for `duet drums read`.
    #[arg(long)]
    pub track: Option<String>,
    /// Writing a drum grid: how many bars it runs to, as for `duet drums read`.
    #[arg(long)]
    pub bars: Option<u32>,
    /// Writing a drum grid: snap each velocity to the text levels, as for
    /// `duet drums read`.
    #[arg(long)]
    pub snap_levels: bool,
}

/// Reads any format and writes any other. A grid is read by the code `duet drums render`
/// reads with, and written from a part by the code `duet drums read` writes with, so they
/// never disagree. Writing a grid from a part needs `--grid`. A grid written from a grid
/// with no `--grid` is copied as it is; with `--grid`, it goes through a part at that step.
/// A grid flag that would do nothing is refused, so a flag is never silently ignored.
///
/// A `.duet` output keeps what was read: a drum-grid document read is written as one, as is a
/// grid made from a part with `--grid`; a part is written as a part document. `duet drums
/// render` makes a part of a grid.
pub fn run(ctx: &Context, input: ConvertInput) -> Result<()> {
    let ConvertInput {
        input,
        output,
        from,
        to,
        grid,
        track,
        bars,
        snap_levels,
    } = input;
    let (input, output) = (input.as_path(), output.as_path());
    let config = ctx.config();
    let any_grid_flag = grid.is_some() || track.is_some() || bars.is_some() || snap_levels;
    let format = part_format(input, from, "convert reads")?;
    let read = read_any(input, format, "convert reads")?;
    let to = part_format(output, to, "convert writes")?;
    // Whether a drum-grid document is written.
    let to_grid = to == Format::Duet && (matches!(read, Loaded::Grid(..)) || any_grid_flag);
    // Grid to grid with no step named: a straight copy, read and written as `drums snap`
    // does, so nothing a part cannot hold (an unplayed lane, the grid's own levels, in a
    // document any velocity) is lost.
    if let (Loaded::Grid(source, _), true, None) = (&read, to_grid, grid) {
        let given: Vec<&str> = [(track.is_some(), "--track"), (bars.is_some(), "--bars")]
            .into_iter()
            .filter_map(|(set, flag)| set.then_some(flag))
            .collect();
        if !given.is_empty() {
            bail!(
                "{} {} for reading a part's drum track, and need --grid; without it, a grid \
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
        return drums::write_grid(ctx, &copy, output, "convert writes");
    }
    // A part read keeps its source; one made of a grid was generated from it.
    let (part, origin, source) = match read {
        Loaded::Part(part, origin) => {
            let source = origin.source();
            (part, origin, source)
        }
        Loaded::Grid(grid, digest) => (
            grid.to_part()?,
            Origin::default(),
            io::generated("convert", [digest]),
        ),
    };
    if !to_grid {
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
                 drum-grid document (.duet), and `duet view --as grid` shows a part's drum track \
                 as a grid's text",
                given.join(", "),
                if given.len() == 1 { "is" } else { "are" },
                output.display()
            );
        }
        return write_from(ctx, &part, output, to, &origin, source);
    }
    let Some(grid) = grid else {
        bail!(
            "writing a drum grid from a part needs --grid, the step, such as --grid 1/16; duet \
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
        ctx,
        &drums::grid_of_part(&part, &options, config)?,
        output,
        "convert writes",
    )
}
