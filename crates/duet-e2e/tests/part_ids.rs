//! A part document's `id` and `source`, through every command that writes one: the id follows
//! the file (a document written over itself keeps it, any other is a new part with a new id),
//! and the source follows the part (an import is by hand, a copy keeps its source, and an
//! operation's part is generated, naming the document it read).

use std::path::Path;

use duet_drums::DrumGrid;
use duet_e2e::{duet, golden, grid_doc, ok, s, scratch, view_text};
use duet_schemas::journal::DocDigest;
use duet_schemas::{Document, Kind, PartBody, Source};
use serde_json::json;

fn body(path: &Path) -> PartBody {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Document::<PartBody>::read(&text)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .body
}

/// A document's digest, as a view's `from` names one: the FNV-1a of its body as compact JSON,
/// written as its type writes it.
fn digest<K: Kind>(path: &Path) -> DocDigest {
    let doc = Document::<K>::read(&std::fs::read_to_string(path).unwrap()).unwrap();
    DocDigest::of(&serde_json::to_vec(&doc.body).unwrap())
}

fn generated(operation: &str, inputs: &[DocDigest]) -> Source {
    serde_json::from_value(json!({ "by": "generated", "operation": operation, "inputs": inputs }))
        .unwrap()
}

#[test]
fn an_import_is_by_hand_with_a_new_id_and_a_copy_keeps_its_source_with_a_new_id() {
    let dir = scratch("part-ids-copy");
    let doc = dir.join("sketch.duet");
    ok(
        &duet(&dir, &["convert", s(&golden("sketch.mid")), s(&doc)]),
        "convert from MIDI",
    );
    let first = body(&doc);
    assert_eq!(first.source, Source::hand(), "an import is by hand");

    let copy = dir.join("copy.duet");
    ok(
        &duet(&dir, &["convert", s(&doc), s(&copy)]),
        "convert a copy",
    );
    let copied = body(&copy);
    assert_ne!(copied.id, first.id, "a copy is a new part");
    assert_eq!(copied.source, first.source, "a copy keeps its source");

    // `part show --to duet` is a copy too.
    let out = duet(&dir, &["part", "show", s(&doc), "--to", "duet"]);
    ok(&out, "part show --to duet");
    let shown = Document::<PartBody>::read(&String::from_utf8_lossy(&out.stdout))
        .unwrap()
        .body;
    assert_eq!(shown.source, first.source);
    assert_ne!(shown.id, first.id);

    // Written over itself, a document keeps its id.
    ok(
        &duet(&dir, &["convert", s(&doc), s(&doc)]),
        "convert in place",
    );
    assert_eq!(body(&doc).id, first.id, "the same file is the same part");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn apply_onto_keeps_the_id_and_a_new_document_from_a_view_is_by_hand() {
    let dir = scratch("part-ids-apply");
    let doc = dir.join("sketch.duet");
    ok(
        &duet(&dir, &["convert", s(&golden("sketch.mid")), s(&doc)]),
        "convert from MIDI",
    );
    let first = body(&doc);
    let view = dir.join("sketch.notes");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&duet(&dir, &["view", s(&doc)]).stdout)
            .lines()
            .next()
            .unwrap(),
        view_text(&dir, &doc, "notes", &[]).replace(" 80\n", " 81\n")
    );
    std::fs::write(&view, text).unwrap();
    ok(
        &duet(&dir, &["apply", s(&view), "--onto", s(&doc)]),
        "apply --onto",
    );
    let applied = body(&doc);
    assert_eq!(
        applied.id, first.id,
        "applied over itself, the part keeps its id"
    );
    assert_eq!(applied.source, first.source);

    let elsewhere = dir.join("elsewhere.duet");
    ok(
        &duet(
            &dir,
            &[
                "apply",
                s(&view),
                "--onto",
                s(&doc),
                "-o",
                s(&elsewhere),
                "--force",
            ],
        ),
        "apply --onto -o",
    );
    assert_ne!(
        body(&elsewhere).id,
        first.id,
        "written elsewhere, it is a new part"
    );

    let fresh = dir.join("fresh.duet");
    ok(
        &duet(&dir, &["apply", s(&view), "-o", s(&fresh)]),
        "apply -o",
    );
    assert_eq!(body(&fresh).source, Source::hand());
    assert_ne!(body(&fresh).id, first.id);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn an_operations_part_is_generated_naming_the_document_it_read() {
    let dir = scratch("part-ids-generated");
    let doc = dir.join("sketch.duet");
    ok(
        &duet(&dir, &["convert", s(&golden("sketch.mid")), s(&doc)]),
        "convert from MIDI",
    );
    let quantized = dir.join("quantized.duet");
    ok(
        &duet(
            &dir,
            &["part", "quantize", s(&doc), s(&quantized), "--grid", "1/16"],
        ),
        "quantize",
    );
    assert_eq!(
        body(&quantized).source,
        generated("part quantize", &[digest::<PartBody>(&doc)])
    );
    assert_ne!(body(&quantized).id, body(&doc).id);

    let text = std::fs::read_to_string(golden("groove.drums")).unwrap();
    let grid = grid_doc(&dir, "groove.drums", &text);
    let rendered = dir.join("rendered.duet");
    ok(
        &duet(&dir, &["drums", "render", s(&grid), s(&rendered)]),
        "drums render",
    );
    assert_eq!(
        body(&rendered).source,
        generated("drums render", &[digest::<DrumGrid>(&grid)])
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
