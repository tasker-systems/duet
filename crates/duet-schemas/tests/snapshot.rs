//! snapshot 1.0: a snapshot's document validates against the committed schema, and what the
//! format refuses is refused by name: no `song.duet` first, a file that is not a part under
//! `parts/`, a file stated twice, and a digest that is not one.

use std::path::Path;

use duet_schemas::snapshot::{Copied, FileDigest};
use duet_schemas::{Document, Error, Snapshot, Tool};
use serde_json::{Value, json};

fn read_json(relative: &str) -> Value {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The committed envelope and snapshot schemas, registered by their `$id`, as a validator.
fn validator() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let snapshot = read_json("schemas/snapshot/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&snapshot)
        .unwrap()
}

fn snapshot() -> Document<Snapshot> {
    let copied =
        |path: &str, bytes: &[u8]| Copied::new(path.parse().unwrap(), FileDigest::of(bytes));
    let mut body = Snapshot::new(
        "take-2".parse().unwrap(),
        "2028-02-29".parse().unwrap(),
        vec![
            copied("song.duet", b"{}"),
            copied("parts/kit.duet", b""),
            copied("parts/lick-2.DUET", &[0xff, 0x00]),
        ],
    );
    body.said = Some("a line\nand \"another\"".into());
    Document::new(Tool::new("duet", "0.3.0"), body)
}

/// A snapshot's document as JSON, broken by `f`, read back.
fn read_broken(f: impl FnOnce(&mut Value)) -> Error {
    let mut doc: Value = serde_json::from_str(&snapshot().write().unwrap()).unwrap();
    f(&mut doc["body"]);
    Document::<Snapshot>::read(&doc.to_string()).unwrap_err()
}

#[test]
fn a_snapshot_validates_against_its_schema_and_names_its_folder() {
    let doc = snapshot();
    let text = doc.write().unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    let errors: Vec<String> = validator()
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}\n{text}");
    assert_eq!(doc.body.folder(), "2028-02-29-take-2");
    assert_eq!(
        value["body"]["takes"],
        json!([]),
        "no takes is stated as none"
    );
}

#[test]
fn a_snapshot_without_its_song_first_is_refused_by_name() {
    let err = read_broken(|body| {
        let files = body["files"].as_array_mut().unwrap();
        files.swap(0, 1);
    });
    assert!(
        err.to_string()
            .contains("files[0] is parts/kit.duet: a snapshot's first file is its song.duet"),
        "{err}"
    );
    let err = read_broken(|body| body["files"] = json!([]));
    assert!(err.to_string().contains("files is empty"), "{err}");
}

#[test]
fn a_file_that_is_no_part_or_is_stated_twice_is_refused_by_name() {
    for (path, says) in [
        ("comps/verse.duet", "files[1] is comps/verse.duet"),
        ("parts/a/b.duet", "files[1] is parts/a/b.duet"),
        ("song.duet", "files[1] is song.duet"),
        (
            "parts/LICK-2.duet",
            "files[2] states parts/LICK-2.duet twice",
        ),
    ] {
        let err = read_broken(|body| {
            let at = if path.starts_with("parts/LICK") { 2 } else { 1 };
            if at == 2 {
                body["files"][1]["path"] = json!("parts/lick-2.duet");
            }
            body["files"][at]["path"] = json!(path);
        });
        assert!(err.to_string().contains(says), "{path}: {err}");
    }
    let err = read_broken(|body| body["files"][1]["path"] = json!("../song.duet"));
    assert!(err.to_string().contains("not a relative path"), "{err}");
}

#[test]
fn a_digest_that_is_not_one_and_an_unknown_field_are_refused_by_name() {
    let err = read_broken(|body| body["files"][0]["digest"] = json!("sha256:00"));
    assert!(err.to_string().contains("is not a file digest"), "{err}");
    let err = read_broken(|body| body["status"] = json!("idea"));
    assert!(err.to_string().contains("status"), "{err}");
}
