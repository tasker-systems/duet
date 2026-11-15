//! A file's envelope means the same thing to every command, and a name means only what it
//! names: `.duet` is a document, read by its envelope, and `.mid` (`.midi`, `.smf`) and `.abc`
//! are the outside formats. Every other name is refused rather than misread: `.json` and
//! `.toml` are no longer read, and a view's text (`.notes`, `.drums`) is read only when
//! `--from text` says so. On a drum grid's side of a command, a `.duet` is a drum-grid document
//! and any other name the grid's text. Standard input is a document when it starts with `{`.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, duet_with, golden, ok, root, s, scratch};

/// Runs duet and checks it refuses, saying each of `want`, and writes nothing to `out`.
fn refused(dir: &Path, args: &[&str], out: &Path, want: &[&str]) {
    let o = duet(dir, args);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "{args:?} should be refused");
    for want in want {
        assert!(err.contains(want), "{args:?}: {err}\nshould say: {want}");
    }
    assert!(!out.exists(), "{args:?} wrote {}", out.display());
}

/// A document of `kind` at `version`, with an empty body.
fn document(dir: &Path, name: &str, kind: &str, version: &str) -> PathBuf {
    let path = dir.join(name);
    let text = format!(
        "{{\"duet\":{{\"kind\":\"{kind}\",\"version\":\"{version}\",\"tool\":{{\"name\":\"duet\",\"version\":\"0\"}}}},\"body\":{{}}}}\n"
    );
    std::fs::write(&path, text).expect("write");
    path
}

/// The sample sketch as a song document in `dir`.
fn sketch_doc(dir: &Path) -> PathBuf {
    let doc = dir.join("sketch.duet");
    let notes = root().join("samples/sketch.notes");
    ok(
        &duet(dir, &["convert", s(&notes), s(&doc), "--from", "text"]),
        "the sample as a .duet",
    );
    doc
}

