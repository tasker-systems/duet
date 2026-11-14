//! A file name means the same thing to every command: `.drums`, `.drums.json` and
//! `.drums.toml` are drum grids, and every other name is a song's. A command that reads or
//! writes a song refuses a grid's name, and one that reads or writes a grid refuses a
//! song's, rather than misreading the file. `--from` and `--to` still say what a name
//! does not, and `-` is each side's default.

use std::path::Path;

use duet_e2e::{duet, duet_with, golden, ok, s, scratch};

/// Runs duet and checks it refuses, saying `want`, and writes nothing to `out`.
fn refused(dir: &Path, args: &[&str], out: &Path, want: &str) {
    let o = duet(dir, args);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "{args:?} should be refused");
    assert!(err.contains(want), "{args:?}: {err}\nshould say: {want}");
    assert!(!out.exists(), "{args:?} wrote {}", out.display());
}

#[test]
fn the_drums_and_song_commands_refuse_a_name_of_the_other_kind() {
    let dir = scratch("names");
    let beat = golden("groove.drums");
    let exact = golden("groove.drums.json");
    let take = golden("groove.mid");
    let song_json = golden("groove.json");
    let p = |name: &str| dir.join(name);
    let grid_name = "is a drum grid's name (.drums, .drums.json or .drums.toml)";
    let not_grid = "is not a drum grid's name";

    // drums render writes a song: a grid's name as its output is refused.
    for out in ["out.drums", "out.drums.json", "out.drums.toml"] {
        let want = format!("{grid_name}, and render writes a song");
        refused(
            &dir,
            &["drums", "render", s(&beat), s(&p(out))],
            &p(out),
            &want,
        );
    }
    // ...and it reads a grid: a song's name as its input is refused.
    let want = format!("{not_grid}, and render reads a grid");
    refused(
        &dir,
        &["drums", "render", s(&song_json), s(&p("r.mid"))],
        &p("r.mid"),
        &want,
    );

    // drums read writes a grid: a song's name as its output is refused.
    for out in ["out.json", "out.toml", "out.notes", "out.mid"] {
        let want = format!("{not_grid}, and read writes a grid");
        refused(
            &dir,
            &["drums", "read", s(&take), s(&p(out)), "--grid", "1/16"],
            &p(out),
            &want,
        );
    }
    // ...and it reads a song: a grid's name as its input is refused.
    let want = format!("{grid_name}, and read reads a song");
    refused(
        &dir,
        &[
            "drums",
            "read",
            s(&exact),
            s(&p("x.drums")),
            "--grid",
            "1/16",
        ],
        &p("x.drums"),
        &want,
    );

    // drums snap reads and writes grids.
    let want = format!("{not_grid}, and snap reads a grid");
    refused(
        &dir,
        &["drums", "snap", s(&song_json), s(&p("s.drums"))],
        &p("s.drums"),
        &want,
    );
    let want = format!("{not_grid}, and snap writes a grid");
    refused(
        &dir,
        &["drums", "snap", s(&beat), s(&p("s.json"))],
        &p("s.json"),
        &want,
    );

    // song show and song quantize read and write songs.
    let want = format!("{grid_name}, and song show reads a song");
    refused(&dir, &["song", "show", s(&beat)], &p("never"), &want);
    let want = format!("{grid_name}, and quantize reads a song");
    refused(
        &dir,
        &[
            "song",
            "quantize",
            s(&exact),
            s(&p("q.notes")),
            "--grid",
            "1/16",
        ],
        &p("q.notes"),
        &want,
    );
    let want = format!("{grid_name}, and quantize writes a song");
    refused(
        &dir,
        &[
            "song",
            "quantize",
            s(&take),
            s(&p("q.drums.json")),
            "--grid",
            "1/16",
        ],
        &p("q.drums.json"),
        &want,
    );

    // The right names still work, in every grid form.
    for out in ["ok.drums", "ok.drums.json", "ok.drums.toml"] {
        ok(
            &duet(
                &dir,
                &["drums", "read", s(&take), s(&p(out)), "--grid", "1/16"],
            ),
            out,
        );
        ok(
            &duet(&dir, &["drums", "render", s(&p(out)), s(&p("ok.mid"))]),
            out,
        );
        ok(
            &duet(&dir, &["drums", "snap", s(&p(out)), s(&p("snapped.drums"))]),
            out,
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `--to` and `--from` say what a name does not, so a song can be written to any name on
/// purpose; and `-` is .drums text on a grid's side and the default format on a song's.
#[test]
fn a_named_format_and_standard_streams_still_work() {
    let dir = scratch("names-explicit");
    let beat = golden("groove.drums");
    let odd = dir.join("song.drums.json");
    let o = duet(
        &dir,
        &["drums", "render", s(&beat), s(&odd), "--to", "json"],
    );
    ok(&o, "--to json");
    let text = std::fs::read_to_string(&odd).expect("written");
    assert!(text.contains("\"tracks\""), "a song, as asked: {text}");
    let o = duet(&dir, &["song", "show", s(&odd), "--from", "json"]);
    ok(&o, "--from json");

    let grid = std::fs::read(&beat).expect("golden");
    let none = dir.join("no-config");
    let o = duet_with(&dir, &none, &["drums", "render", "-", "-"], &grid);
    ok(&o, "drums render - -");
    assert!(String::from_utf8_lossy(&o.stdout).contains("[track \"Drums\" ch10]"));
    std::fs::remove_dir_all(&dir).expect("clean up");
}
