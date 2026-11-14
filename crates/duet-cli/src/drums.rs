//! `duet drums`: drum grids, as .drums text or as the exact .drums.json and .drums.toml.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use duet_core::{Grid, Song};
use duet_drums::DrumGrid;

use crate::config::Config;
use crate::io::{Format, format_of, read, same_file, with_end, write};

#[derive(Subcommand)]
pub enum DrumsCommand {
    /// Render a drum grid as a song: MIDI, notefmt text, JSON or TOML, by the output's
    /// extension (.mid or .midi, .json, .toml, anything else is notefmt text).
    Render {
        /// The grid to read (.drums, .drums.json or .drums.toml), or - for .drums text
        /// on standard input.
        input: PathBuf,
        /// The file to write, or - for standard output. Must not be the input.
        output: PathBuf,
        #[arg(long, value_enum)]
        to: Option<Format>,
    },
    /// Read a song's drum track into a drum grid. Nothing is moved to fit: a note off
    /// the grid, or on a key no lane plays, is refused by name. Note lengths, controls
    /// and other tracks are not part of a grid, and are left out.
    Read {
        /// The song to read (MIDI, notefmt text, JSON or TOML), or - for standard input.
        input: PathBuf,
        /// The grid to write (.drums, .drums.json or .drums.toml), or - for .drums text
        /// on standard output. Must not be the input.
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
        /// Snap each velocity to the nearest of the text form's levels (g, x, X) as the
        /// track is read. Without it, a velocity that is not a level is kept exactly,
        /// and writing it as .drums text is refused.
        #[arg(long)]
        snap_levels: bool,
        #[arg(long, value_enum)]
        from: Option<Format>,
    },
    /// Write a copy of a drum grid with every velocity snapped to the nearest of the text
    /// form's levels (g, x, X). The input is never changed.
    Snap {
        /// The grid to read, or - for .drums text on standard input.
        input: PathBuf,
        /// The grid to write, or - for .drums text on standard output. Must not be the
        /// input.
        output: PathBuf,
    },
    /// List the lanes of the gm map, as config extends or changes it: each name, its
    /// key, and its aliases.
    Lanes,
}

/// The forms a drum grid is stored in, by extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridForm {
    Text,
    Json,
    Toml,
}

/// `.drums.json` and `.drums.toml` (or any `.json` and `.toml`) are the exact forms;
/// anything else, and `-`, is .drums text.
fn form_of(path: &Path) -> GridForm {
    if path == Path::new("-") {
        return GridForm::Text;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => GridForm::Json,
        Some("toml") => GridForm::Toml,
        _ => GridForm::Text,
    }
}

/// Reads a grid in a form. As .drums text, `map gm` means General MIDI with only the lanes
/// and aliases config adds, and anything config sets that does not apply is said.
/// `duet convert` and the `duet drums` commands all read grids here.
pub fn read_grid(path: &Path, form: GridForm, config: &Config) -> Result<DrumGrid> {
    let text = if path == Path::new("-") {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading standard input")?;
        buf
    } else {
        fs::read_to_string(path).with_context(|| path.display().to_string())?
    };
    let grid = match form {
        GridForm::Text => {
            for notice in &config.text_notices {
                eprintln!("duet: {notice}");
            }
            DrumGrid::from_text_in(&text, &config.text_lanes)?
        }
        GridForm::Json => normalized(serde_json::from_str(&text)?)?,
        GridForm::Toml => normalized(toml::from_str(&text)?)?,
    };
    Ok(grid)
}

fn normalized(mut grid: DrumGrid) -> duet_drums::Result<DrumGrid> {
    grid.normalize()?;
    Ok(grid)
}

