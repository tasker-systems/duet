//! `duet drums`: drum grids, as drum-grid documents (`.duet`).
//!
//! The grid side of each command is a drum-grid document: a `.duet`, read by its envelope, or
//! `-` for one on standard input or output. A grid's text is a view of one: `duet view --as
//! grid` writes it and `duet apply` reads it, and any other name on the grid side is refused
//! ([`grid_path`]). A document holds any velocity; the view shows only the levels.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use duet_core::{Grid, Part};
use duet_drums::DrumGrid;

use crate::Context;
use crate::config::Config;
use crate::io::{
    Format, FormatParser, generated, grid_document, grid_path, part_format, read,
    read_grid_document, read_grid_with_digest, same_file, with_end, write, write_bytes,
};
use clap::Args;
use schemars::JsonSchema;

#[derive(Debug, Subcommand)]
pub enum DrumsCommand {
    /// Render a drum grid as a part: a part document (.duet), MIDI (.mid or .midi) or ABC
    /// (.abc) by the output's name, or a part document on standard output.
    Render(RenderInput),
    /// Read a part's drum track into a drum grid. Nothing is moved to fit: a note off
    /// the grid, or on a key no lane plays, is refused by name. Note lengths, controls
    /// and other tracks are not part of a grid, and are left out.
    Read(ReadInput),
    /// Write a copy of a drum-grid document with every velocity snapped to the nearest of the
    /// grid view's levels (g, x, X), and say how many moved. The input is never changed.
    Snap(SnapInput),
    /// List the lanes of the gm map, as config extends or changes it: each name, its
    /// key, and its aliases.
    Lanes(LanesInput),
}

/// Render a drum grid as a part: a part document (.duet), MIDI (.mid or .midi) or ABC
/// (.abc) by the output's name, or a part document on standard output.
#[derive(Debug, Args, JsonSchema)]
pub struct RenderInput {
    /// The grid: a drum-grid document (.duet), or - for one on standard input.
    pub input: PathBuf,
    /// The part to write, or - for a part document on standard output (another format
    /// with --to). Must not be the input.
    pub output: PathBuf,
    #[arg(long, value_parser = FormatParser)]
    pub to: Option<Format>,
}

/// Read a part's drum track into a drum grid. Nothing is moved to fit: a note off
/// the grid, or on a key no lane plays, is refused by name. Note lengths, controls
/// and other tracks are not part of a grid, and are left out.
#[derive(Debug, Args, JsonSchema)]
pub struct ReadInput {
    /// The part to read (a .duet document, MIDI or ABC by its name), or - for standard
    /// input: a document if it starts with {, MIDI, or ABC with --from abc.
    pub input: PathBuf,
    /// The grid to write: a drum-grid document (.duet), which keeps any velocity, or - for
    /// one on standard output. Must not be the input.
    pub output: PathBuf,
    /// The step, such as 1/16 for sixteenths or 1/8t for triplet eighths.
    #[arg(long)]
    pub grid: Grid,
    /// The drum track's name. Needed only when the part has no single track on
    /// channel 10.
    #[arg(long)]
    pub track: Option<String>,
    /// How many bars the grid runs to. Without it, the grid runs to the part's end:
    /// the one it names, or else the bar line after its last bar with a note or
    /// control. A hit or a tempo, meter or key change past the bars is refused.
    #[arg(long)]
    pub bars: Option<u32>,
    /// Snap each velocity to the nearest of the grid view's levels (g, x, X) as the track
    /// is read. Without it, every velocity is kept.
    #[arg(long)]
    pub snap_levels: bool,
    #[arg(long, value_parser = FormatParser)]
    pub from: Option<Format>,
}

/// Write a copy of a drum-grid document with every velocity snapped to the nearest of the
/// grid view's levels (g, x, X), and say how many moved. The input is never changed.
#[derive(Debug, Args, JsonSchema)]
pub struct SnapInput {
    /// The grid to read, as for `render`.
    pub input: PathBuf,
    /// The grid to write, as for `read`. Must not be the input.
    pub output: PathBuf,
}

/// List the lanes of the gm map, as config extends or changes it: each name, its
/// key, and its aliases.
#[derive(Debug, Args, JsonSchema)]
pub struct LanesInput {}

/// Reads a drum-grid document ([`read_grid_document`]). `duet convert` and the `duet drums`
/// commands all read grids here.
pub fn read_grid(path: &Path, what: &str) -> Result<DrumGrid> {
    read_grid_document(path, what)
}

/// Writes a grid as a drum-grid document, which holds any velocity, to a `.duet` or to standard
/// output. `duet convert` and the `duet drums` commands all write grids here.
pub fn write_grid(ctx: &Context, grid: &DrumGrid, path: &Path, what: &str) -> Result<()> {
    grid_path(path, what)?;
    write_bytes(ctx, path, grid_document(grid)?.as_bytes())
}

/// How to read a part's drum track into a grid: `duet drums read`'s flags, which
/// `duet convert` takes too.
#[derive(Debug)]
pub struct ReadOptions {
    /// The step.
    pub grid: Grid,
    /// The drum track's name, when it is not the one track on the map's channel.
    pub track: Option<String>,
    /// How many bars the grid runs to, when not to the part's end.
    pub bars: Option<u32>,
    /// Whether to snap velocities to the text levels.
    pub snap_levels: bool,
}

