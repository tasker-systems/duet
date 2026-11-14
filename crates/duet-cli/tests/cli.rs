//! The `duet` binary end to end: with no end named anywhere, every conversion writes
//! exactly the bytes it wrote before songs could carry an end, and a drum grid that ends
//! in empty bars comes back whole through MIDI with no bar count.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duet-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn duet(args: &[&Path]) {
    let out = Command::new(env!("CARGO_BIN_EXE_duet"))
        .args(args)
        .output()
        .expect("run duet");
    assert!(
        out.status.success(),
        "duet {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The golden files in `tests/golden` were written by duet at 145077f, before a song
/// could name an end, from `samples/sketch.txt` and `samples/groove.drums`. Neither
/// sample names an end, so every byte must be the same now.
#[test]
fn with_no_end_named_every_output_is_unchanged() {
    let dir = scratch("golden");
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let sketch = root().join("samples/sketch.txt");
    let groove = root().join("samples/groove.drums");
    let out = |name: &str| dir.join(name);
    let p = |s: &'static str| Path::new(s);

    for name in ["sketch.mid", "sketch.txt", "sketch.json", "sketch.toml"] {
        duet(&[p("convert"), &sketch, &out(name)]);
    }
    duet(&[
        p("convert"),
        &out("sketch.mid"),
        &out("sketch-from-midi.txt"),
    ]);
    for name in ["groove.mid", "groove.txt", "groove.json"] {
        duet(&[p("drums"), p("render"), &groove, &out(name)]);
    }
    for name in ["groove.drums", "groove.drums.json"] {
        duet(&[
            p("drums"),
            p("read"),
            &out("groove.mid"),
            &out(name),
            p("--grid"),
            p("1/16"),
        ]);
    }

    let mut compared = 0;
    for entry in std::fs::read_dir(&golden).expect("golden files") {
        let path = entry.expect("entry").path();
        let name = path.file_name().expect("a name").to_owned();
        let want = std::fs::read(&path).expect("golden");
        let got = std::fs::read(dir.join(&name)).expect("written");
        assert!(got == want, "{} differs from before", path.display());
        compared += 1;
    }
    assert_eq!(compared, 10);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Witness that the drum grid's trailing-bars caveat is gone: a grid ending in an empty
/// bar, rendered to MIDI and read back with no `--bars`, is the same text.
#[test]
fn a_grid_ending_in_empty_bars_survives_midi_with_no_bar_count() {
    let dir = scratch("rests");
    let text = "# notefmt drums v1: one row per lane, one column per step, | at bar lines\n\
        tpq 480\ntempo 120\ntime 4/4\ngrid 1/8\nmap gm\n\n[bars 1-3]\n\
        #       1 . 2 . 3 . 4 .   1 . 2 . 3 . 4 .   1 . 2 . 3 . 4 .\n\
        snare | . . x . . . x . | . . . . . . . . | . . . . . . . . |\n\
        kick  | x . . . x . . . | . . . . . . . . | . . . . . . . . |\n";
    let grid = dir.join("rests.drums");
    std::fs::write(&grid, text).expect("write");
    let (mid, back) = (dir.join("rests.mid"), dir.join("back.drums"));
    let p = |s: &'static str| Path::new(s);
    duet(&[p("drums"), p("render"), &grid, &mid]);
    duet(&[p("drums"), p("read"), &mid, &back, p("--grid"), p("1/8")]);
    let read = std::fs::read_to_string(&back).expect("read back");
    assert_eq!(read, text);
    std::fs::remove_dir_all(&dir).expect("clean up");
}
