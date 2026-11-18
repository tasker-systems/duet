//! project 1.0: a project's document round-trips exactly through its reader and validates
//! against the committed schema, its committed example reads, and what the format refuses is
//! refused by name: an unknown field, two takes with one name or one digest, an unknown `given`
//! field.

use std::path::Path;

use duet_schemas::project::{Defaults, Given, ProjectTake};
use duet_schemas::{AnyDocument, Document, Error, Kind, Project, Tool, Version};
use serde_json::{Value, json};

fn read_text(relative: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}

fn read_json(relative: &str) -> Value {
    serde_json::from_str(&read_text(relative)).unwrap()
}

/// The committed envelope and project schemas, registered by their `$id`, as a validator.
fn validator() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let project = read_json("schemas/project/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&project)
        .unwrap()
}

fn take(digest_byte: char, name: &str) -> ProjectTake {
    let digest = format!("sha256:{}", digest_byte.to_string().repeat(64));
    ProjectTake::new(
        name.parse().unwrap(),
        serde_json::from_value(json!({
            "digest": digest, "rate": 48000, "channels": 2, "frames": 4_800_000
        }))
        .unwrap(),
        format!("{name}.wav").parse().unwrap(),
        "2026-11-18".parse().unwrap(),
    )
}

/// A project that uses every field, unlike the committed example in its values: odd floats, a
/// triplet grid, a modal key, a tuning in Hz, and every `given` field.
fn project() -> Document<Project> {
    let mut project = Project::new(
        "Songs for the porch, \"vol. 2\"",
        "2024-02-29".parse().unwrap(),
        Version::new(0, 3),
    );
    let mut defaults = Defaults::default();
    defaults.tempo_bpm = Some(91.837_5);
    defaults.meter = Some("6/8".parse().unwrap());
    defaults.key = Some("D dorian".parse().unwrap());
    defaults.tuning = Some("442.5".parse().unwrap());
    defaults.grid = Some("1/8t".parse().unwrap());
    project.defaults = Some(defaults);
    let mut given = Given::default();
    given.tempo_bpm = Some(0.1 + 0.2);
    given.meter = Some("4/4".parse().unwrap());
    given.key = Some("F# minor".parse().unwrap());
    let mut first = take('a', "wayfaring");
    first.given = Some(given);
    project.takes = vec![first, take('b', "blood-sport-v2")];
    Document::new(Tool::new("duet", "0.3.0"), project)
}

#[test]
fn a_project_round_trips_exactly_and_validates_against_its_schema() {
    let doc = project();
    let text = doc.write().unwrap();
    let back = Document::<Project>::read(&text).unwrap();
    assert_eq!(back, doc);
    assert_eq!(
        back.write().unwrap(),
        text,
        "the text is not written back exactly"
    );
    let validator = validator();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert!(
        validator.is_valid(&value),
        "{:?}",
        validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
    // A project with no defaults and no takes, as `duet init` will write it.
    let bare = Document::new(
        Tool::new("duet", "0.3.0"),
        Project::new("porch", "2026-11-18".parse().unwrap(), Version::new(0, 3)),
    );
    let text = bare.write().unwrap();
    assert!(!text.contains("defaults"), "{text}");
    assert_eq!(Document::<Project>::read(&text).unwrap(), bare);
    assert!(validator.is_valid(&serde_json::from_str(&text).unwrap()));
}

#[test]
fn the_committed_example_reads() {
    let text = read_text("schemas/project/examples/1.0.duet");
    let AnyDocument::Project(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("the example is not read as a project");
    };
    assert_eq!(doc.body.title, "Porch");
    let names: Vec<&str> = doc.body.takes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["wayfaring", "blood-sport"]);
    assert!(
        doc.body
            .take(&"wayfaring".parse().unwrap())
            .unwrap()
            .given
            .is_some()
    );
    assert!(validator().is_valid(&serde_json::from_str(&text).unwrap()));
}

/// The body of `project()` as JSON, changed by `edit`, then read.
fn read_edited(edit: impl FnOnce(&mut Value)) -> (Error, bool) {
    let mut value: Value = serde_json::from_str(&project().write().unwrap()).unwrap();
    edit(&mut value["body"]);
    let valid = validator().is_valid(&value);
    let error = Document::<Project>::read(&value.to_string()).unwrap_err();
    (error, valid)
}

