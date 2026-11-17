//! A song imported from ABC or MIDI records where it came from in its envelope's `from`, and
//! the provenance follows the song: kept when a `.duet` is read and written again, kept by
//! quantizing, replaced when the song goes out to ABC or MIDI and in again, and absent for a
//! song read from text.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, golden, ok, root, s, scratch};
use serde_json::{Value, json};

/// An ABC tune with a slur over two notes, on line 6.
const SLURRED: &str = "X:1\nT:Slurred\nM:4/4\nL:1/8\nK:G\nG2 (AB) c2 d2 | e8 |]\n";

fn json_of(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("written")).expect("a document")
}

/// The committed schema of `name` at 1.0.
fn schema(name: &str, version: &str) -> Value {
    let path = root().join(format!("crates/duet-schemas/schemas/{name}/{version}.json"));
    serde_json::from_slice(&std::fs::read(path).expect("committed")).expect("json")
}

/// Validates a written document against the committed song schema, with the committed
/// envelope schema registered for its `duet`.
fn assert_valid(doc: &Value) {
    let envelope = schema("envelope", "1.0");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().expect("an $id"), &envelope)
        .expect("a URI")
        .prepare()
        .expect("a registry");
    let song = schema("song", "1.1");
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&song)
        .expect("the schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{doc}\ndoes not validate:\n{}",
        errors.join("\n")
    );
}

fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let abc = dir.join("slurred.abc");
    std::fs::write(&abc, SLURRED).expect("write");
    (dir, abc)
}

#[test]
fn an_abc_import_records_its_notices_at_their_lines_and_columns() {
    let (dir, abc) = setup("from-abc");
    let doc = dir.join("slurred.duet");
    ok(
        &duet(&dir, &["convert", s(&abc), s(&doc)]),
        "convert .abc to .duet",
    );
    let written = json_of(&doc);
    let from = &written["duet"]["from"];
    assert_eq!(from["format"], "abc", "{from}");
    assert_eq!(from["tune"], "1", "{from}");
    let notices = from["notices"].as_array().expect("notices");
    for (col, found) in [(4, "("), (7, ")")] {
        let want = json!({ "line": 6, "col": col, "construct": "slur", "found": found });
        assert!(notices.contains(&want), "{want} in {from}");
    }
    // Every notice duet abc check lists is recorded, at the same line and column.
    let check = duet(&dir, &["abc", "check", s(&abc)]);
    ok(&check, "abc check");
    let listed = String::from_utf8_lossy(&check.stdout);
    for notice in notices {
        let at = format!("line {}, col {}: ", notice["line"], notice["col"]);
        assert!(listed.contains(&at), "{at} in\n{listed}");
    }
    assert_valid(&written);

    // abc render records the same, naming the tune.
    let rendered = dir.join("rendered.duet");
    ok(
        &duet(&dir, &["abc", "render", s(&abc), s(&rendered)]),
        "abc render",
    );
    assert_eq!(json_of(&rendered)["duet"]["from"], *from);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn out_to_abc_and_in_again_gives_the_same_body_and_a_new_provenance() {
    let (dir, abc) = setup("from-abc-again");
    let first = dir.join("first.duet");
    ok(&duet(&dir, &["convert", s(&abc), s(&first)]), "in");
    let out = dir.join("out.abc");
    ok(&duet(&dir, &["convert", s(&first), s(&out)]), "out to .abc");
    let again = dir.join("again.duet");
    ok(&duet(&dir, &["convert", s(&out), s(&again)]), "in again");
    let (first, again) = (json_of(&first), json_of(&again));
    assert_eq!(again["body"], first["body"], "the same song");
    // The new import is the new provenance: duet's own ABC holds no slur, so it records
    // none, and the first import's notices are gone.
    let from = &again["duet"]["from"];
    assert_eq!(from["format"], "abc", "{from}");
    assert_eq!(from["tune"], "1", "{from}");
    assert_ne!(*from, first["duet"]["from"], "replaced, not kept");
    let notices = from["notices"].as_array().cloned().unwrap_or_default();
    assert!(notices.iter().all(|n| n["construct"] != "slur"), "{from}");
    assert_valid(&again);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_midi_import_records_midi() {
    let dir = scratch("from-midi");
    let doc = dir.join("sketch.duet");
    ok(
        &duet(&dir, &["convert", s(&golden("sketch.mid")), s(&doc)]),
        "convert .mid",
    );
    let written = json_of(&doc);
    assert_eq!(written["duet"]["from"], json!({ "format": "midi" }));
    assert_valid(&written);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_document_read_and_written_again_keeps_its_from() {
    let (dir, abc) = setup("from-kept");
    let doc = dir.join("a.duet");
    ok(&duet(&dir, &["convert", s(&abc), s(&doc)]), "in");
    let from = json_of(&doc)["duet"]["from"].clone();

    // Read and written again: the document is byte for byte the same.
    let again = dir.join("b.duet");
    ok(&duet(&dir, &["convert", s(&doc), s(&again)]), "rewritten");
    assert_eq!(std::fs::read(&again).unwrap(), std::fs::read(&doc).unwrap());
    let shown = duet(&dir, &["song", "show", s(&doc), "--to", "duet"]);
    ok(&shown, "song show --to duet");
    assert_eq!(shown.stdout, std::fs::read(&doc).unwrap(), "song show too");

    // Quantizing keeps it: the notices still name what the import did not carry.
    let quantized = dir.join("q.duet");
    ok(
        &duet(
            &dir,
            &["song", "quantize", s(&doc), s(&quantized), "--grid", "1/16"],
        ),
        "quantize",
    );
    assert_eq!(json_of(&quantized)["duet"]["from"], from);
    assert_valid(&json_of(&quantized));

    // A song read from a view's text has no provenance.
    let notes = root().join("samples/sketch.notes");
    let plain = dir.join("plain.duet");
    ok(
        &duet(&dir, &["apply", s(&notes), "-o", s(&plain)]),
        "from text",
    );
    assert!(json_of(&plain)["duet"].get("from").is_none());
    std::fs::remove_dir_all(&dir).expect("clean up");
}
