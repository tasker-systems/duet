//! `duet convert` from a drum-grid document to a drum-grid document with no `--grid` is a
//! straight copy, as `duet drums snap` writes one without snapping, so what a part cannot hold
//! (a lane with no hits, the grid's own velocity levels, any velocity at all) survives. With
//! `--grid`, the grid goes through a part at that step, as `drums render` then `drums read`
//! would take it. A grid's text is a view: `duet apply` reads it and `duet view` shows it.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, golden, grid_doc, notes_doc, ok, root, s, scratch, view_text};

/// A grid a part cannot carry whole: a lane with no hits, and its own velocity levels.
const KEEPS: &str = "grid 1/8\nvel g 30 x 80 X 110\n\n[bars 1-2]\n\
    ride  | . . . . . . . . | . . . . . . . . |\n\
    snare | . . x . . . x g | . . x . . . x . |\n\
    kick  | X . . . X . . . | X . . . X X . . |\n";

/// The kit take with velocities off the levels.
const TAKE: &str = "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n1.2.0 D2 0.25 64\n";

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

/// A grid's text read into a drum-grid document `dir/<name>.duet` with `duet apply`.
fn applied(dir: &Path, name: &str, text: &Path) -> PathBuf {
    let doc = dir.join(format!("{name}.duet"));
    ok(
        &duet(dir, &["apply", s(text), "--as", "grid", "-o", s(&doc)]),
        &format!("apply {}", text.display()),
    );
    doc
}

/// A document's grid view, each line after a newline, so a line can be found whole.
fn shown(dir: &Path, doc: &Path) -> String {
    format!("\n{}", view_text(dir, doc, "grid", &[]))
}

