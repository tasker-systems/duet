//! `duet convert --from drums --to drums` with no `--grid` is a straight copy: it reads and
//! writes the grid's text as `duet drums snap` does, without snapping, so what a song cannot
//! hold (a lane with no hits, the grid's own velocity levels) survives. With `--grid`, the
//! grid goes through a song at that step, as `drums render` then `drums read` would take it.

use std::path::Path;

use duet_e2e::{duet, golden, ok, root, s, scratch};

/// A grid a song cannot carry whole: a lane with no hits, and its own velocity levels.
const KEEPS: &str = "grid 1/8\nvel g 30 x 80 X 110\n\n[bars 1-2]\n\
    ride  | . . . . . . . . | . . . . . . . . |\n\
    snare | . . x . . . x g | . . x . . . x . |\n\
    kick  | X . . . X . . . | X . . . X X . . |\n";

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

/// A grid's text copied is what `drums snap` writes, the empty lane and the levels kept, and
/// it says nothing.
#[test]
fn a_grid_copied_loses_nothing() {
    let dir = scratch("copy");
    let keeps = dir.join("keeps.drums");
    std::fs::write(&keeps, KEEPS).expect("write");
    let sources = [
        keeps,
        root().join("samples/groove.drums"),
        golden("groove.drums"),
    ];
    for (i, source) in sources.iter().enumerate() {
        let a = dir.join(format!("{i}-convert.drums"));
        let b = dir.join(format!("{i}-snap.drums"));
        let conv = duet(
            &dir,
            &[
                "convert",
                s(source),
                s(&a),
                "--from",
                "drums",
                "--to",
                "drums",
            ],
        );
        ok(&conv, &format!("convert {}", source.display()));
        ok(&duet(&dir, &["drums", "snap", s(source), s(&b)]), "snap");
        assert!(
            read(&a) == read(&b),
            "{}: convert and snap differ",
            source.display()
        );
        assert!(
            conv.stderr.is_empty(),
            "a copy says nothing: {:?}",
            conv.stderr
        );
    }
    let text = String::from_utf8(read(&dir.join("0-convert.drums"))).expect("utf-8");
    assert!(
        text.contains("\nride  | . . . . . . . . |"),
        "the empty lane is kept: {text}"
    );
    assert!(
        text.contains("\nvel g 30 x 80 X 110\n"),
        "the levels are kept: {text}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A velocity off the text's levels is refused as a grid's text, as `drums read` refuses it,
/// until `--snap-levels` snaps it as `drums read --snap-levels` does. Nothing snaps unasked.
#[test]
fn off_level_velocities_are_refused_and_snap_only_when_asked() {
    let dir = scratch("copy-levels");
    let take = dir.join("take.notes");
    std::fs::write(
        &take,
        "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n1.2.0 D2 0.25 64\n",
    )
    .expect("write");
    let args = |out: &Path| {
        vec![
            "convert".to_owned(),
            s(&take).to_owned(),
            s(out).to_owned(),
            "--from".to_owned(),
            "text".to_owned(),
            "--to".to_owned(),
            "drums".to_owned(),
            "--grid".to_owned(),
            "1/16".to_owned(),
        ]
    };
    let text = dir.join("take.drums");
    let o = duet(&dir, &args(&text));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("is not one of the drum grid text's levels"),
        "{err}"
    );
    assert!(err.contains("--snap-levels"), "{err}");
    assert!(!text.exists(), "nothing written");

    let mut snap = args(&text);
    snap.push("--snap-levels".to_owned());
    ok(&duet(&dir, &snap), "snapped");
    let read_snapped = dir.join("read.drums");
    ok(
        &duet(
            &dir,
            &[
                "drums",
                "read",
                s(&take),
                s(&read_snapped),
                "--grid",
                "1/16",
                "--from",
                "text",
                "--snap-levels",
            ],
        ),
        "drums read --snap-levels",
    );
    assert!(
        read(&text) == read(&read_snapped),
        "convert snaps as drums read does"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// With `--grid`, a grid goes through a song at that step: the same bytes as `drums render`
/// then `drums read` at that step, so a 1/8 grid becomes a 1/16 one. `--track` and `--bars`
/// read a song's drum track, so without `--grid` they are refused by name.
#[test]
fn a_named_step_re_steps_a_grid_and_track_or_bars_need_one() {
    let dir = scratch("copy-restep");
    let keeps = dir.join("keeps.drums");
    std::fs::write(&keeps, KEEPS.replace("vel g 30 x 80 X 110\n", "")).expect("write");

    let a = dir.join("a.drums");
    ok(
        &duet(
            &dir,
            &[
                "convert",
                s(&keeps),
                s(&a),
                "--from",
                "drums",
                "--to",
                "drums",
                "--grid",
                "1/16",
            ],
        ),
        "re-step",
    );
    let (mid, b) = (dir.join("k.mid"), dir.join("b.drums"));
    ok(
        &duet(&dir, &["drums", "render", s(&keeps), s(&mid)]),
        "render",
    );
    ok(
        &duet(&dir, &["drums", "read", s(&mid), s(&b), "--grid", "1/16"]),
        "read",
    );
    assert!(
        read(&a) == read(&b),
        "--grid is render then read at that step"
    );
    let text = String::from_utf8(read(&a)).expect("utf-8");
    assert!(text.contains("\ngrid 1/16\n"), "{text}");
    assert!(
        !text.contains("ride"),
        "a song carries no unplayed lane: {text}"
    );

    for (flags, want) in [
        (
            &["--track", "Drums"][..],
            "--track is for reading a song's drum track",
        ),
        (
            &["--bars", "2"][..],
            "--bars is for reading a song's drum track",
        ),
        (
            &["--track", "Drums", "--bars", "2"][..],
            "--track, --bars are for reading",
        ),
    ] {
        let out = dir.join("never.drums");
        let mut args = vec![
            "convert",
            s(&keeps),
            s(&out),
            "--from",
            "drums",
            "--to",
            "drums",
        ];
        args.extend(flags);
        let o = duet(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            !o.status.success() && err.contains(want),
            "{flags:?}: {err}"
        );
        assert!(err.contains("need --grid"), "{err}");
        assert!(!out.exists(), "nothing written");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}
