//! `duet view` and `duet apply`: a document's text views rendered, edited and read back.
//!
//! - Lossless views are the identity: document → view → document, and view → document → view.
//! - A partial grid view of a song, applied back unedited, leaves the song equal, and each
//!   one-cell edit changes exactly one note.
//! - What a view cannot place is refused by name, and the document is left as it was.
//! - Today's `.notes` and `.drums` files read through `apply` by their `# notefmt` lines.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, duet_with, golden, notes_doc, ok, root, s, scratch};
use serde_json::Value;

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

fn text(path: &Path) -> String {
    String::from_utf8(read(path)).expect("utf-8")
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&read(path)).expect("a document")
}

/// Runs duet, checks it refuses saying each of `want`, and returns nothing.
fn refused(dir: &Path, args: &[&str], want: &[&str]) {
    let o = duet(dir, args);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "{args:?} should be refused");
    for want in want {
        assert!(err.contains(want), "{args:?}: {err}\nshould say: {want}");
    }
}

/// Everything after a view's header line.
fn under_header(view: &str) -> &str {
    view.split_once('\n').map_or("", |(_, rest)| rest)
}

#[test]
fn an_old_notes_file_applies_and_views_back_identical_under_a_new_header() {
    let dir = scratch("views-old-notes");
    for (old, kind) in [
        (root().join("samples/sketch.notes"), "notes v1 of song 1.0"),
        (golden("sketch-from-midi.notes"), "notes v1 of song 1.0"),
        (
            root().join("samples/groove.drums"),
            "grid v1 of drum-grid 1.0",
        ),
    ] {
        let doc = dir.join("x.duet");
        ok(&duet(&dir, &["apply", s(&old), "-o", s(&doc)]), "apply -o");
        let view = duet(&dir, &["view", s(&doc)]);
        ok(&view, "view");
        let view = String::from_utf8(view.stdout).expect("utf-8");
        let header = view.lines().next().expect("a header");
        assert!(
            header.starts_with(&format!("# duet {kind} from ")),
            "{header}"
        );
        let old_text = text(&old);
        // The golden from MIDI predates `end`: the view states the default, as every writer does.
        let old_text = if old_text.contains("\nend ") {
            old_text
        } else {
            old_text.replacen("key D minor\n", "key D minor\nend 5.1.0\n", 1)
        };
        assert_eq!(under_header(&view), old_text, "{}", old.display());
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_lossless_view_is_the_identity_both_ways() {
    let dir = scratch("views-identity");
    let p = |n: &str| dir.join(n);
    let sources: Vec<PathBuf> = vec![
        golden("sketch.mid"),
        golden("groove.mid"),
        root().join("crates/duet-schemas/schemas/song/examples/1.0.duet"),
        root().join("crates/duet-schemas/schemas/drum-grid/examples/1.0.duet"),
    ];
    for (i, source) in sources.iter().enumerate() {
        // A document of each: imported, or the committed example.
        let doc = p(&format!("{i}.duet"));
        ok(&duet(&dir, &["convert", s(source), s(&doc)]), "a document");
        let snap = source.to_string_lossy().contains("drum-grid");
        let mut view_args = vec!["view", s(&doc), "-o"];
        let view = p(&format!("{i}.view"));
        view_args.push(s(&view));
        if snap {
            // The example grid holds velocities off the levels: the view shows them snapped.
            view_args.push("--snap-levels");
        }
        ok(&duet(&dir, &view_args), "view");
        // document → view → document, onto its source: equal.
        let before = read(&doc);
        ok(
            &duet(&dir, &["apply", s(&view), "--onto", s(&doc)]),
            "apply --onto",
        );
        assert!(
            read(&doc) == before,
            "{}: view → apply changed the document",
            source.display()
        );
        // view → document → view: byte-identical.
        let again = p(&format!("{i}-again.view"));
        let mut args = vec!["view", s(&doc), "-o", s(&again)];
        if snap {
            args.push("--snap-levels");
        }
        ok(&duet(&dir, &args), "view again");
        assert!(
            read(&again) == read(&view),
            "{}: the view changed",
            source.display()
        );
        if !snap {
            // A lossless view also makes a new document equal to its source's body.
            let fresh = p(&format!("{i}-fresh.duet"));
            ok(
                &duet(&dir, &["apply", s(&view), "-o", s(&fresh)]),
                "apply -o",
            );
            assert_eq!(
                json(&fresh)["body"],
                json(&doc)["body"],
                "{}",
                source.display()
            );
        }
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A song with three tracks, controls on its drum track, note lengths other than one step and
/// a velocity off the levels.
const BAND: &str = "tpq 480\ntempo 100\ntime 4/4\nend 3.1.0\n\n\
    [track \"Keys\" ch1 prog4]\n1.1.0 C4 2 80\n2.1.0 E4 1.5 70\n\n\
    [track \"Bass\" ch2]\n1.1.0 C2 1 90\n1.3.0 G2 0.5 85\n\n\
    [track \"Kit\" ch10 prog0]\n1.1.0 cc7 100\n1.1.0 C2 0.3 120\n1.1.0 F#2 0.1 90\n\
    1.2.0 F#2 0.1 90\n1.2.0 D2 0.5 101\n1.3.0 F#2 0.1 90\n1.4.0 F#2 0.1 40\n\
    2.1.0 C2 0.3 120\n2.3.0 D2 0.5 120\n";

fn band(dir: &Path) -> PathBuf {
    let notes = dir.join("band.notes");
    std::fs::write(&notes, BAND).expect("write");
    let doc = dir.join("band.duet");
    ok(
        &duet(dir, &["apply", s(&notes), "-o", s(&doc), "--as", "notes"]),
        "the band",
    );
    doc
}

fn render_kit(dir: &Path, doc: &Path) -> String {
    let o = duet(
        dir,
        &[
            "view",
            s(doc),
            "--as",
            "grid",
            "--grid",
            "1/16",
            "--track",
            "Kit",
            "--snap-levels",
        ],
    );
    ok(&o, "the grid view of the Kit");
    String::from_utf8(o.stdout).expect("utf-8")
}

/// The notes of the Kit track, as (at, pitch, beats, vel).
fn kit(doc: &Path) -> Vec<Value> {
    let body = &json(doc)["body"];
    let tracks = body["tracks"].as_array().expect("tracks");
    let kit = tracks
        .iter()
        .find(|t| t["name"] == "Kit")
        .expect("a Kit track");
    kit["notes"].as_array().expect("notes").clone()
}

#[test]
fn a_partial_grid_view_applied_unedited_leaves_the_song_equal() {
    let dir = scratch("views-silent");
    let doc = band(&dir);
    let view = render_kit(&dir, &doc);
    assert!(
        view.starts_with("# duet grid v1 of song 1.0 track \"Kit\" step 1/16 from "),
        "{view}"
    );
    assert!(
        view.contains("# this view shows 1 velocities snapped"),
        "{view}"
    );
    let path = dir.join("kit.view");
    std::fs::write(&path, &view).expect("write");
    let before = read(&doc);
    ok(
        &duet(&dir, &["apply", s(&path), "--onto", s(&doc)]),
        "apply unedited",
    );
    assert!(read(&doc) == before, "silent means unchanged");
    let held = text(&doc);
    assert!(
        held.contains("\"vel\": 101") && held.contains("\"beats\": 0.3"),
        "{held}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn each_one_cell_edit_changes_exactly_one_note() {
    let dir = scratch("views-edits");
    // (the row's first cells before, after, what changes: (at, key, vel before, vel after))
    let cases = [
        // a . to an x: a new note one step long at the hit level.
        (
            "snare | . . . . x",
            "snare | x . . . x",
            ("1.1.0", "D2", None, Some(90)),
        ),
        // an x to a .: the note is removed.
        (
            "hh    | x . . . x",
            "hh    | . . . . x",
            ("1.1.0", "F#2", Some(90), None),
        ),
        // an x to an X: the velocity is the accent level, and the length is kept.
        (
            "hh    | x . . . x . . . x . . . g",
            "hh    | x . . . x . . . X . . . g",
            ("1.3.0", "F#2", Some(90), Some(120)),
        ),
    ];
    for (i, (from, to, (at, pitch, was, now))) in cases.into_iter().enumerate() {
        let doc = band(&dir);
        let view = render_kit(&dir, &doc);
        assert!(
            view.contains(from),
            "case {i}: the view should hold '{from}':\n{view}"
        );
        let path = dir.join(format!("edit-{i}.view"));
        std::fs::write(&path, view.replacen(from, to, 1)).expect("write");
        let before = kit(&doc);
        let others = |d: &Path| {
            let body = json(d)["body"].clone();
            let mut tracks = body["tracks"].as_array().unwrap().clone();
            tracks.retain(|t| t["name"] != "Kit");
            (tracks, body["tempo_bpm"].clone(), body["end"].clone())
        };
        let before_others = others(&doc);
        ok(
            &duet(&dir, &["apply", s(&path), "--onto", s(&doc)]),
            "apply one edit",
        );
        let after = kit(&doc);
        let only_before: Vec<&Value> = before.iter().filter(|n| !after.contains(n)).collect();
        let only_after: Vec<&Value> = after.iter().filter(|n| !before.contains(n)).collect();
        let found = |notes: &[&Value]| -> Option<i64> {
            match notes {
                [] => None,
                [n] => {
                    assert_eq!(
                        (n["at"].as_str(), n["pitch"].as_str()),
                        (Some(at), Some(pitch)),
                        "case {i}"
                    );
                    n["vel"].as_i64()
                }
                more => panic!("case {i}: more than one note changed: {more:?}"),
            }
        };
        assert_eq!(found(&only_before), was, "case {i}: the note before");
        assert_eq!(found(&only_after), now, "case {i}: the note after");
        if let (Some(_), Some(_)) = (was, now) {
            assert_eq!(
                only_before[0]["beats"], only_after[0]["beats"],
                "case {i}: the length is kept"
            );
        }
        if was.is_none() {
            assert_eq!(
                only_after[0]["beats"], 0.25,
                "case {i}: a new note is one step long"
            );
        }
        assert_eq!(
            others(&doc),
            before_others,
            "case {i}: the other tracks are untouched"
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn an_off_level_velocity_survives_view_and_apply_unedited() {
    let dir = scratch("views-off-level");
    let take = notes_doc(
        &dir,
        "take.notes",
        "[track \"Kit\" ch10]\n1.1.0 C2 0.25 101\n1.2.0 D2 0.25 7\n",
    );
    let doc = dir.join("grid.duet");
    ok(
        &duet(
            &dir,
            &["drums", "read", s(&take), s(&doc), "--grid", "1/16"],
        ),
        "a drum-grid document",
    );
    refused(
        &dir,
        &["view", s(&doc)],
        &[
            "velocity 101 is not one of the grid view's levels",
            "--snap-levels",
        ],
    );
    let view = dir.join("take.view");
    ok(
        &duet(&dir, &["view", s(&doc), "--snap-levels", "-o", s(&view)]),
        "view snapped",
    );
    assert!(
        text(&view).contains("# this view shows 2 velocities snapped"),
        "{}",
        text(&view)
    );
    let before = read(&doc);
    ok(
        &duet(&dir, &["apply", s(&view), "--onto", s(&doc)]),
        "apply unedited",
    );
    assert!(read(&doc) == before, "101 and 7 are kept exactly");
    // A cell whose level is changed takes the new level; the other keeps its exact velocity.
    let edited = text(&view).replacen("kick  | x", "kick  | X", 1);
    std::fs::write(&view, edited).expect("write");
    ok(
        &duet(&dir, &["apply", s(&view), "--onto", s(&doc)]),
        "apply one edit",
    );
    let data = json(&doc)["body"]["vel"]["data"].clone();
    let data: Vec<i64> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert!(
        data.contains(&120) && data.contains(&7) && !data.contains(&101),
        "{data:?}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn what_a_view_cannot_place_is_refused_by_name_and_nothing_changes() {
    let dir = scratch("views-refused");
    let doc = band(&dir);
    let view = render_kit(&dir, &doc);
    let before = read(&doc);
    let try_view = |name: &str, view: &str, want: &[&str]| {
        let path = dir.join(name);
        std::fs::write(&path, view).expect("write");
        refused(&dir, &["apply", s(&path), "--onto", s(&doc)], want);
        assert!(read(&doc) == before, "{name}: the document is unchanged");
    };
    try_view(
        "tempo.view",
        &view.replacen("\ntempo 100\n", "\ntempo 140\n", 1),
        &[
            "the view changes the song's tempo",
            "change it in the notes view",
        ],
    );
    let lanes_line = view
        .lines()
        .find(|l| l.starts_with("snare"))
        .expect("a snare row");
    let added = lanes_line.replacen("snare", "crash", 1);
    try_view(
        "lane.view",
        &view.replacen(
            &format!("{lanes_line}\n"),
            &format!("{added}\n{lanes_line}\n"),
            1,
        ),
        &["cannot add or remove a lane"],
    );
    try_view(
        "header.view",
        &view.replacen("# duet grid v1", "# duet grid v2", 1),
        &["grid v2 view; this duet reads grid v1"],
    );
    try_view(
        "schema.view",
        &view.replacen("of song 1.0", "of song 1.3", 1),
        &["view of song 1.3; this duet reads song up to 1.0"],
    );

    // A document changed since the view was rendered is refused, naming both digests, unless
    // --force; then the edit lands, and duet says it applied anyway.
    let path = dir.join("kit.view");
    std::fs::write(&path, &view).expect("write");
    let notes = dir.join("band.notes");
    std::fs::write(&notes, BAND.replacen("1.1.0 C4 2 80", "1.1.0 C4 2 81", 1)).expect("write");
    ok(
        &duet(
            &dir,
            &["apply", s(&notes), "--onto", s(&doc), "--as", "notes"],
        ),
        "change the song",
    );
    let changed = read(&doc);
    refused(
        &dir,
        &["apply", s(&path), "--onto", s(&doc)],
        &["has changed since the view was rendered", "--force"],
    );
    assert!(read(&doc) == changed);
    let o = duet(&dir, &["apply", s(&path), "--onto", s(&doc), "--force"]);
    ok(&o, "--force");
    assert!(String::from_utf8_lossy(&o.stderr).contains("as --force asks"));

    // A lossy view makes no document of its own; a view with no header asks what it is.
    refused(
        &dir,
        &["apply", s(&path), "-o", s(&dir.join("new.duet"))],
        &["lossy", "--onto"],
    );
    let bare = dir.join("bare.view");
    std::fs::write(&bare, "[track \"P\" ch1]\n1.1.0 C4 1 80\n").expect("write");
    refused(
        &dir,
        &["apply", s(&bare), "-o", s(&dir.join("new.duet"))],
        &["no header line", "--as notes"],
    );
    ok(
        &duet(
            &dir,
            &[
                "apply",
                s(&bare),
                "-o",
                s(&dir.join("new.duet")),
                "--as",
                "notes",
            ],
        ),
        "--as notes",
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn apply_onto_keeps_the_envelope_and_views_drop_from() {
    let dir = scratch("views-envelope");
    let abc = dir.join("tune.abc");
    std::fs::write(
        &abc,
        "X:1\nT:Slurred\nM:4/4\nL:1/8\nK:G\nG2 (AB) c2 d2 | e8 |]\n",
    )
    .expect("write");
    let doc = dir.join("tune.duet");
    ok(&duet(&dir, &["convert", s(&abc), s(&doc)]), "an import");
    let from = json(&doc)["duet"]["from"].clone();
    assert!(from.is_object());
    let view = duet(&dir, &["view", s(&doc)]);
    ok(&view, "view");
    let shown = String::from_utf8(view.stdout).expect("utf-8");
    assert!(
        !shown.contains("slur") && !shown.contains("abc"),
        "a view drops from:\n{shown}"
    );
    let path = dir.join("tune.view");
    std::fs::write(&path, shown.replacen(" 80\n", " 81\n", 1)).expect("write");
    ok(
        &duet(&dir, &["apply", s(&path), "--onto", s(&doc)]),
        "apply an edit",
    );
    assert_eq!(
        json(&doc)["duet"]["from"],
        from,
        "apply --onto keeps the envelope's from"
    );

    // Standard input carries a view too.
    let none = dir.join("no-config");
    let view = std::fs::read(&path).expect("view");
    let o = duet_with(
        &dir,
        &none,
        &["apply", "-", "-o", s(&dir.join("stdin.duet"))],
        &view,
    );
    ok(&o, "apply - -o");
    assert_eq!(json(&dir.join("stdin.duet"))["body"], json(&doc)["body"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}