/// A drum-grid document copied is what `drums snap` writes, the empty lane and the levels kept,
/// and it says nothing.
#[test]
fn a_grid_copied_loses_nothing() {
    let dir = scratch("copy");
    let sources = [
        grid_doc(&dir, "keeps.drums", KEEPS),
        applied(&dir, "sample", &root().join("samples/groove.drums")),
        applied(&dir, "golden", &golden("groove.drums")),
    ];
    for (i, source) in sources.iter().enumerate() {
        let a = dir.join(format!("{i}-convert.duet"));
        let b = dir.join(format!("{i}-snap.duet"));
        let conv = duet(&dir, &["convert", s(source), s(&a)]);
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
    let text = shown(&dir, &dir.join("0-convert.duet"));
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

/// A velocity off the view's levels is kept by `convert --grid` in a drum-grid document, as
/// `drums read` keeps it, and refused by the grid's view; `--snap-levels` snaps it as
/// `drums read --snap-levels` does. Nothing snaps unasked.
#[test]
fn off_level_velocities_are_refused_and_snap_only_when_asked() {
    let dir = scratch("copy-levels");
    let take = notes_doc(&dir, "take.notes", TAKE);
    let args = |out: &Path| {
        vec![
            "convert".to_owned(),
            s(&take).to_owned(),
            s(out).to_owned(),
            "--grid".to_owned(),
            "1/16".to_owned(),
        ]
    };
    let exact = dir.join("exact.duet");
    ok(&duet(&dir, &args(&exact)), "kept exactly");
    let held = String::from_utf8(read(&exact)).expect("utf-8");
    assert!(
        held.contains("100") && held.contains("64"),
        "nothing snaps unasked: {held}"
    );
    let text = dir.join("take.drums");
    let o = duet(&dir, &["view", s(&exact), "--as", "grid", "-o", s(&text)]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("is not one of the grid view's levels"),
        "{err}"
    );
    assert!(err.contains("--snap-levels"), "{err}");
    assert!(!text.exists(), "nothing written");

    let snapped = dir.join("snapped.duet");
    let mut snap = args(&snapped);
    snap.push("--snap-levels".to_owned());
    ok(&duet(&dir, &snap), "snapped");
    let read_snapped = dir.join("read.duet");
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
                "--snap-levels",
            ],
        ),
        "drums read --snap-levels",
    );
    assert!(
        read(&snapped) == read(&read_snapped),
        "convert snaps as drums read does"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// With `--grid`, a grid goes through a part at that step: the same bytes as `drums render`
/// then `drums read` at that step, so a 1/8 grid becomes a 1/16 one. `--track` and `--bars`
/// read a part's drum track, so without `--grid` they are refused by name.
#[test]
fn a_named_step_re_steps_a_grid_and_track_or_bars_need_one() {
    let dir = scratch("copy-restep");
    let keeps = grid_doc(
        &dir,
        "keeps.drums",
        &KEEPS.replace("vel g 30 x 80 X 110\n", ""),
    );

    let a = dir.join("a.duet");
    ok(
        &duet(&dir, &["convert", s(&keeps), s(&a), "--grid", "1/16"]),
        "re-step",
    );
    let (mid, b) = (dir.join("k.mid"), dir.join("b.duet"));
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
    let text = shown(&dir, &a);
    assert!(text.contains("\ngrid 1/16\n"), "{text}");
    assert!(
        !text.contains("ride"),
        "a part carries no unplayed lane: {text}"
    );

    for (flags, want) in [
        (
            &["--track", "Drums"][..],
            "--track is for reading a part's drum track",
        ),
        (
            &["--bars", "2"][..],
            "--bars is for reading a part's drum track",
        ),
        (
            &["--track", "Drums", "--bars", "2"][..],
            "--track, --bars are for reading",
        ),
    ] {
        let out = dir.join("never.duet");
        let mut args = vec!["convert", s(&keeps), s(&out)];
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

/// A drum-grid document is the exact form: copied to a document it is unchanged, whatever its
/// velocities; its view is the text it was read from when its velocities are levels, and
/// refused when they are not, until `--snap-levels` snaps them as `drums snap` does.
#[test]
fn a_grid_document_copies_exactly_and_snaps_only_when_asked() {
    let dir = scratch("copy-documents");
    let copy = |from: &Path, to: &Path, extra: &[&str]| {
        let mut args = vec!["convert", s(from), s(to)];
        args.extend(extra);
        duet(&dir, &args)
    };

    // Text to a document and back: what `drums snap` writes, the empty lane and levels kept.
    let doc = grid_doc(&dir, "keeps.drums", KEEPS);
    let snapped = dir.join("snapped.duet");
    ok(
        &duet(&dir, &["drums", "snap", s(&doc), s(&snapped)]),
        "drums snap",
    );
    assert!(read(&doc) == read(&snapped), "apply and drums snap agree");
    let again = dir.join("keeps-again.duet");
    ok(&copy(&doc, &again, &[]), "a document to a document");
    assert_eq!(
        shown(&dir, &again),
        shown(&dir, &snapped),
        "through a document"
    );
    assert!(shown(&dir, &again).contains("\nride  | . . . . . . . . |"));

    // Velocities off the levels: drums read keeps them in a document, which copies unchanged.
    let take = notes_doc(&dir, "take.notes", TAKE);
    let exact = dir.join("exact.duet");
    ok(
        &duet(
            &dir,
            &["drums", "read", s(&take), s(&exact), "--grid", "1/16"],
        ),
        "drums read to a document",
    );
    let held = String::from_utf8(read(&exact)).expect("utf-8");
    assert!(
        held.contains("100") && held.contains("64"),
        "kept exactly: {held}"
    );
    let again = dir.join("again.duet");
    let o = copy(&exact, &again, &[]);
    ok(&o, "a document to a document");
    assert!(o.stderr.is_empty(), "a copy says nothing");
    assert!(read(&again) == read(&exact), "a document copies unchanged");

    let text = dir.join("exact.drums");
    let view = |extra: &[&str]| {
        let mut args = vec!["view", s(&exact), "--as", "grid", "-o", s(&text)];
        args.extend(extra);
        duet(&dir, &args)
    };
    let o = view(&[]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("is not one of the grid view's levels"),
        "{err}"
    );
    assert!(
        err.contains("--snap-levels") && err.contains("apply --onto"),
        "{err}"
    );
    assert!(!text.exists(), "nothing written");
    ok(&view(&["--snap-levels"]), "snapped");
    let by_view = dir.join("by-view.duet");
    ok(
        &duet(&dir, &["apply", s(&text), "-o", s(&by_view)]),
        "the snapped view applied",
    );
    let by_snap = dir.join("by-snap.duet");
    ok(
        &duet(&dir, &["drums", "snap", s(&exact), s(&by_snap)]),
        "drums snap",
    );
    let body = |path: &Path| {
        serde_json::from_slice::<serde_json::Value>(&read(path)).expect("json")["body"].clone()
    };
    assert_eq!(
        body(&by_view),
        body(&by_snap),
        "--snap-levels snaps as drums snap does"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// An off-level velocity survives grid → .duet → MIDI → grid exactly, by the drum commands.
#[test]
fn an_off_level_velocity_survives_a_document_and_midi_exactly() {
    let dir = scratch("copy-midi");
    let take = notes_doc(
        &dir,
        "part.notes",
        "[track \"Kit\" ch10]\n1.1.0 C2 0.25 101\n1.2.0 D2 0.25 7\n1.3.0 F#2 0.25 127\n",
    );
    let doc = dir.join("take.duet");
    ok(
        &duet(
            &dir,
            &["drums", "read", s(&take), s(&doc), "--grid", "1/16"],
        ),
        "a document",
    );
    let mid = dir.join("take.mid");
    ok(
        &duet(&dir, &["drums", "render", s(&doc), s(&mid)]),
        "to MIDI",
    );
    let back = dir.join("back.duet");
    ok(
        &duet(
            &dir,
            &["drums", "read", s(&mid), s(&back), "--grid", "1/16"],
        ),
        "back",
    );
    assert!(
        read(&back) == read(&doc),
        "the same document, every velocity exact"
    );
    let body: serde_json::Value = serde_json::from_slice(&read(&back)).expect("json");
    let data = body["body"]["vel"]["data"].as_array().expect("data");
    for v in [101, 7, 127] {
        assert!(
            data.contains(&serde_json::Value::from(v)),
            "{v} kept: {body}"
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}
