//! The schema gate: every committed schema is what the code generates at the same version, and
//! every committed example still reads and validates.
//!
//! - No committed file for a kind's current version: fails, naming the file. Write it with
//!   `DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas`.
//! - A committed file whose shape differs (compared with every `description` stripped): fails,
//!   naming the kind, and asks for a version bump by the rule in the design's question 3.
//! - A committed file that differs only in descriptions: fails, and asks only to regenerate.
//! - Every committed version has an example (`<kind>/examples/<version>.duet`, or
//!   `<version>.duet.jsonl` for a stream's kind), which reads with this build's reader and
//!   validates against its version's schema: a stream's first line against the schema, and each
//!   later line on its own against the schema's `$defs/Record`.
//!
//! An older schema file is never regenerated, since the code no longer produces it: it is frozen
//! by review, and its example proves the reader still honours it.

use std::fs;
use std::path::{Path, PathBuf};

use duet_capture::Analysis;
use duet_core::{
    Control, Key, Note, Part, Pitch, Position, TempoChange, TimeChange, TimeSig, Track,
};
use duet_dsp::{FrameBlock, PerformanceHead};
use duet_schemas::journal::{
    AskAnswered, AskAsked, AskWithdrawn, At, Audience, By, Closes, Concerns, DocDigest, DocWritten,
    Entry, EventInfo, Id, IndexLine, Indexed, MomentPlayed, Opens, Party, RelPath, SegmentStamp,
    SnapshotTaken, SongStatusSet, TakeAdded, TakeRef, VerdictGiven,
};
use duet_schemas::part::CompedTake;
use duet_schemas::project::{Defaults, Given, ProjectTake};
use duet_schemas::snapshot::{Copied, FileDigest};
use duet_schemas::song::{Composition, KeyAt, MeterAt, Placement, Section, Status, TempoAt};
use duet_schemas::{
    AnyDocument, Document, ENVELOPE, ENVELOPE_INFO, EVENTS, Framing, From, IndexHead, Journal,
    JournalHead, JournalIndex, KINDS, KindInfo, Notice, PartBody, Project, Snapshot, Song, Source,
    Stream, Tool, Version, read_envelope, schema,
};
use serde_json::{Value, json};

const BLESS: &str = "DUET_BLESS_SCHEMAS";

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

/// `schemas/<name>/<version>.json`, relative to the crate, as messages name it.
fn schema_path(name: &str, version: Version) -> String {
    format!("crates/duet-schemas/schemas/{name}/{version}.json")
}

fn example_path(info: &KindInfo, version: Version) -> String {
    let name = info.name;
    match info.framing {
        Framing::Stream => {
            format!("crates/duet-schemas/schemas/{name}/examples/{version}.duet.jsonl")
        }
        _ => format!("crates/duet-schemas/schemas/{name}/examples/{version}.duet"),
    }
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}

// --- The comparison ---

/// `schema` with every `description` keyword removed. The keys of a `properties`, `$defs` or
/// `patternProperties` map are names, not keywords, so a property named `description` stays.
fn without_descriptions(schema: &Value) -> Value {
    fn walk(value: &Value, names: bool) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .filter(|(key, _)| names || key.as_str() != "description")
                    .map(|(key, value)| {
                        let holds_names = !names
                            && matches!(key.as_str(), "properties" | "$defs" | "patternProperties");
                        (key.clone(), walk(value, holds_names))
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|v| walk(v, false)).collect()),
            other => other.clone(),
        }
    }
    walk(schema, false)
}

/// What the gate makes of one schema: nothing to do, or why it fails. `bless` says whether the
/// generated schema may be written over the committed one.
#[derive(Debug, PartialEq)]
enum Verdict {
    Current,
    /// No file at this version. Blessing writes it.
    Missing(String),
    /// Only descriptions changed. Blessing writes it; no version moves.
    Descriptions(String),
    /// The shape changed at the same version. Blessing never writes it.
    Shape(String),
}

fn verdict(info: &KindInfo, committed: Option<&Value>, generated: &Value) -> Verdict {
    let what = if info.name == ENVELOPE {
        Target::Envelope
    } else {
        Target::Kind
    };
    verdict_at(what, info.name, info.version, committed, generated)
}

/// What a schema is the schema of: the envelope, a kind, or a journal event kind, which
/// versions on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Envelope,
    Kind,
    Event,
}

fn target_path(what: Target, name: &str, version: Version) -> String {
    match what {
        Target::Event => event_path(name, version),
        _ => schema_path(name, version),
    }
}

/// `schemas/journal/events/<name>/<version>.json`, as messages name it.
fn event_path(name: &str, version: Version) -> String {
    format!(
        "crates/duet-schemas/schemas/{}/{version}.json",
        schema::event_schema_name(name)
    )
}

fn verdict_at(
    what: Target,
    name: &str,
    version: Version,
    committed: Option<&Value>,
    generated: &Value,
) -> Verdict {
    let path = target_path(what, name, version);
    let Some(committed) = committed else {
        return Verdict::Missing(format!(
            "{name} {version} has no committed schema: {path} is missing. Write it with \
             `{BLESS}=1 cargo test -p duet-schemas`"
        ));
    };
    if committed == generated {
        Verdict::Current
    } else if without_descriptions(committed) == without_descriptions(generated) {
        Verdict::Descriptions(format!(
            "{name} {version}: only descriptions changed in {path}. Regenerate it with \
             `{BLESS}=1 cargo test -p duet-schemas`; no version change is needed"
        ))
    } else if what == Target::Event {
        Verdict::Shape(format!(
            "the schema of the journal event {name} changed at the same version {version} \
             ({path}). Bump {name}'s own version (its `v`) by the rule in the design's version \
             policy: minor for an additive change, major for any other. The journal's version \
             does not move"
        ))
    } else if what == Target::Envelope {
        Verdict::Shape(format!(
            "the schema of the envelope changed at {version} ({path}). A change to the \
             envelope's shape is a new major version of every kind: move ENVELOPE_VERSION and \
             every kind's major, with a converter for each"
        ))
    } else {
        Verdict::Shape(format!(
            "the schema of {name} changed at the same version {version} ({path}). Bump {name}'s \
             version by the rule in the design's version policy (question 3): minor for an \
             additive change, major for any other"
        ))
    }
}

fn targets() -> impl Iterator<Item = &'static KindInfo> {
    std::iter::once(&ENVELOPE_INFO).chain(KINDS)
}

