//! `duet song`: work on a song in the notefmt hub, whatever format it is stored in.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Subcommand;
use duet_core::Grid;

use crate::config::Config;
use crate::io::{Format, format_of, read, same_file, write};

#[derive(Subcommand)]
pub enum SongCommand {
    /// Print a file as notefmt text (or another format with --to).
    Show {
        /// The file to read, or - for standard input.
        input: PathBuf,
        #[arg(long, value_enum)]
        from: Option<Format>,
        #[arg(long, value_enum, default_value = "text")]
        to: Format,
    },
    /// Write a copy with every note snapped to a grid. The input is never changed.
    Quantize {
        /// The file to read, or - for standard input.
        input: PathBuf,
        /// The file to write, or - for standard output. Must not be the input.
        output: PathBuf,
        /// The grid, such as 1/16 for sixteenths or 1/8t for triplet eighths.
        #[arg(long)]
        grid: Grid,
        #[arg(long, value_enum)]
        from: Option<Format>,
        #[arg(long, value_enum)]
        to: Option<Format>,
    },
}

pub fn run(command: SongCommand, config: &Config) -> Result<()> {
    match command {
        SongCommand::Show { input, from, to } => {
            let from = from.unwrap_or_else(|| format_of(&input, Format::Midi));
            write(&read(&input, from)?, Path::new("-"), to, config.end)
        }
        SongCommand::Quantize {
            input,
            output,
            grid,
            from,
            to,
        } => {
            if input != Path::new("-") && same_file(&input, &output) {
                anyhow::bail!(
                    "{} is the input; quantize writes a new file and never changes its input",
                    output.display()
                );
            }
            let from = from.unwrap_or_else(|| format_of(&input, Format::Text));
            let to = to.unwrap_or_else(|| format_of(&output, Format::Text));
            write(
                &read(&input, from)?.quantized(grid)?,
                &output,
                to,
                config.end,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::io::{Format, read};
    use crate::tests::run_hermetic;

    #[test]
    fn quantize_never_writes_over_its_input() {
        let dir = std::env::temp_dir().join(format!("duet-quantize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let take = dir.join("take.txt");
        let played = "[track \"P\" ch1]\n1.1.10 C4 0.48 80\n";
        std::fs::write(&take, played).expect("write");
        let cli = |out: &std::path::Path| {
            run_hermetic(&[
                "song",
                "quantize",
                take.to_str().expect("utf-8 path"),
                out.to_str().expect("utf-8 path"),
                "--grid",
                "1/8",
            ])
        };

        let err = cli(&take).expect_err("writing over the input is refused");
        assert!(err.to_string().contains("never changes its input"), "{err}");
        let kept = std::fs::read_to_string(&take).expect("read");
        assert_eq!(kept, played, "the input is untouched");

        let out = dir.join("take-q.txt");
        cli(&out).expect("quantize to a new file");
        let q = read(&out, Format::Text).expect("read the output");
        assert_eq!(q.tracks[0].notes[0].at.to_string(), "1.1.0");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