#[test]
fn an_unknown_field_is_refused_by_name() {
    let (error, valid) = read_edited(|body| body["songs"] = json!(["wayfaring-stranger"]));
    assert!(matches!(error, Error::Body { .. }), "{error:?}");
    assert!(
        error.to_string().contains("unknown field `songs`"),
        "{error}"
    );
    assert!(!valid, "the schema accepts a project with `songs`");
    // The dropped fields are refused too, and on a take as on the project.
    for (field, at) in [
        ("kept", "take"),
        ("restriction", "take"),
        ("licence", "project"),
    ] {
        let (error, valid) = read_edited(|body| match at {
            "take" => body["takes"][1][field] = json!("copied"),
            _ => body[field] = json!("CC-BY"),
        });
        assert!(
            error
                .to_string()
                .contains(&format!("unknown field `{field}`")),
            "{error}"
        );
        assert!(!valid, "the schema accepts `{field}`");
    }
    let (error, valid) = read_edited(|body| body["defaults"]["lanes"] = json!({}));
    assert!(
        error.to_string().contains("unknown field `lanes`"),
        "{error}"
    );
    assert!(!valid, "the schema accepts a default it does not know");
}

#[test]
fn a_duplicate_take_name_is_refused_by_name() {
    let (error, valid) = read_edited(|body| body["takes"][1]["name"] = json!("wayfaring"));
    assert!(matches!(error, Error::Body { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains(r#"two takes are named "wayfaring""#),
        "{error}"
    );
    // A schema cannot say that names differ: the reader is what refuses it.
    assert!(valid);
    // A name that is not one is refused, naming it.
    let (error, _) = read_edited(|body| body["takes"][1]["name"] = json!("Wayfaring"));
    assert!(
        error.to_string().contains(r#""Wayfaring" is not a name"#),
        "{error}"
    );
}

#[test]
fn two_takes_with_one_digest_are_refused_naming_both() {
    let (error, valid) = read_edited(|body| {
        body["takes"][1]["take"]["digest"] = body["takes"][0]["take"]["digest"].clone();
    });
    assert!(matches!(error, Error::Body { .. }), "{error:?}");
    let said = error.to_string();
    assert!(
        said.contains(r#"takes "wayfaring" and "blood-sport-v2" are one take"#),
        "{said}"
    );
    assert!(
        said.contains(&format!("sha256:{}", "a".repeat(64))),
        "{said}"
    );
    // A schema cannot say that digests differ: the reader is what refuses it.
    assert!(valid);
}

#[test]
fn an_unknown_given_field_is_refused_by_name() {
    let (error, valid) = read_edited(|body| body["takes"][0]["given"]["tuning"] = json!("+0c"));
    assert!(matches!(error, Error::Body { .. }), "{error:?}");
    assert!(
        error.to_string().contains("unknown field `tuning`"),
        "{error}"
    );
    assert!(
        !valid,
        "the schema accepts a `given` field it does not know"
    );
}

#[test]
fn what_else_a_take_or_a_default_cannot_be_is_refused_naming_it() {
    type Edit = fn(&mut Value);
    let cases: [(&str, Edit); 5] = [
        ("is not a file's name", |b| {
            b["takes"][0]["file"] = json!("takes/wayfaring/wayfaring.flac");
        }),
        ("is not a date", |b| {
            b["takes"][0]["added"] = json!("2026-02-30")
        }),
        ("takes[wayfaring].given.tempo_bpm is 0", |b| {
            b["takes"][0]["given"]["tempo_bpm"] = json!(0.0);
        }),
        ("cents from 440 Hz", |b| {
            b["defaults"]["tuning"] = json!("+150c")
        }),
        ("time signature denominator 5", |b| {
            b["defaults"]["meter"] = json!("3/5")
        }),
    ];
    for (said, edit) in cases {
        let (error, _) = read_edited(edit);
        assert!(error.to_string().contains(said), "{said}: {error}");
    }
}

#[test]
fn the_kind_is_project_1_0() {
    assert_eq!(
        (Project::NAME, Project::VERSION),
        ("project", Version::new(1, 0))
    );
    assert!(duet_schemas::kind("project").is_some());
}
