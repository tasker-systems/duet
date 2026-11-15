//! `duet drums`: drum grids, as their text.
//!
//! Until drum-grid documents exist, a grid is read and written as its text whatever the file is
//! named: the grid side of each command means a grid. A name that is something else is refused
//! ([`grid_path`]), and a `.duet` is read by its envelope and refused naming its kind.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use duet_core::{Grid, Song};
use duet_drums::DrumGrid;

use crate::config::Config;
use crate::io::{
    Format, grid_path, read, read_grid_text, same_file, song_format, with_end, write, write_bytes,
};

#[derive(Subcommand)]
pub enum DrumsCommand {
    /// Render a drum grid's text as a song: a document (.duet), MIDI (.mid or .midi) or ABC
    /// (.abc) by the output's name, or notefmt text with --to text or on standard output.
    Render {
        /// The grid's text, or - for standard input.
        input: PathBuf,
        /// The song to write, or - for notefmt text on standard output. Must not be the input.
        output: PathBuf,
        #[arg(long, value_enum)]
        to: Option<Format>,
    },
    /// Read a song's drum track into a drum grid. Nothing is moved to fit: a note off
    /// the grid, or on a key no lane plays, is refused by name. Note lengths, controls
    /// and other tracks are not part of a grid, and are left out.
    Read {
        /// The song to read (a .duet document, MIDI or ABC by its name, or notefmt text with
        /// --from text), or - for standard input: a document if it starts with {, else MIDI.
        input: PathBuf,
        /// The grid's text to write, or - for standard output. Must not be the input.
        output: PathBuf,
        /// The step, such as 1/16 for sixteenths or 1/8t for triplet eighths.
        #[arg(long)]
        grid: Grid,
        /// The drum track's name. Needed only when the song has no single track on
        /// channel 10.
        #[arg(long)]
        track: Option<String>,
        /// How many bars the grid runs to. Without it, the grid runs to the song's end:
        /// the one it names, or else the bar line after its last bar with a note or
        /// control. A hit or a tempo, meter or key change past the bars is refused.
        #[arg(long)]
        bars: Option<u32>,
        /// Snap each velocity to the nearest of the text's levels (g, x, X) as the track is
        /// read. Without it, a velocity that is not a level is refused, since the text holds
        /// only its levels.
        #[arg(long)]
        snap_levels: bool,
        #[arg(long, value_enum)]
        from: Option<Format>,
    },
    /// Write a copy of a drum grid's text with every velocity snapped to the nearest of the
    /// text's levels (g, x, X). The input is never changed.
    Snap {
        /// The grid's text to read, or - for standard input.
        input: PathBuf,
        /// The grid's text to write, or - for standard output. Must not be the input.
        output: PathBuf,
    },
    /// List the lanes of the gm map, as config extends or changes it: each name, its
    /// key, and its aliases.
    Lanes,
}

/// Reads a grid's text. `map gm` means General MIDI with only the lanes and aliases config
/// adds, and anything config sets that does not apply is said. `duet convert` and the
/// `duet drums` commands all read grids here.
pub fn read_grid(path: &Path, what: &str, config: &Config) -> Result<DrumGrid> {
    grid_path(path, what, false)?;
    let text = read_grid_text(path, what)?;
    for notice in &config.text_notices {
        eprintln!("duet: {notice}");
    }
    Ok(DrumGrid::from_text_in(&text, &config.text_lanes)?)
}

/// Writes a grid's text, refusing a velocity off the text's levels. `duet convert` and the
/// `duet drums` commands all write grids here.
pub fn write_grid(grid: &DrumGrid, path: &Path, what: &str) -> Result<()> {
    grid_path(path, what, true)?;
    let grid = levels_for(grid.clone(), false)?;
    write_bytes(path, grid.to_text()?.as_bytes())
}

/// How to read a song's drum track into a grid: `duet drums read`'s flags, which
/// `duet convert` takes too.
pub struct ReadOptions {
    /// The step.
    pub grid: Grid,
    /// The drum track's name, when it is not the one track on the map's channel.
    pub track: Option<String>,
    /// How many bars the grid runs to, when not to the song's end.
    pub bars: Option<u32>,
    /// Whether to snap velocities to the text levels.
    pub snap_levels: bool,
}

/// Reads a song's drum track into a grid: the song runs to its configured default end when it
/// names none, nothing is moved to fit, every velocity is kept unless `snap_levels` snaps them
/// to the text's levels, and what is left out is said on standard error. `duet convert`,
/// `duet drums read` and `duet capture` all read drum tracks here.
pub fn grid_of_song(song: &Song, options: &ReadOptions, config: &Config) -> Result<DrumGrid> {
    // A song that names no end runs to the configured default's.
    let song = with_end(song, config.end)?;
    let track = options.track.as_deref();
    let drums = DrumGrid::from_song(&song, options.grid, &config.lanes, track, options.bars)?;
    let drums = if options.snap_levels {
        drums.snapped_to_levels()
    } else {
        drums
    };
    report_left_out(&song, track, &drums);
    Ok(drums)
}