#[test]
fn a_name_duet_no_longer_reads_is_refused_saying_so() {
    let dir = scratch("names-cut");
    let take = golden("groove.mid");
    let p = |name: &str| dir.join(name);
    for name in ["old.json", "old.toml", "old.drums.json", "old.drums.toml"] {
        std::fs::write(p(name), "{}").expect("write");
        let want = ["is not a duet document", "no longer read"];
        refused(&dir, &["song", "show", s(&p(name))], &p("never"), &want);
        refused(
            &dir,
            &["convert", s(&take), s(&p(&format!("new-{name}")))],
            &p(&format!("new-{name}")),
            &want,
        );
    }
    // A view's old name is refused, and --from text reads it.
    let notes = root().join("samples/sketch.notes");
    let want = ["sketch.notes", "notefmt text is a view", "--from text"];
    refused(
        &dir,
        &["convert", s(&notes), s(&p("x.mid"))],
        &p("x.mid"),
        &want,
    );
    refused(
        &dir,
        &["convert", s(&take), s(&p("x.notes"))],
        &p("x.notes"),
        &["x.notes", "--from text"],
    );
    ok(
        &duet(
            &dir,
            &["convert", s(&notes), s(&p("x.mid")), "--from", "text"],
        ),
        "--from text",
    );
    ok(
        &duet(
            &dir,
            &["convert", s(&take), s(&p("x.notes")), "--to", "text"],
        ),
        "--to text",
    );
    let back = std::fs::read_to_string(p("x.notes")).expect("written");
    assert!(back.starts_with("# notefmt v1"), "{back}");
    // A name with no format at all asks for one.
    refused(
        &dir,
        &["convert", s(&take), s(&p("song"))],
        &p("song"),
        &["name any other with --from or --to"],
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_document_is_read_by_its_envelope_and_another_kind_is_refused_by_name() {
    let dir = scratch("names-kind");
    let p = |name: &str| dir.join(name);
    let song = sketch_doc(&dir);

    // A song document reads for every song command, whatever else it is called.
    let renamed = p("anything.DUET");
    std::fs::copy(&song, &renamed).expect("copy");
    let shown = duet(&dir, &["song", "show", s(&renamed)]);
    ok(&shown, "song show of a .duet");
    let notes = std::fs::read_to_string(root().join("samples/sketch.notes")).expect("sample");
    assert_eq!(
        String::from_utf8_lossy(&shown.stdout),
        notes,
        "the same song"
    );

    // A kind this duet does not know.
    let doc = document(&dir, "lute-tab.duet", "lute-tab", "1.0");
    let want = [s(&doc), "lute-tab", "this duet knows no kind"];
    refused(&dir, &["song", "show", s(&doc)], &p("never"), &want);
    refused(
        &dir,
        &["convert", s(&doc), s(&p("out.mid"))],
        &p("out.mid"),
        &want,
    );
    refused(
        &dir,
        &["drums", "render", s(&doc), s(&p("out.mid"))],
        &p("out.mid"),
        &want,
    );

    // A drum-grid document where a song is read is refused, naming its kind and what makes a
    // song of it.
    let beat = root().join("samples/groove.drums");
    let grid = p("beat.duet");
    ok(
        &duet(&dir, &["drums", "snap", s(&beat), s(&grid)]),
        "a drum-grid document",
    );
    let want = [
        "beat.duet is a drum-grid document",
        "song show reads a song",
        "duet drums render makes a song of one",
    ];
    refused(&dir, &["song", "show", s(&grid)], &p("never"), &want);

    // A song at another major points at duet convert.
    let doc = document(&dir, "future.duet", "song", "2.0");
    let want = ["song 2.0", "this duet reads song 1.x", "duet convert"];
    refused(
        &dir,
        &["convert", s(&doc), s(&p("out.mid"))],
        &p("out.mid"),
        &want,
    );

    // A song document where a grid's text is read is refused, naming its kind.
    let want = ["is a song document", "render reads a drum grid"];
    refused(
        &dir,
        &["drums", "render", s(&song), s(&p("out.mid"))],
        &p("out.mid"),
        &want,
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// The grid side of a drum command is a drum-grid document for a .duet and a grid's text for
/// any other name, and refuses a name that is something else. The song side is a song's.
#[test]
fn the_drum_commands_read_a_grid_text_on_the_grid_side_and_refuse_other_names() {
    let dir = scratch("names-drums");
    let beat = root().join("samples/groove.drums");
    let take = golden("groove.mid");
    let p = |name: &str| dir.join(name);

    for out in ["out.mid", "out.abc", "out.json", "out.duet.jsonl"] {
        refused(
            &dir,
            &["drums", "read", s(&take), s(&p(out)), "--grid", "1/16"],
            &p(out),
            &[out],
        );
    }
    // A .duet on the grid side is a drum-grid document.
    ok(
        &duet(
            &dir,
            &["drums", "read", s(&take), s(&p("g.duet")), "--grid", "1/16"],
        ),
        "g.duet",
    );
    let doc = std::fs::read_to_string(p("g.duet")).expect("written");
    assert!(doc.contains("\"kind\": \"drum-grid\""), "{doc}");
    refused(
        &dir,
        &[
            "drums",
            "render",
            s(&take),
            s(&p("r.notes")),
            "--to",
            "text",
        ],
        &p("r.notes"),
        &["is a song's name (midi)"],
    );
    refused(
        &dir,
        &["drums", "render", s(&beat), s(&p("r.drums"))],
        &p("r.drums"),
        &["--from text"],
    );

    // Any other name is a grid's text, read and written.
    for out in ["ok.drums", "ok.grid", "ok"] {
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
        assert_eq!(
            std::fs::read(p(out)).unwrap(),
            std::fs::read(&beat).unwrap(),
            "{out}"
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Standard input is a document when its first non-blank byte is `{`, and the command's
/// default format otherwise; standard output is the default unless --to says.
#[test]
fn standard_input_is_sniffed_for_a_document() {
    let dir = scratch("names-stdin");
    let none = dir.join("no-config");
    let doc = std::fs::read(sketch_doc(&dir)).expect("the .duet");
    let notes = std::fs::read(root().join("samples/sketch.notes")).expect("sample");
    let mid = std::fs::read(golden("sketch.mid")).expect("golden");

    // song show defaults to MIDI on standard input, and reads a document all the same.
    let mut padded = b"\n  \t".to_vec();
    padded.extend_from_slice(&doc);
    for input in [&doc, &padded] {
        let o = duet_with(&dir, &none, &["song", "show", "-"], input);
        ok(&o, "song show - of a document");
        assert_eq!(o.stdout, notes, "the sample, as text");
    }
    let o = duet_with(&dir, &none, &["song", "show", "-"], &mid);
    ok(&o, "song show - of MIDI");

    // convert defaults to text on standard input, and to a document when it is one.
    let o = duet_with(&dir, &none, &["convert", "-", "-", "--to", "duet"], &notes);
    ok(&o, "text in, a document out");
    assert_eq!(o.stdout, doc, "the same document");
    let o = duet_with(&dir, &none, &["convert", "-", "-"], &doc);
    ok(&o, "a document in, text out");
    assert_eq!(o.stdout, notes);

    // A document of another kind on standard input is refused by name.
    let beat = root().join("samples/groove.drums");
    ok(
        &duet(&dir, &["drums", "snap", s(&beat), s(&dir.join("g.duet"))]),
        "a drum-grid document",
    );
    let grid = std::fs::read(dir.join("g.duet")).expect("doc");
    let o = duet_with(&dir, &none, &["song", "show", "-"], &grid);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("drum-grid"));
    // The grid side of drums render reads it as a grid.
    let o = duet_with(&dir, &none, &["drums", "render", "-", "-"], &grid);
    ok(&o, "drums render - - of a drum-grid document");
    assert!(String::from_utf8_lossy(&o.stdout).contains("[track \"Drums\" ch10]"));

    // The grid side of drums render reads a grid's text from standard input.
    let beat = std::fs::read(root().join("samples/groove.drums")).expect("sample");
    let o = duet_with(&dir, &none, &["drums", "render", "-", "-"], &beat);
    ok(&o, "drums render - -");
    assert!(String::from_utf8_lossy(&o.stdout).contains("[track \"Drums\" ch10]"));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `duet convert` goes between `.duet`, `.mid` and `.abc` by their names alone, and back to
/// the same song.
#[test]
fn convert_goes_between_documents_midi_and_abc() {
    let dir = scratch("names-convert");
    let p = |name: &str| dir.join(name);
    let doc = sketch_doc(&dir);
    let notes = std::fs::read_to_string(root().join("samples/sketch.notes")).expect("sample");
    let show = |path: &Path| {
        let o = duet(&dir, &["song", "show", s(path)]);
        ok(&o, "song show");
        String::from_utf8(o.stdout).expect("utf-8")
    };

    ok(
        &duet(&dir, &["convert", s(&doc), s(&p("a.mid"))]),
        ".duet to .mid",
    );
    ok(
        &duet(&dir, &["convert", s(&p("a.mid")), s(&p("b.duet"))]),
        ".mid to .duet",
    );
    let json = |path: &Path| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    };
    let (a, b) = (json(&doc), json(&p("b.duet")));
    assert_eq!(b["body"], a["body"], "MIDI keeps the song");
    assert_eq!(
        b["duet"]["from"],
        serde_json::json!({ "format": "midi" }),
        "and says so"
    );
    assert_eq!(show(&p("b.duet")), notes);

    // ABC holds a single-voice tune: the ABC corpus's whole tune goes round.
    let tune = root().join("crates/duet-abc/corpus/several-tunes.abc");
    ok(
        &duet(
            &dir,
            &["abc", "render", s(&tune), s(&p("t.duet")), "--tune", "2"],
        ),
        "abc render to .duet",
    );
    ok(
        &duet(&dir, &["convert", s(&p("t.duet")), s(&p("t.abc"))]),
        ".duet to .abc",
    );
    ok(
        &duet(&dir, &["convert", s(&p("t.abc")), s(&p("t2.duet"))]),
        ".abc to .duet",
    );
    assert_eq!(
        show(&p("t2.duet")),
        show(&p("t.duet")),
        "ABC keeps the song"
    );
    let written = std::fs::read_to_string(p("t2.duet")).expect("written");
    assert!(
        written.contains("\"kind\": \"song\"") && written.contains("\"name\": \"duet\""),
        "{written}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
