//! Tests that run the CLI in-process: clap's parse, the action, and its context, with no
//! config but what the test gives.

use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use duet_actions::config::Dirs;

use crate::{Cli, run};

/// Runs `duet <args>` with no config at all: a fresh, empty user config directory, no project
/// `.duet` and no project, passed in rather than set in the process's environment, so a
/// developer's own duet.toml, or a `.duet` above the checkout, cannot change a test.
pub(crate) fn run_hermetic(args: &[&str]) -> anyhow::Result<()> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let user = std::env::temp_dir().join(format!(
        "duet-no-config-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let dirs = Dirs {
        user: Some((user, "the test's empty config directory")),
        project: None,
    };
    run(
        Cli::parse_from(std::iter::once("duet").chain(args.iter().copied())),
        &dirs,
        None,
    )
}

mod part {
    use duet_core::Part;

    use super::run_hermetic;
    use duet_actions::io::{Format, read};

    #[test]
    fn quantize_never_writes_over_its_input() {
        let dir = std::env::temp_dir().join(format!("duet-quantize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let take = dir.join("take.duet");
        let played = "[track \"P\" ch1]\n1.1.10 C4 0.48 80\n";
        let played = duet_actions::io::part_document(Part::from_text(played).expect("parse"));
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

mod drums {
    use std::path::{Path, PathBuf};

    use duet_core::Part;
    use duet_drums::DrumGrid;

    use super::run_hermetic;
    use duet_actions::io::{grid_document, read_grid_document};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-drums-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn duet(args: &[&str]) -> anyhow::Result<()> {
        run_hermetic(args)
    }

    fn grid_at(path: &Path) -> DrumGrid {
        read_grid_document(path, "reads").expect("a drum-grid document")
    }

    #[test]
    fn a_grid_renders_to_midi_and_reads_back_the_same() {
        let dir = temp("render");
        let grid = "grid 1/8\n\n[bars 1-2]\nhh    | x x x x x x x x | x x x x x x X . |\nsnare | . . X . . . X g | . . X . . . X . |\nkick  | X . . . X X . . | X . . . X . . . |\n";
        let want = DrumGrid::from_text(grid).expect("parse");
        std::fs::write(
            dir.join("beat.duet"),
            grid_document(&want).expect("document"),
        )
        .expect("write");
        let p = |name: &str| dir.join(name).to_str().expect("utf-8").to_owned();
        duet(&["drums", "render", &p("beat.duet"), &p("beat.mid")]).expect("render");
        duet(&[
            "drums",
            "read",
            &p("beat.mid"),
            &p("back.duet"),
            "--grid",
            "1/8",
        ])
        .expect("read");
        assert_eq!(grid_at(&dir.join("back.duet")), want);
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn off_level_velocities_are_refused_in_the_view_until_snapped() {
        let dir = temp("snap");
        let part = "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n1.2.0 D2 0.25 64\n";
        let part = Part::from_text(part).expect("parse");
        std::fs::write(dir.join("take.duet"), duet_actions::io::part_document(part))
            .expect("write");
        let p = |name: &str| dir.join(name).to_str().expect("utf-8").to_owned();
        let read = |out: &str, snap: bool| {
            let (take, out) = (p("take.duet"), p(out));
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
        let view = |of: &str, out: &str| duet(&["view", &p(of), "--as", "grid", "-o", &p(out)]);

        read("exact.duet", false).expect("a document keeps 100");
        let exact = grid_at(&dir.join("exact.duet"));
        assert_eq!(exact.vel.data().iter().filter(|&&v| v == 100).count(), 1);
        let err = view("exact.duet", "exact.drums").expect_err("100 is not a level");
        assert!(err.to_string().contains("--snap-levels"), "{err}");
        assert!(!dir.join("exact.drums").exists(), "nothing is written");

        read("snapped.duet", true).expect("snapped as read");
        duet(&["drums", "snap", &p("snapped.duet"), &p("snapped-2.duet")]).expect("snap");
        let (a, b) = (
            grid_at(&dir.join("snapped.duet")),
            grid_at(&dir.join("snapped-2.duet")),
        );
        assert_eq!(a, b, "a snapped grid snaps to itself");
        view("snapped.duet", "snapped.drums").expect("a snapped grid has a view");
        let a = std::fs::read_to_string(dir.join("snapped.drums")).expect("a");
        // 100 is nearest x (90); 64 is nearest g (40).
        assert!(a.contains("kick  | x"), "{a}");
        assert!(a.contains("snare | . . . . g"), "{a}");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}

/// The registry witness (bus task 2): every registered action has a CLI path, and every
/// subcommand is an action or one of the readers, which change no file. That the readers write
/// nothing is witnessed end to end, in a project, by `duet-e2e`'s
/// `readers_write_nothing_in_a_project`.
mod registry {
    use clap::CommandFactory;
    use duet_actions::ACTIONS;

    use crate::Cli;

    /// The subcommands that are no action: they read the configuration or the journal, and
    /// write nothing.
    const READERS: &[&[&str]] = &[
        &["config", "path"],
        &["config", "show"],
        &["journal", "show"],
        &["journal", "check"],
    ];

    /// Every leaf of the CLI's tree, as its path.
    fn leaves(command: &clap::Command, path: &[String], out: &mut Vec<Vec<String>>) {
        let subs: Vec<&clap::Command> = command
            .get_subcommands()
            .filter(|sub| sub.get_name() != "help")
            .collect();
        if subs.is_empty() {
            out.push(path.to_vec());
        }
        for sub in subs {
            let mut deeper = path.to_vec();
            deeper.push(sub.get_name().to_owned());
            leaves(sub, &deeper, out);
        }
    }

    #[test]
    fn every_action_has_a_cli_path_and_every_subcommand_is_an_action_or_a_reader() {
        let mut paths = Vec::new();
        leaves(&Cli::command(), &[], &mut paths);
        for action in ACTIONS {
            assert!(
                paths.iter().any(|p| p == action.cli),
                "the action {} has no CLI path {:?}",
                action.name,
                action.cli
            );
        }
        for path in &paths {
            let is_action = ACTIONS.iter().any(|a| a.cli == path.as_slice());
            let is_reader = READERS.iter().any(|r| *r == path.as_slice());
            assert!(
                is_action != is_reader,
                "duet {} is {}",
                path.join(" "),
                if is_action {
                    "both an action and a reader"
                } else {
                    "neither an action nor a reader: a command that writes a file is an action"
                }
            );
        }
        assert_eq!(paths.len(), ACTIONS.len() + READERS.len());
    }
}