#[test]
fn every_schema_is_committed_and_current() {
    let bless = std::env::var_os(BLESS).is_some();
    let mut failures = Vec::new();
    for info in targets() {
        let generated = (info.schema)();
        let path = repo_path(&schema_path(info.name, info.version));
        let committed = read_json(&path);
        match verdict(info, committed.as_ref(), &generated) {
            Verdict::Current => {}
            Verdict::Missing(_) | Verdict::Descriptions(_) if bless => {
                write_json(&path, &generated)
            }
            Verdict::Missing(why) | Verdict::Descriptions(why) | Verdict::Shape(why) => {
                failures.push(why);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the schema gate:\n- {}",
        failures.join("\n- ")
    );
}

/// The gate for each journal event kind's body schema, which versions on its own: committed
/// and current at the event's version, by the same verdicts as a kind's.
#[test]
fn every_event_schema_is_committed_and_current() {
    let bless = std::env::var_os(BLESS).is_some();
    let mut failures = Vec::new();
    for info in EVENTS {
        let generated = (info.schema)();
        let path = repo_path(&event_path(info.name, info.version));
        let committed = read_json(&path);
        match verdict_at(
            Target::Event,
            info.name,
            info.version,
            committed.as_ref(),
            &generated,
        ) {
            Verdict::Current => {}
            Verdict::Missing(_) | Verdict::Descriptions(_) if bless => {
                write_json(&path, &generated)
            }
            Verdict::Missing(why) | Verdict::Descriptions(why) | Verdict::Shape(why) => {
                failures.push(why);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the event schema gate:\n- {}",
        failures.join("\n- ")
    );
}

fn event_info(name: &str) -> &'static EventInfo {
    duet_schemas::event_kind(name).unwrap()
}

#[test]
fn the_gate_fails_on_a_field_added_to_an_event_at_the_same_version_naming_the_event() {
    let info = event_info("ask.answered");
    let committed = (info.schema)();
    let mut generated = committed.clone();
    generated["properties"]["mood"] = json!({ "type": "string" });
    let Verdict::Shape(why) = verdict_at(
        Target::Event,
        info.name,
        info.version,
        Some(&committed),
        &generated,
    ) else {
        panic!("a new field at the same version must fail as a shape change");
    };
    assert!(
        why.contains("ask.answered") && why.contains("its `v`"),
        "{why}"
    );
    assert!(why.contains("journal's version does not move"), "{why}");
}

/// A new event kind changes no committed schema but its own: the journal's schema does not
/// name any event, so it stays current however many there are.
#[test]
fn the_journal_schema_names_no_event() {
    let journal = (duet_schemas::kind("journal").unwrap().schema)().to_string();
    for info in EVENTS {
        assert!(
            !journal.contains(info.name),
            "journal's schema names {}",
            info.name
        );
    }
}

fn part_info() -> &'static KindInfo {
    duet_schemas::kind("part").unwrap()
}

#[test]
fn the_gate_fails_on_a_field_added_to_part_at_the_same_version_naming_part() {
    let committed = (part_info().schema)();
    let mut generated = committed.clone();
    generated["$defs"]["Part"]["properties"]["swing"] = json!({ "type": "number" });
    let Verdict::Shape(why) = verdict(part_info(), Some(&committed), &generated) else {
        panic!("a new field passed the gate");
    };
    assert!(
        why.contains("the schema of part changed at the same version 1.0"),
        "{why}"
    );
    assert!(
        why.contains("crates/duet-schemas/schemas/part/1.0.json"),
        "{why}"
    );
}

#[test]
fn the_gate_fails_on_a_field_added_to_the_envelope_or_from_naming_the_envelope() {
    let committed = (ENVELOPE_INFO.schema)();
    for def in [None, Some("From"), Some("Tool"), Some("Notice")] {
        let mut generated = committed.clone();
        let target = match def {
            None => &mut generated,
            Some(def) => &mut generated["$defs"][def],
        };
        target["properties"]["checksum"] = json!({ "type": "string" });
        let Verdict::Shape(why) = verdict(&ENVELOPE_INFO, Some(&committed), &generated) else {
            panic!("a new field in {def:?} passed the gate");
        };
        assert!(why.contains("the schema of the envelope changed"), "{why}");
        assert!(why.contains("new major version of every kind"), "{why}");
    }
}

#[test]
fn a_description_only_change_asks_to_regenerate_without_a_version_change() {
    let committed = (part_info().schema)();
    let mut generated = committed.clone();
    generated["$defs"]["Note"]["properties"]["vel"]["description"] = json!("How hard.");
    generated["$defs"]["PartBody"]["description"] = json!("A part, reworded.");
    let Verdict::Descriptions(why) = verdict(part_info(), Some(&committed), &generated) else {
        panic!("a reworded description was not seen as one");
    };
    assert!(
        why.contains("only descriptions changed") && why.contains(BLESS),
        "{why}"
    );
    assert!(
        why.contains("no version change is needed") && !why.contains("Bump"),
        "{why}"
    );
}

#[test]
fn a_missing_schema_fails_naming_the_file_and_how_to_write_it() {
    let Verdict::Missing(why) = verdict(part_info(), None, &(part_info().schema)()) else {
        panic!("a missing file passed the gate");
    };
    assert!(
        why.contains("crates/duet-schemas/schemas/part/1.0.json"),
        "{why}"
    );
    assert!(
        why.contains("DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas"),
        "{why}"
    );
}

#[test]
fn a_property_named_description_is_shape_not_description() {
    let committed = json!({ "properties": { "description": { "type": "string" } } });
    let generated = json!({ "properties": { "description": { "type": "number" } } });
    assert!(matches!(
        verdict(part_info(), Some(&committed), &generated),
        Verdict::Shape(_)
    ));
    let removed = json!({ "properties": {} });
    assert!(matches!(
        verdict(part_info(), Some(&committed), &removed),
        Verdict::Shape(_)
    ));
}

// --- Validation against the committed schemas ---

/// Every committed envelope schema, by its `$id`, so that a kind's `$ref` to it resolves without
/// any network or file resolver.
fn validator(schema: &Value) -> jsonschema::Validator {
    validator_with(schema, &[])
}

/// A validator for each later line of a stream whose schema is `stream`: its `$defs/Record`, by
/// reference, with the stream's schema registered beside the envelopes.
fn record_validator(stream: &Value) -> jsonschema::Validator {
    let id = stream["$id"].as_str().unwrap();
    validator_with(&json!({ "$ref": format!("{id}#/$defs/Record") }), &[stream])
}

/// [`validator`], with `more` schemas registered by their `$id` too.
fn validator_with(schema: &Value, more: &[&Value]) -> jsonschema::Validator {
    let envelopes: Vec<Value> = committed_versions(ENVELOPE)
        .into_iter()
        .map(|v| read_json(&repo_path(&schema_path(ENVELOPE, v))).unwrap())
        .collect();
    let mut registry = jsonschema::Registry::new();
    for schema in envelopes.iter().chain(more.iter().copied()) {
        registry = registry
            .add(schema["$id"].as_str().unwrap(), schema)
            .unwrap();
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .unwrap_or_else(|e| panic!("{} does not compile: {e}", schema["$id"]))
}

fn assert_valid(validator: &jsonschema::Validator, document: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(document)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{what} does not validate:\n{}",
        errors.join("\n")
    );
}

/// The versions committed under `schemas/<name>/`, oldest first.
fn committed_versions(name: &str) -> Vec<Version> {
    let mut versions: Vec<Version> = fs::read_dir(schemas_dir().join(name))
        .map(|dir| {
            dir.filter_map(|entry| {
                let file = entry.ok()?.file_name().into_string().ok()?;
                file.strip_suffix(".json")?.parse().ok()
            })
            .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions
}

fn committed_schema(name: &str, version: Version) -> Value {
    read_json(&repo_path(&schema_path(name, version))).unwrap()
}

#[test]
fn every_committed_example_reads_and_validates_against_its_version() {
    let bless = std::env::var_os(BLESS).is_some();
    // Blessing writes a missing schema here too, since the tests run in parallel.
    for info in targets() {
        let current = repo_path(&schema_path(info.name, info.version));
        if bless && !current.exists() {
            write_json(&current, &(info.schema)());
        }
    }
    for info in KINDS {
        let versions = committed_versions(info.name);
        assert!(
            !versions.is_empty(),
            "{} has no committed schema",
            info.name
        );
        for version in versions {
            let relative = example_path(info, version);
            let path = repo_path(&relative);
            if bless && version == info.version && !path.exists() {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, example_text(info)).unwrap();
            }
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("{} {version} has no example: {relative}", info.name));
            let first = match info.framing {
                Framing::Stream => text.lines().next().unwrap_or_default(),
                _ => &text,
            };
            let envelope = read_envelope(first).unwrap_or_else(|e| panic!("{relative}: {e}"));
            assert_eq!(
                (envelope.kind.as_str(), envelope.version),
                (info.name, version)
            );
            if info.framing == Framing::Stream {
                check_stream_example(info, version, &text, &relative);
                continue;
            }
            let doc = AnyDocument::read(&text).unwrap_or_else(|e| panic!("{relative}: {e}"));
            assert_eq!(doc.kind(), info.name);
            let schema = committed_schema(info.name, version);
            assert_valid(
                &validator(&schema),
                &serde_json::from_str(&text).unwrap(),
                &relative,
            );
            if version == info.version {
                assert_eq!(
                    doc.write().unwrap(),
                    text,
                    "{relative} is not as this build writes it"
                );
            }
        }
    }
}

/// A stream's example: it reads with this build's reader, its first line validates against its
/// version's schema, and each later line validates on its own against the schema's
/// `$defs/Record`.
fn check_stream_example(info: &KindInfo, version: Version, text: &str, relative: &str) {
    let written = match info.name {
        "performance" => Stream::<PerformanceHead>::read(text)
            .and_then(|stream| stream.write())
            .unwrap_or_else(|e| panic!("{relative}: {e}")),
        "journal" => {
            check_journal_events(text, relative);
            Journal::read(text)
                .and_then(|journal| journal.write())
                .unwrap_or_else(|e| panic!("{relative}: {e}"))
        }
        "journal-index" => JournalIndex::read(text)
            .and_then(|index| index.write())
            .unwrap_or_else(|e| panic!("{relative}: {e}")),
        other => panic!("no stream reader is named for {other}"),
    };
    let schema = committed_schema(info.name, version);
    let (head, records) = (validator(&schema), record_validator(&schema));
    let mut lines = text.lines().enumerate();
    let (_, first) = lines.next().expect("a stream has a first line");
    assert_valid(
        &head,
        &serde_json::from_str(first).unwrap(),
        &format!("{relative}:1"),
    );
    let mut later = 0;
    for (n, line) in lines {
        let what = format!("{relative}:{}", n + 1);
        assert_valid(&records, &serde_json::from_str(line).unwrap(), &what);
        later += 1;
    }
    assert!(later > 0, "{relative} has no record to validate");
    if version == info.version {
        assert_eq!(written, text, "{relative} is not as this build writes it");
    }
}

/// Each entry of a journal's example validates its body against its event kind's committed
/// schema at its `v`, and the example holds every event kind this build knows.
fn check_journal_events(text: &str, relative: &str) {
    let mut seen = Vec::new();
    for (n, line) in text.lines().enumerate().skip(1) {
        let entry: Value = serde_json::from_str(line).unwrap();
        let (name, v) = (
            entry["event"].as_str().unwrap(),
            entry["v"].as_str().unwrap(),
        );
        let version: Version = v.parse().unwrap();
        let path = repo_path(&event_path(name, version));
        let schema = read_json(&path).unwrap_or_else(|| panic!("{}: no schema", path.display()));
        assert_valid(
            &validator(&schema),
            &entry["body"],
            &format!("{relative}:{} ({name} {v})", n + 1),
        );
        seen.push(name.to_string());
    }
    for info in EVENTS {
        assert!(
            seen.iter().any(|name| name == info.name),
            "{relative} has no {} entry",
            info.name
        );
    }
}

/// The text of the example committed at a kind's current version, when blessing writes it.
fn example_text(info: &KindInfo) -> String {
    match info.framing {
        Framing::Stream => example_stream(info.name),
        _ => example(info.name).write().unwrap(),
    }
}

/// A stream's example: every column, voiced and unvoiced frames, a short last block, and the
/// take (1.1).
fn example_stream(name: &str) -> String {
    if name == "journal" {
        return example_journal();
    }
    if name == "journal-index" {
        return example_index();
    }
    assert_eq!(name, "performance", "no example is defined for {name}");
    let mut head = PerformanceHead::new(22_050, 256, 512);
    head.take = Some(
        serde_json::from_value(json!({
            "digest": EXAMPLE_DIGEST, "rate": 44100, "channels": 2, "frames": 1764000
        }))
        .unwrap(),
    );
    let mut stream = Stream::new(Tool::new("duet", "0.2.0"), head);
    stream.records = vec![
        FrameBlock::new(
            512,
            vec![None, Some(6907.9), Some(6908.3), Some(6907.1)],
            vec![0.01, 0.912_345_7, 0.957_1, 0.880_6],
            vec![0.000_12, 0.213_7, 0.241_05, 0.238_9],
        )
        .unwrap(),
        FrameBlock::new(
            1536,
            vec![Some(6913.0), None],
            vec![0.794_2, 0.010_3],
            vec![0.198_8, 0.002_1],
        )
        .unwrap(),
    ];
    stream.write().unwrap()
}

fn id(n: u8) -> Id {
    Id::from_parts(1_795_446_245_000 + u64::from(n), [n; 10])
}

/// A journal's example: one entry of every event kind, every optional field used once, and a
/// `cause`.
fn example_journal() -> String {
    let head = JournalHead::new("wayfaring", id(20), example_stamp(1));
    let mut text = Journal::head_line(&Tool::new("duet", "0.3.0"), &head).unwrap();
    let claude = By::new("claude", "cli");
    let owner = By::new("owner", "cli");
    let at = |n: u8| At::from_unix_ms(1_795_446_245_000 + 1000 * u64::from(n));
    let mut asked = AskAsked::new("Which of the two endings do you hear as the tune?");
    asked.about = Some("wayfaring@2.15-3.58".into());
    asked.options = vec!["one".into(), "two".into()];
    asked.clips = vec!["wayfaring@2.15-2.58".into(), "wayfaring@3.15-3.58".into()];
    asked.to = Some(Party::Owner);
    let mut answered = AskAnswered::new(id(1), "two");
    answered.said = Some("a new word".into());
    let mut withdrawn = AskWithdrawn::new(id(3));
    withdrawn.why = Some("asked twice".into());
    let mut verdict = VerdictGiven::new("wayfaring@2.15-3.58", "no audible difference");
    verdict.ask = Some(id(1));
    let digest = EXAMPLE_DIGEST.parse().unwrap();
    let wayfaring: duet_schemas::project::Name = "wayfaring".parse().unwrap();
    let mut lead = DocWritten::new(
        "songs/wayfaring/parts/lead.duet".parse().unwrap(),
        "part",
        Version::new(1, 0),
        DocDigest(0x0f1e_2d3c_4b5a_6978),
        true,
    );
    lead.song = Some(id(30));
    lead.part = Some(id(31));
    let entries = [
        Entry::new(id(1), at(1), claude.clone(), asked),
        Entry::new(id(2), at(2), owner.clone(), answered),
        Entry::new(id(3), at(3), claude.clone(), AskAsked::new("Again?")),
        Entry::new(id(4), at(4), claude.clone(), withdrawn),
        Entry::new(
            id(5),
            at(5),
            claude.clone(),
            MomentPlayed::new("wayfaring@2.15-3.58", Audience::Owner, "take"),
        ),
        Entry::new(id(6), at(6), owner.clone(), verdict),
        Entry::new(
            id(7),
            at(7),
            owner.clone(),
            TakeAdded::new("wayfaring", digest),
        ),
        Entry::new(
            id(8),
            at(8),
            claude.clone(),
            DocWritten::new(
                "takes/wayfaring.capture.duet".parse().unwrap(),
                "capture-analysis",
                Version::new(1, 13),
                DocDigest(0x0123_4567_89ab_cdef),
                false,
            ),
        ),
        Entry::new(id(9), at(9), claude.clone(), lead),
        Entry::new(
            id(10),
            at(10),
            owner.clone(),
            SongStatusSet::new(id(30), wayfaring.clone(), Status::Idea, Status::Sketch),
        ),
        Entry::new(
            id(11),
            at(11),
            owner,
            SnapshotTaken::new(
                id(30),
                wayfaring,
                "2026-11-20-first-verse",
                DocDigest(0xfedc_ba98_7654_3210),
            ),
        ),
    ];
    for (n, mut entry) in entries.into_iter().enumerate() {
        if n == 7 {
            entry.cause = Some(id(7));
        }
        text.push_str(&entry.line().unwrap());
    }
    text
}

/// The stamp of the example's `n`th segment.
fn example_stamp(n: u64) -> SegmentStamp {
    SegmentStamp::new(At::from_unix_ms(1_795_446_245_000 + 86_400_000 * (n - 1)))
}

/// An index's example: the first line opens a segment, the second closes it with every field of
/// its summary and opens the next.
fn example_index() -> String {
    let head = IndexHead::new("wayfaring", id(0));
    let mut text = JournalIndex::head_line(&Tool::new("duet", "0.3.0"), &head).unwrap();
    let first = Opens::new(example_stamp(1), id(20), example_stamp(1).at());
    text.push_str(&IndexLine::new(first, None).line().unwrap());
    let mut concerns = Concerns::new();
    concerns.songs = vec![Indexed::new(Some(id(30)), "wayfaring")];
    concerns.parts = vec![
        Indexed::new(None, "wayfaring/kit"),
        Indexed::new(Some(id(31)), "wayfaring/lead"),
    ];
    concerns.takes = vec![TakeRef::new("wayfaring", EXAMPLE_DIGEST.parse().unwrap())];
    let closes = Closes::new(
        example_stamp(1),
        At::from_unix_ms(1_795_446_256_000),
        11,
        id(11),
        concerns,
    );
    let second = Opens::new(example_stamp(2), id(21), example_stamp(2).at());
    text.push_str(&IndexLine::new(second, Some(closes)).line().unwrap());
    text
}

/// The example committed at a kind's current version, when blessing writes it.
fn example(name: &str) -> AnyDocument {
    if name == "drum-grid" {
        return AnyDocument::DrumGrid(Document::new(Tool::new("duet", "0.2.0"), example_grid()));
    }
    if name == "capture-analysis" {
        return AnyDocument::CaptureAnalysis(Box::new(Document::new(
            Tool::new("duet", "0.2.0"),
            example_analysis(),
        )));
    }
    if name == "project" {
        return AnyDocument::Project(Document::new(Tool::new("duet", "0.3.0"), example_project()));
    }
    if name == "song" {
        return AnyDocument::Song(Document::new(Tool::new("duet", "0.3.0"), example_song()));
    }
    if name == "snapshot" {
        return AnyDocument::Snapshot(Document::new(
            Tool::new("duet", "0.3.0"),
            example_snapshot(),
        ));
    }
    assert_eq!(name, "part", "no example is defined for {name}");
    let mut doc = Document::new(Tool::new("duet", "0.2.0"), part_body(example_part()));
    let mut from = From::new("abc");
    from.tune = Some("1".into());
    from.notices.push(Notice::new(7, 9, "slur", "("));
    doc.from = Some(from);
    doc.statements = vec!["the slur on bar 2 is notation only, and is not carried".into()];
    AnyDocument::Part(doc)
}

/// A song that uses every field: an intent, a tempo, meter and key change, sections with a
/// name twice, and placements layered, repeated and cut to a stretch, with a stated end.
fn example_song() -> Song {
    let mut composition = Composition::new(92.0, "3/4".parse().unwrap(), "G minor".parse().ok());
    composition.tempo.push(TempoAt::new(pos("17.1.0"), 96.5));
    composition
        .meter
        .push(MeterAt::new(pos("25.1.0"), "4/4".parse().unwrap()));
    composition
        .key
        .push(KeyAt::new(pos("17.1.0"), "A minor".parse().unwrap()));
    composition.sections = vec![
        Section::new("verse", pos("1.1.0")),
        Section::new("verse", pos("9.1.0")),
        Section::new("lift", pos("17.1.0")),
        Section::new("coda", pos("25.1.0")),
    ];
    let mut kit = Placement::new("kit".parse().unwrap(), pos("9.1.0"));
    kit.repeat = Some(2);
    let mut tail = Placement::new("melody".parse().unwrap(), pos("25.1.0"));
    tail.from = Some(pos("1.1.0"));
    tail.to = Some(pos("5.1.0"));
    composition.placements = vec![
        Placement::new("melody".parse().unwrap(), pos("1.1.0")),
        Placement::new("bass".parse().unwrap(), pos("9.1.0")),
        kit,
        tail,
    ];
    composition.end = Some(pos("29.1.0"));
    let mut song = Song::new(id(30), "Wayfaring Stranger", composition);
    song.status = Status::Sketch;
    song.intent = Some("slow and bare; voice first, the bass enters in the second verse".into());
    song
}

/// A snapshot that uses every field: the owner's words, the song and two placed parts, and a
/// linked take.
fn example_snapshot() -> Snapshot {
    let copied = |path: &str| {
        Copied::new(
            path.parse().unwrap(),
            FileDigest::of(format!("the bytes of {path}").as_bytes()),
        )
    };
    let mut snapshot = Snapshot::new(
        "first-verse".parse().unwrap(),
        "2026-11-20".parse().unwrap(),
        vec![
            copied("song.duet"),
            copied("parts/melody.duet"),
            copied("parts/bass.duet"),
        ],
    );
    snapshot.said = Some("the first verse sits; the bass is right at last".into());
    snapshot.takes = vec![TakeRef::new("wayfaring", EXAMPLE_DIGEST.parse().unwrap())];
    snapshot
}

/// A project that uses every field: every default, a take with something given and one
/// without, each named by its digest.
fn example_project() -> Project {
    let date = |s: &str| s.parse().unwrap();
    let mut project = Project::new("Porch", date("2026-11-17"), Version::new(0, 3));
    let mut defaults = Defaults::default();
    defaults.tempo_bpm = Some(92.0);
    defaults.meter = Some("3/4".parse().unwrap());
    defaults.key = Some("Ab".parse().unwrap());
    defaults.tuning = Some("+0c".parse().unwrap());
    defaults.grid = Some("1/16".parse().unwrap());
    project.defaults = Some(defaults);
    let take = |digest: &str, rate: u32, frames: u64| {
        serde_json::from_value(json!({
            "digest": digest, "rate": rate, "channels": 1, "frames": frames
        }))
        .unwrap()
    };
    let mut wayfaring = ProjectTake::new(
        "wayfaring".parse().unwrap(),
        take(EXAMPLE_DIGEST, 44_100, 976_080),
        "wayfaring-stranger.flac".parse().unwrap(),
        date("2026-11-17"),
    );
    let mut given = Given::default();
    given.tempo_bpm = Some(92.0);
    wayfaring.given = Some(given);
    let blood_sport = ProjectTake::new(
        "blood-sport".parse().unwrap(),
        take(
            "sha256:41ab07e3c9d25f18b6a4e0c7d3f92b5a8e1c6d4f7a0b3e9c2d5f8a1b4c7e0d36",
            48_000,
            1_203_200,
        ),
        "Blood Sport 3.m4a".parse().unwrap(),
        date("2026-11-18"),
    );
    project.takes = vec![wayfaring, blood_sport];
    project
}

/// A grid that uses every field: a meter change (so bars differ in length), its own levels,
/// a lane gm does not name, another channel, an end inside its last bar, and velocities off
/// the text's levels, which only the document holds.
fn example_grid() -> duet_drums::DrumGrid {
    let text = "title \"The Example Groove\"\ntpq 480\ntempo 96\ntime 4/4\ntime 3/4 at 2.1.0\n\
        key A minor\nend 3.3.0\ngrid 1/8\nmap gm\nchannel 11\nvel g 30 x 80 X 110\n\
        lane cajon 80\n\n[bars 1-3]\n\
        hh    | x x x x x x x x | x x x x x x | x x x x . . |\n\
        snare | . . X . . . X g | . . X . . . | . . X . . . |\n\
        kick  | X . . X X . . . | X . . . X . | X . . . . . |\n\
        cajon | . g . . . g . . | . . . g . . | . . . . . . |\n";
    let mut grid = duet_drums::DrumGrid::from_text(text).expect("the example's text reads");
    let mut rows: Vec<Vec<u8>> = (0..grid.lanes.len())
        .map(|l| grid.row(l).to_vec())
        .collect();
    rows[0][1] = 64;
    rows[2][0] = 127;
    rows[2][3] = 101;
    let bar_steps = grid.bar_steps.clone();
    grid.set_matrix(bar_steps, rows).expect("a whole matrix");
    grid.check().expect("the example is a valid grid");
    grid
}

/// A melody take's analysis that uses every field: each kind of event, an estimated tempo with
/// its half and double, a given meter, an estimated downbeat, a key with its evidence, the tempo
/// beat by beat, the grid, the quantize offsets, and the reading (1.1) with a note between keys
/// and a note with excursions (1.6), rejoined from two decoded notes (1.7), a note carried
/// on over its breathy fall (1.9), a note made from a pitch held before the note it reaches,
/// suggested a likely approach to it (1.11), the key's profile (1.12), and the variant
/// annotation (1.13): the take's digest, rate, channels and length, and the passes that ran.
pub fn example_analysis() -> Analysis {
    serde_json::from_value(json!({
        "rate": 22050,
        "take": {
            "digest": EXAMPLE_DIGEST,
            "rate": 44100,
            "channels": 2,
            "frames": 1764000
        },
        "events": [
            { "type": "onset", "frame": 11025, "strength": 0.8125 },
            { "type": "note", "start": 11100, "end": 21600, "midi": 63, "confidence": 0.93,
              "strength": 0.8125 },
            { "type": "onset", "frame": 22050, "strength": 0.5 },
            { "type": "note", "start": 22100, "end": 32600, "midi": 67, "confidence": 0.875,
              "strength": 0.5 },
            { "type": "hit", "frame": 33075, "lane": "asked", "strength": 0.25 }
        ],
        "timing": {
            "tempo_bpm": { "estimated": { "value": 120.0, "confidence": 0.84 } },
            "tempo_alternatives": { "half": 60.0, "double": 240.0 },
            "beat_period_frames": 11025.0,
            "meter": { "given": "4/4" },
            "downbeat_frame": { "estimated": { "value": 11025, "confidence": 0.62 } }
        },
        "key": {
            "key": { "estimated": { "value": "Eb major", "confidence": 0.71 } },
            "alternative": "C minor",
            "histogram": [0.0, 0.0, 0.125, 0.25, 0.0, 0.0, 0.0, 0.375, 0.125, 0.0, 0.125, 0.0],
            "scores": [["Eb major", 0.71], ["C minor", 0.64], ["Bb major", 0.52]],
            "profile": "aarden-essen"
        },
        "beats": [
            { "beat": 0, "secs": 0.5, "bpm": 119.5 },
            { "beat": 1, "secs": 1.002, "bpm": 120.25 }
        ],
        "grid": "1/16",
        "offsets": [
            { "note": 0, "moved_ms": -3.4, "moved_steps": -0.027 },
            { "note": 1, "moved_ms": 2.25, "moved_steps": 0.018 }
        ],
        "reading": {
            "tuning": {
                "a4": { "estimated": { "value": 6913.4, "confidence": 0.81 } },
                "spread_cents": 9.25
            },
            "reader": { "name": "hmm", "note_cost": 4.0 },
            "notes": [
                { "start": 11100, "end": 21600, "held_from": 11356, "key": 63, "centre": -2.5,
                  "between_keys": false, "confidence": 0.93, "strength": 0.8125,
                  "residual": {
                      "centre": -1.75,
                      "drift": { "cents_per_sec": 4.5, "range_cents": 2.125 },
                      "modulation": { "rate_hz": 5.75, "depth_cents": 31.5 },
                      "scoop": { "cents": -180.0, "ms": 92.875 },
                      "rest_cents": 3.25,
                      "frames": 41,
                      "excursions": [
                          { "start": 11100, "end": 11868, "cents": -96.5 },
                          { "start": 14428, "end": 15452, "cents": 198.0 }
                      ]
                  } },
                { "start": 22100, "end": 32600, "held_from": 22612, "key": 67, "centre": 38.0,
                  "between_keys": true, "confidence": 0.875, "strength": 0.5,
                  "residual": {
                      "centre": 37.5,
                      "drift": { "cents_per_sec": -1.25, "range_cents": 0.5 },
                      "fall": { "cents": -95.0, "ms": 58.0 },
                      "rest_cents": 2.0,
                      "frames": 40
                  } }
            ],
            "residual": {
                "centres": { "from_cents": -50.0, "bin_cents": 10.0,
                             "counts": [0, 0, 0, 0, 1, 0, 0, 0, 1, 0] },
                "centre_cents_per_sec": 78.5,
                "between_keys": 1,
                "read_frames": 81,
                "unread_frames": 3,
                "loss": { "pitch": 31.25, "unread": 3.0, "notes": 8.0 }
            },
            "rejoins": [
                { "decoded": [
                      { "start": 11100, "end": 16200, "held_from": 11356, "key": 63,
                        "centre": -2.0, "between_keys": false, "confidence": 0.9375,
                        "strength": 0.8125 },
                      { "start": 16200, "end": 21600, "held_from": 17224, "key": 63,
                        "centre": -3.5, "between_keys": false, "confidence": 0.921875,
                        "strength": 0.0625 }
                  ],
                  "dips": [ { "start": 16200, "end": 17224, "cents": -172.5 } ],
                  "loss_change": 6.375 }
            ],
            "breathy": [
                { "decoded": { "start": 22100, "end": 31832, "held_from": 22612, "key": 67,
                               "centre": 38.0, "between_keys": true, "confidence": 0.90625,
                               "strength": 0.5 },
                  "end": 32600, "lowest_db": -9.5, "loss_change": 2.25 }
            ],
            "held": [
                { "decoded": { "start": 34100, "end": 41500, "held_from": 36400, "key": 70,
                               "centre": 4.5, "between_keys": false, "confidence": 0.875,
                               "strength": 0.75 },
                  "notes": [
                      { "start": 34100, "end": 35900, "held_from": 34600, "key": 69,
                        "centre": 21.5, "between_keys": false, "confidence": 0.84375,
                        "strength": 0.75 }
                  ],
                  "loss_change": 1.625 }
            ],
            "suggestions": [
                { "start": 34100, "key": 69, "suggests": "likely-approach",
                  "rule": "semitone-with-drift", "next_key": 70, "step": 1,
                  "drift_cents": 43.5 }
            ],
            "passes": ["breathy", "rejoin", "held-pitch"]
        }
    }))
    .expect("the example analysis reads")
}

/// The take the examples state: a digest written as duet writes one.
const EXAMPLE_DIGEST: &str =
    "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34";

fn pos(s: &str) -> Position {
    s.parse().unwrap()
}

fn pitch(s: &str) -> Pitch {
    s.parse().unwrap()
}

/// A part that uses every field and every control.
fn example_part() -> Part {
    let mut part = Part::default();
    part.title = Some("The Example Reel".into());
    part.ticks_per_quarter = 480;
    part.tempo_bpm = 112.5;
    part.time_sig = TimeSig::new(4, 4).unwrap();
    part.key = Some("D dorian".parse::<Key>().unwrap());
    part.tempo_changes = vec![TempoChange {
        at: pos("3.1.0"),
        bpm: 108.0,
    }];
    part.time_changes = vec![TimeChange {
        at: pos("4.1.0"),
        time_sig: TimeSig::new(6, 8).unwrap(),
    }];
    part.key_changes = vec![duet_core::KeyChange {
        at: pos("4.1.0"),
        key: "Bb major".parse().unwrap(),
    }];
    part.end = Some(pos("6.1.0"));
    let mut lead = Track::new("fiddle", 1);
    lead.program = Some(40);
    lead.notes = vec![
        Note::new(pos("1.1.0"), pitch("D4"), 0.5, 90),
        Note::new(pos("1.1.240"), pitch("E4"), 0.5, 72),
        Note::new(pos("1.2.0"), pitch("F#4"), 1.0, 80),
        Note::new(pos("2.1.0"), pitch("Bb3"), 1.5, 64),
        Note::new(pos("4.1.0"), pitch("C5"), 0.333, 100),
        // Spellings beyond a sharp or flat on a black key: Cb, E#, a double sharp and flat.
        Note::new(pos("4.2.0"), pitch("Cb5"), 0.5, 80),
        Note::new(pos("4.3.0"), pitch("E#4"), 0.5, 80),
        Note::new(pos("4.4.0"), pitch("F##4"), 0.5, 80),
        Note::new(pos("5.1.0"), pitch("Bbb3"), 0.5, 80),
    ];
    lead.controls = vec![
        Control::Cc {
            at: pos("1.1.0"),
            controller: 64,
            value: 127,
        },
        Control::Bend {
            at: pos("2.1.0"),
            value: -8192,
        },
        Control::Program {
            at: pos("3.1.0"),
            program: 41,
        },
    ];
    let mut bass = Track::new("bass", 2);
    bass.notes = vec![Note::new(pos("1.1.0"), pitch("D2"), 4.0, 1)];
    part.tracks = vec![lead, bass];
    part
}

#[test]
fn the_example_part_is_a_part_duet_can_write() {
    let part = example_part();
    part.check_end().unwrap();
    assert_eq!(part.stated_end().unwrap(), pos("6.1.0"));
    assert_eq!(Part::from_text(&part.to_text()).unwrap(), part);
}

#[test]
fn written_parts_validate_against_the_committed_schema() {
    let schema = committed_schema("part", part_info().version);
    let validator = validator(&schema);
    let minimal = Part::default();
    let mut untitled = example_part();
    untitled.title = None;
    untitled.key = None;
    untitled.end = None;
    untitled.tracks[0].program = None;
    untitled.tracks[0].controls.clear();
    for part in [example_part(), minimal, untitled] {
        for source in sources() {
            let text = Document::new(
                Tool::new("duet", "0.2.0"),
                PartBody::new(id(7), source, part.clone()),
            )
            .write()
            .unwrap();
            assert_valid(&validator, &serde_json::from_str(&text).unwrap(), &text);
        }
    }
}

/// A part's body: the part, by hand, with an id.
fn part_body(part: Part) -> PartBody {
    PartBody::new(id(7), Source::hand(), part)
}

/// One source of each kind, and a comped one with every field.
fn sources() -> Vec<Source> {
    let digest = EXAMPLE_DIGEST.parse().unwrap();
    let mut take = CompedTake::new(digest);
    take.take = Some("wayfaring".parse().unwrap());
    take.reading = Some("basic-pitch".into());
    let full = serde_json::from_value(json!({
        "by": "comped",
        "takes": [take, CompedTake::new(digest)],
        "comp": "verse-comp",
    }))
    .unwrap();
    let generated = serde_json::from_value(json!({
        "by": "generated", "operation": "drums render", "inputs": ["00ff00ff00ff00ff"]
    }))
    .unwrap();
    vec![
        Source::hand(),
        Source::reading(digest, Some("rules".into())),
        full,
        generated,
    ]
}

#[test]
fn the_schema_refuses_what_the_reader_refuses() {
    let schema = committed_schema("part", part_info().version);
    let validator = validator(&schema);
    let good: Value = serde_json::from_str(&example(part_info().name).write().unwrap()).unwrap();
    type Break = fn(&mut Value);
    let cases: [(&str, Break); 14] = [
        ("an unknown body field", |d| d["body"]["swing"] = json!(0.6)),
        ("a part with no id", |d| {
            d["body"].as_object_mut().unwrap().remove("id");
        }),
        ("a part with no source", |d| {
            d["body"].as_object_mut().unwrap().remove("source");
        }),
        ("an id that is not a UUIDv7", |d| {
            d["body"]["id"] = json!("0196f1a2-7b3c-4d4e-8f90-a1b2c3d4e5f6")
        }),
        ("a source of no kind", |d| {
            d["body"]["source"] = json!({ "by": "magic" })
        }),
        ("a source with an unknown field", |d| {
            d["body"]["source"] = json!({ "by": "hand", "who": "claude" })
        }),
        ("a comped source with no takes", |d| {
            d["body"]["source"] = json!({ "by": "comped" })
        }),
        ("an unknown note field", |d| {
            d["body"]["tracks"][0]["notes"][0]["len"] = json!(1)
        }),
        ("velocity 0", |d| {
            d["body"]["tracks"][0]["notes"][0]["vel"] = json!(0)
        }),
        ("velocity 128", |d| {
            d["body"]["tracks"][0]["notes"][0]["vel"] = json!(128)
        }),
        ("channel 17", |d| {
            d["body"]["tracks"][0]["channel"] = json!(17)
        }),
        ("a null title", |d| d["body"]["title"] = Value::Null),
        ("another kind in duet", |d| {
            d["duet"]["kind"] = json!("drum-grid")
        }),
        ("an unknown envelope field", |d| {
            d["duet"]["checksum"] = json!(1)
        }),
    ];
    for (what, break_it) in cases {
        let mut doc = good.clone();
        break_it(&mut doc);
        assert!(!validator.is_valid(&doc), "the schema accepts {what}");
    }
    let mut doc = good.clone();
    doc["$schema"] = json!(duet_schemas::schema_url("part", Version::new(1, 1)));
    assert!(
        !validator.is_valid(&doc),
        "the schema accepts another $schema"
    );
    // The pitch pattern refuses what no spelling keeps.
    for pitch in ["C###4", "C#b4"] {
        let mut doc = good.clone();
        doc["body"]["tracks"][0]["notes"][0]["pitch"] = json!(pitch);
        assert!(!validator.is_valid(&doc), "the schema accepts {pitch}");
    }
}

/// Each hand-written pattern matches every form its type writes and its reader takes, and
/// refuses forms the reader refuses.
#[test]
fn the_hand_written_patterns_match_what_reads() {
    fn schema_of<T: schemars::JsonSchema>() -> Value {
        schemars::generate::SchemaSettings::draft2020_12()
            .into_generator()
            .into_root_schema_for::<T>()
            .to_value()
    }
    let defs = json!({
        "Position": schema_of::<Position>(),
        "TimeSig": schema_of::<TimeSig>(),
        "Key": schema_of::<Key>(),
        "Grid": schema_of::<duet_core::Grid>(),
        "Pitch": schema_of::<Pitch>(),
    });
    let check = |def: &str, reads: &[&str], refused: &[&str], parses: &dyn Fn(&str) -> bool| {
        let validator = jsonschema::validator_for(&defs[def]).unwrap();
        for s in reads {
            assert!(parses(s), "{def}: {s:?} should parse");
            assert!(
                validator.is_valid(&json!(s)),
                "{def}: the pattern refuses {s:?}"
            );
        }
        for s in refused {
            assert!(!parses(s), "{def}: {s:?} should not parse");
            assert!(
                !validator.is_valid(&json!(s)),
                "{def}: the pattern accepts {s:?}"
            );
        }
    };
    check(
        "Position",
        &["1.1.0", "3.2.240", "12.4", "1.1.0000", "01.1.5"],
        &["0.1.0", "1.0.0", "1", "1.1.1.1", "1..0", "a.b.c", ""],
        &|s| s.parse::<Position>().is_ok(),
    );
    check(
        "TimeSig",
        &["4/4", "6/8", "7/16", "3/2", "1/1", "12/64", " 5 / 8 "],
        &["4/3", "0/4", "4/128", "4", "4-4", ""],
        &|s| s.parse::<TimeSig>().is_ok(),
    );
    check(
        "Key",
        &[
            "D minor",
            "Bb major",
            "D dorian",
            "F# mixolydian",
            "C",
            "Am",
            "ebm",
            "G Lydian",
            "Cb major",
        ],
        &["H major", "D blues", "", "#C"],
        &|s| s.parse::<Key>().is_ok(),
    );
    check(
        "Grid",
        &["1/16", "1/8t", "1/4", "1/1", " 1/32 "],
        &["1/0", "2/16", "1/8x", "16", ""],
        &|s| s.parse::<duet_core::Grid>().is_ok(),
    );
    check(
        "Pitch",
        &[
            "C4", "C#4", "Db4", "Bb2", "c#-1", "G9", "Cb4", "A♯3", "E♭5", "F##4", "Bbb3", "C♯♯4",
        ],
        &["H4", "C", "4", "C#x4", "", "C###4", "Cbbb4", "C#b4"],
        &|s| s.parse::<Pitch>().is_ok(),
    );
    let journal_defs = json!({
        "Id": schema_of::<Id>(),
        "At": schema_of::<At>(),
        "RelPath": schema_of::<RelPath>(),
        "DocDigest": schema_of::<DocDigest>(),
        "SegmentStamp": schema_of::<SegmentStamp>(),
    });
    let check_journal =
        |def: &str, reads: &[&str], refused: &[&str], parses: &dyn Fn(&str) -> bool| {
            let validator = jsonschema::validator_for(&journal_defs[def]).unwrap();
            for s in reads {
                assert!(parses(s), "{def}: {s:?} should parse");
                assert!(
                    validator.is_valid(&json!(s)),
                    "{def}: the pattern refuses {s:?}"
                );
            }
            for s in refused {
                assert!(!parses(s), "{def}: {s:?} should not parse");
                assert!(
                    !validator.is_valid(&json!(s)),
                    "{def}: the pattern accepts {s:?}"
                );
            }
        };
    check_journal(
        "Id",
        &[
            "0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6",
            "00000000-0000-7000-b000-000000000000",
        ],
        &[
            "0196F1A2-7b3c-7d4e-8f90-a1b2c3d4e5f6",
            "0196f1a2-7b3c-4d4e-8f90-a1b2c3d4e5f6",
            "",
        ],
        &|s| s.parse::<Id>().is_ok(),
    );
    check_journal(
        "At",
        &["2026-11-17T15:04:05.123Z", "1970-01-01T00:00:00.000Z"],
        &[
            "2026-11-17T15:04:05Z",
            "2026-11-17 15:04:05.123Z",
            "2026-11-17T15:04:05.123",
        ],
        &|s| s.parse::<At>().is_ok(),
    );
    check_journal(
        "RelPath",
        &["comps/a.duet", ".hidden", "..x", "a/...", "a b/c"],
        &["/abs", "a/../b", "./a", "a//b", "a/", "", "..", "a\\b"],
        &|s| s.parse::<RelPath>().is_ok(),
    );
    check_journal(
        "SegmentStamp",
        &["20261117T150405123Z", "19700101T000000000Z"],
        &[
            "2026-11-17T15:04:05.123Z",
            "20261117T150405Z",
            "20261117T150405123",
            "",
        ],
        &|s| s.parse::<SegmentStamp>().is_ok(),
    );
    check_journal(
        "DocDigest",
        &["0123456789abcdef"],
        &[
            "0123456789ABCDEF",
            "0123456789abcde",
            "fnv1a:0123456789abcdef",
        ],
        &|s| s.parse::<DocDigest>().is_ok(),
    );
    // Every value the writer writes matches too.
    let validator = jsonschema::validator_for(&defs["Pitch"]).unwrap();
    for midi in 0..=127 {
        for alter in -2..=2 {
            if let Some(p) = Pitch::new(midi).unwrap().spelled(alter) {
                let p = p.to_string();
                assert!(validator.is_valid(&json!(p)), "{p}");
            }
        }
    }
}

#[test]
fn an_unknown_field_inside_a_part_body_is_refused_by_name() {
    let text = example(part_info().name).write().unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    for (path, field) in [
        ("/body", "swing"),
        ("/body/tracks/0", "muted"),
        ("/body/tracks/0/notes/0", "len"),
        ("/body/tracks/0/controls/0", "channel"),
        ("/body/tempo_changes/0", "ramp"),
        ("/body/source", "who"),
    ] {
        let mut broken = doc.clone();
        broken.pointer_mut(path).unwrap()[field] = json!(1);
        let err = AnyDocument::read(&broken.to_string()).unwrap_err();
        assert!(
            err.to_string().contains(&format!("`{field}`")),
            "{path}: {err}"
        );
    }
    doc["duet"]["kind"] = json!("lute-tab");
    doc["$schema"] = json!(duet_schemas::schema_url("lute-tab", Version::new(1, 0)));
    let err = AnyDocument::read(&doc.to_string()).unwrap_err();
    assert!(err.to_string().contains("knows no kind lute-tab"), "{err}");
}
