//! `duet part`: work on a part in the notefmt hub, whatever format it is stored in.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Subcommand;
use duet_core::Grid;

use crate::config::Config;
use crate::io::{
    Format, FormatParser, part_format, read_with_from, same_file, with_end, write_bytes, write_from,
};

#[derive(Subcommand)]
pub enum PartCommand {
    /// Print a part as notefmt text (or in another format with --to).
    Show {
        /// The part to read: a .duet document, MIDI or ABC by its name, or - for standard
        /// input (a document if it starts with {, MIDI, or ABC with --from abc).
        input: PathBuf,
        #[arg(long, value_parser = FormatParser)]
        from: Option<Format>,
        /// Print the part in this format instead of as notefmt text.
        #[arg(long, value_parser = FormatParser)]
        to: Option<Format>,
    },
    /// Write a copy with every note snapped to a grid. The input is never changed.
    Quantize {
        /// The part to read, by its name or on standard input as for `part show`.
        input: PathBuf,
        /// The part to write, by its name, or - for a part document on standard output
        /// (another format with --to). Must not be the input.
        output: PathBuf,
        /// The grid, such as 1/16 for sixteenths or 1/8t for triplet eighths.
        #[arg(long)]
        grid: Grid,
        #[arg(long, value_parser = FormatParser)]
        from: Option<Format>,
        #[arg(long, value_parser = FormatParser)]
        to: Option<Format>,
    },
}

pub fn run(command: PartCommand, config: &Config) -> Result<()> {
    match command {
        PartCommand::Show { input, from, to } => {
            let from = part_format(&input, from, "part show reads")?;
            let (part, origin) = read_with_from(&input, from, "part show reads")?;
            match to {
                Some(to) => {
                    let source = origin.source();
                    write_from(&part, Path::new("-"), to, config.end, &origin, source)
                }
                None => write_bytes(
                    Path::new("-"),
                    with_end(&part, config.end)?.to_text().as_bytes(),
                ),
            }
        }
        PartCommand::Quantize {
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
            let from = part_format(&input, from, "quantize reads")?;
            let to = part_format(&output, to, "quantize writes")?;
            // Quantizing keeps where the part came from: the notices still name what the
            // import did not carry. It is an operation, so the part it writes is generated,
            // naming the document it read.
            let (part, origin) = read_with_from(&input, from, "quantize reads")?;
            let source = origin.generated("part quantize");
            write_from(
                &part.quantized(grid)?,
                &output,
                to,
                config.end,
                &origin,
                source,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use duet_core::Part;

    use crate::io::{Format, read};
    use crate::tests::run_hermetic;

    #[test]
    fn quantize_never_writes_over_its_input() {
        let dir = std::env::temp_dir().join(format!("duet-quantize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let take = dir.join("take.duet");
        let played = "[track \"P\" ch1]\n1.1.10 C4 0.48 80\n";
        let played = crate::io::part_document(Part::from_text(played).expect("parse"));
        std::fs::write(&take, &played).expect("write");
        let cli = |out: &std::path::Path| {
            run_hermetic(&[
                "part",
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

        let out = dir.join("take-q.duet");
        cli(&out).expect("quantize to a new file");
        let q = read(&out, Format::Duet, "reads").expect("read the output");
        assert_eq!(q.tracks[0].notes[0].at.to_string(), "1.1.0");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