/// The grid ready to write as text: snapped to the text levels when asked, and otherwise
/// refused when a velocity is off them. Nothing snaps unless asked. Reading a drum track and
/// copying a grid both decide here.
pub fn levels_for(grid: DrumGrid, snap_levels: bool) -> Result<DrumGrid> {
    if snap_levels {
        return Ok(grid.snapped_to_levels());
    }
    if let Some((lane, bar, step, vel)) = grid.first_off_level() {
        bail!(
            "{lane}, bar {bar}, step {step}: velocity {vel} is not one of the drum grid \
             text's levels; pass --snap-levels to snap velocities to the levels"
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

pub fn run(command: DrumsCommand, config: &Config) -> Result<()> {
    match command {
        DrumsCommand::Render { input, output, to } => {
            refuse_overwrite(&input, &output, "render")?;
            let to = song_format(&output, to, Format::Text, "render writes")?;
            write(
                &read_grid(&input, "render reads", config)?.to_song()?,
                &output,
                to,
                config.end,
            )
        }
        DrumsCommand::Read {
            input,
            output,
            grid,
            track,
            bars,
            snap_levels,
            from,
        } => {
            refuse_overwrite(&input, &output, "read")?;
            let from = song_format(&input, from, Format::Midi, "read reads")?;
            grid_path(&output, "read writes", true)?;
            let options = ReadOptions {
                grid,
                track,
                bars,
                snap_levels,
            };
            let drums = grid_of_song(&read(&input, from, "read reads")?, &options, config)?;
            write_grid(&drums, &output, "read writes")
        }
        DrumsCommand::Snap { input, output } => {
            refuse_overwrite(&input, &output, "snap")?;
            grid_path(&output, "snap writes", true)?;
            let grid = read_grid(&input, "snap reads", config)?;
            let snapped = grid.snapped_to_levels();
            let moved = moved_velocities(&grid, &snapped);
            eprintln!("duet: snapped {moved} velocities to the levels");
            write_grid(&snapped, &output, "snap writes")
        }
        DrumsCommand::Lanes => {
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
    a.bars
        .iter()
        .zip(&b.bars)
        .flat_map(|(x, y)| x.vel.iter().flatten().zip(y.vel.iter().flatten()))
        .filter(|(x, y)| x != y)
        .count()
}

/// Says on standard error what reading a drum track left out, so nothing is dropped
/// silently. Note lengths are always dropped, and the help says so.
fn report_left_out(song: &Song, track: Option<&str>, grid: &DrumGrid) {
    let drums = match track {
        Some(name) => song.tracks.iter().find(|t| t.name == name),
        None => song.tracks.iter().find(|t| t.channel == grid.channel),
    };
    let others = song.tracks.len().saturating_sub(1);
    let controls = drums.map_or(0, |t| t.controls.len());
    if others > 0 || controls > 0 {
        eprintln!(
            "duet: read the drum track; left out {others} other track(s), and {controls} \
             control(s) on the drum track"
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::run_hermetic;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-drums-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn duet(args: &[&str]) -> anyhow::Result<()> {
        run_hermetic(args)
    }

    #[test]
    fn a_grid_renders_to_midi_and_reads_back_the_same() {
        let dir = temp("render");
        let grid = "grid 1/8\n\n[bars 1-2]\nhh    | x x x x x x x x | x x x x x x X . |\nsnare | . . X . . . X g | . . X . . . X . |\nkick  | X . . . X X . . | X . . . X . . . |\n";
        let input = dir.join("beat.drums");
        std::fs::write(&input, grid).expect("write");
        let p = |name: &str| dir.join(name).to_str().expect("utf-8").to_owned();
        duet(&["drums", "render", &p("beat.drums"), &p("beat.mid")]).expect("render");
        duet(&[
            "drums",
            "read",
            &p("beat.mid"),
            &p("back.drums"),
            "--grid",
            "1/8",
        ])
        .expect("read");
        let back = std::fs::read_to_string(dir.join("back.drums")).expect("read back");
        let want = duet_drums::DrumGrid::from_text(grid).expect("parse");
        assert_eq!(
            duet_drums::DrumGrid::from_text(&back).expect("parse back"),
            want
        );
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn off_level_velocities_are_refused_as_text_until_snapped() {
        let dir = temp("snap");
        let song = "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n1.2.0 D2 0.25 64\n";
        std::fs::write(dir.join("take.notes"), song).expect("write");
        let p = |name: &str| dir.join(name).to_str().expect("utf-8").to_owned();
        let read = |out: &str, snap: bool| {
            let (take, out) = (p("take.notes"), p(out));
            let mut args = vec![
                "drums",
                "read",
                take.as_str(),
                out.as_str(),
                "--grid",
                "1/16",
                "--from",
                "text",
            ];
            if snap {
                args.push("--snap-levels");
            }
            duet(&args)
        };

        let err = read("take.drums", false).expect_err("100 is not a level");
        assert!(err.to_string().contains("--snap-levels"), "{err}");
        assert!(!dir.join("take.drums").exists(), "nothing is written");

        read("snapped.drums", true).expect("snapped as read");
        duet(&["drums", "snap", &p("snapped.drums"), &p("snapped-2.drums")]).expect("snap");
        let a = std::fs::read_to_string(dir.join("snapped.drums")).expect("a");
        let b = std::fs::read_to_string(dir.join("snapped-2.drums")).expect("b");
        assert_eq!(a, b, "a snapped grid snaps to itself");
        // 100 is nearest x (90); 64 is nearest g (40).
        assert!(a.contains("kick  | x"), "{a}");
        assert!(a.contains("snare | . . . . g"), "{a}");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