/// Reads a part's drum track into a grid: the part runs to its configured default end when it
/// names none, nothing is moved to fit, every velocity is kept unless `snap_levels` snaps them
/// to the text's levels, and what is left out is said on standard error. `duet convert`,
/// `duet drums read` and `duet capture` all read drum tracks here.
pub fn grid_of_part(part: &Part, options: &ReadOptions, config: &Config) -> Result<DrumGrid> {
    // A part that names no end runs to the configured default's.
    let part = with_end(part, config.end)?;
    let track = options.track.as_deref();
    let drums = DrumGrid::from_part(&part, options.grid, &config.lanes, track, options.bars)?;
    let drums = if options.snap_levels {
        drums.snapped_to_levels()
    } else {
        drums
    };
    report_left_out(&part, track, &drums);
    Ok(drums)
}

/// The grid ready to show as its view's text: snapped to the levels when asked, and otherwise
/// refused when a velocity is off them. Nothing snaps unless asked.
pub fn levels_for(grid: DrumGrid, snap_levels: bool) -> Result<DrumGrid> {
    if snap_levels {
        return Ok(grid.snapped_to_levels());
    }
    if let Some((lane, bar, step, vel)) = grid.first_off_level() {
        bail!(
            "{lane}, bar {bar}, step {step}: velocity {vel} is not one of the grid view's \
             levels; pass --snap-levels to show velocities snapped to the levels (apply --onto \
             keeps each exact where its level is unchanged)"
        );
    }
    Ok(grid)
}

pub fn refuse_overwrite(input: &Path, output: &Path, what: &str) -> Result<()> {
    if input != Path::new("-") && same_file(input, output) {
        bail!(
            "{} is the input; {what} writes a new file and never changes its input",
            output.display()
        );
    }
    Ok(())
}

pub fn run(ctx: &Context, command: DrumsCommand) -> Result<()> {
    let config = ctx.config();
    match command {
        DrumsCommand::Render(RenderInput { input, output, to }) => {
            refuse_overwrite(&input, &output, "render")?;
            let to = part_format(&output, to, "render writes")?;
            // The part is generated from the grid, naming it.
            let (grid, digest) = read_grid_with_digest(&input, "render reads")?;
            write(
                ctx,
                &grid.to_part()?,
                &output,
                to,
                generated("drums render", [digest]),
            )
        }
        DrumsCommand::Read(ReadInput {
            input,
            output,
            grid,
            track,
            bars,
            snap_levels,
            from,
        }) => {
            refuse_overwrite(&input, &output, "read")?;
            let from = part_format(&input, from, "read reads")?;
            grid_path(&output, "read writes")?;
            let options = ReadOptions {
                grid,
                track,
                bars,
                snap_levels,
            };
            let drums = grid_of_part(&read(&input, from, "read reads")?, &options, config)?;
            write_grid(ctx, &drums, &output, "read writes")
        }
        DrumsCommand::Snap(SnapInput { input, output }) => {
            refuse_overwrite(&input, &output, "snap")?;
            grid_path(&output, "snap writes")?;
            let grid = read_grid(&input, "snap reads")?;
            let snapped = grid.snapped_to_levels();
            let moved = moved_velocities(&grid, &snapped);
            eprintln!("duet: snapped {moved} velocities to the levels");
            write_grid(ctx, &snapped, &output, "snap writes")
        }
        DrumsCommand::Lanes(LanesInput {}) => {
            let map = &config.lanes;
            let mut out = format!("# lane map {}, channel {}\n", map.name(), map.channel());
            let width = map.lanes().map(|l| l.name.len()).max().unwrap_or(1);
            let groups = map.lanes().map(|l| l.group.len()).max().unwrap_or(1);
            for lane in map.lanes() {
                let line = format!(
                    "{:<width$}  {:>3}  {:<groups$}",
                    lane.name, lane.key, lane.group
                );
                if lane.aliases.is_empty() {
                    out.push_str(line.trim_end());
                } else {
                    out.push_str(&format!("{line}  also {}", lane.aliases.join(", ")));
                }
                out.push('\n');
            }
            io::stdout()
                .write_all(out.as_bytes())
                .context("writing standard output")
        }
    }
}

/// How many velocities differ between a grid and its snapped copy.
pub fn moved_velocities(a: &DrumGrid, b: &DrumGrid) -> usize {
    a.vel
        .data()
        .iter()
        .zip(b.vel.data())
        .filter(|(x, y)| x != y)
        .count()
}

/// Says on standard error what reading a drum track left out, so nothing is dropped
/// silently. Note lengths are always dropped, and the help says so.
fn report_left_out(part: &Part, track: Option<&str>, grid: &DrumGrid) {
    let drums = match track {
        Some(name) => part.tracks.iter().find(|t| t.name == name),
        None => part.tracks.iter().find(|t| t.channel == grid.channel),
    };
    let others = part.tracks.len().saturating_sub(1);
    let controls = drums.map_or(0, |t| t.controls.len());
    if others > 0 || controls > 0 {
        eprintln!(
            "duet: read the drum track; left out {others} other track(s), and {controls} \
             control(s) on the drum track"
        );
    }
}