/// Writes a grid in a form. `duet convert` and the `duet drums` commands all write grids
/// here.
pub fn write_grid(grid: &DrumGrid, path: &Path, form: GridForm) -> Result<()> {
    let bytes = match form {
        GridForm::Text => grid.to_text()?.into_bytes(),
        GridForm::Json => {
            let mut s = serde_json::to_string_pretty(grid)?;
            s.push('\n');
            s.into_bytes()
        }
        GridForm::Toml => toml::to_string(grid)?.into_bytes(),
    };
    if path == Path::new("-") {
        io::stdout()
            .write_all(&bytes)
            .context("writing standard output")?;
    } else {
        fs::write(path, bytes).with_context(|| path.display().to_string())?;
    }
    Ok(())
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

/// Reads a song's drum track into the grid to be written in `form`: the song runs to its
/// configured default end when it names none, nothing is moved to fit, a velocity off the
/// levels is refused for the text form unless snapped, and what is left out is said on
/// standard error. `duet convert` and `duet drums read` both read drum tracks here.
pub fn grid_of_song(
    song: &Song,
    options: &ReadOptions,
    form: GridForm,
    config: &Config,
) -> Result<DrumGrid> {
    // A song that names no end runs to the configured default's.
    let song = with_end(song, config.end)?;
    let track = options.track.as_deref();
    let mut drums = DrumGrid::from_song(&song, options.grid, &config.lanes, track, options.bars)?;
    if options.snap_levels {
        drums = drums.snapped_to_levels();
    } else if form == GridForm::Text
        && let Some((lane, bar, step, vel)) = drums.first_off_level()
    {
        bail!(
            "{lane}, bar {bar}, step {step}: velocity {vel} is not one of the .drums \
             text levels; pass --snap-levels to snap velocities to the levels, or \
             write the exact form (.drums.json or .drums.toml)"
        );
    }
    report_left_out(&song, track, &drums);
    Ok(drums)
}

fn refuse_overwrite(input: &Path, output: &Path, what: &str) -> Result<()> {
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
            let to = to.unwrap_or_else(|| format_of(&output, Format::Text));
            write(
                &read_grid(&input, form_of(&input), config)?.to_song()?,
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
            let from = from.unwrap_or_else(|| format_of(&input, Format::Midi));
            let options = ReadOptions {
                grid,
                track,
                bars,
                snap_levels,
            };
            let form = form_of(&output);
            let drums = grid_of_song(&read(&input, from)?, &options, form, config)?;
            write_grid(&drums, &output, form)
        }
        DrumsCommand::Snap { input, output } => {
            refuse_overwrite(&input, &output, "snap")?;
            let grid = read_grid(&input, form_of(&input), config)?;
            let snapped = grid.snapped_to_levels();
            let moved = moved_velocities(&grid, &snapped);
            eprintln!("duet: snapped {moved} velocities to the levels");
            write_grid(&snapped, &output, form_of(&output))
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

fn moved_velocities(a: &DrumGrid, b: &DrumGrid) -> usize {
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
        std::fs::write(dir.join("take.txt"), song).expect("write");
        let p = |name: &str| dir.join(name).to_str().expect("utf-8").to_owned();
        let read = |out: &str, snap: bool| {
            let (take, out) = (p("take.txt"), p(out));
            let mut args = vec![
                "drums",
                "read",
                take.as_str(),
                out.as_str(),
                "--grid",
                "1/16",
            ];
            if snap {
                args.push("--snap-levels");
            }
            duet(&args)
        };

        let err = read("take.drums", false).expect_err("100 is not a level");
        assert!(err.to_string().contains("--snap-levels"), "{err}");
        read("take.drums.json", false).expect("the exact form keeps any velocity");
        let exact = std::fs::read_to_string(dir.join("take.drums.json")).expect("json");
        assert!(exact.contains("100") && exact.contains("64"), "{exact}");

        read("snapped.drums", true).expect("snapped as read");
        duet(&[
            "drums",
            "snap",
            &p("take.drums.json"),
            &p("snapped-2.drums"),
        ])
        .expect("snap");
        let a = std::fs::read_to_string(dir.join("snapped.drums")).expect("a");
        let b = std::fs::read_to_string(dir.join("snapped-2.drums")).expect("b");
        assert_eq!(a, b, "--snap-levels and drums snap agree");
        // 100 is nearest x (90); 64 is nearest g (40).
        assert!(a.contains("kick  | x"), "{a}");
        assert!(a.contains("snare | . . . . g"), "{a}");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
