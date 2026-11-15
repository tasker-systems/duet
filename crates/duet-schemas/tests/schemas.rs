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

use duet_core::{
    Control, Key, Note, Pitch, Position, Song, TempoChange, TimeChange, TimeSig, Track,
};
use duet_dsp::{FrameBlock, PerformanceHead};
use duet_schemas::{
    AnyDocument, Document, ENVELOPE, ENVELOPE_INFO, Framing, From, KINDS, KindInfo, Notice, Stream,
    Tool, Version, read_envelope,
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
    let (name, version) = (info.name, info.version);
    let path = schema_path(name, version);
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
    } else if name == ENVELOPE {
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

fn song_info() -> &'static KindInfo {
    duet_schemas::kind("song").unwrap()
}

#[test]
fn the_gate_fails_on_a_field_added_to_song_at_the_same_version_naming_song() {
    let committed = (song_info().schema)();
    let mut generated = committed.clone();
    generated["$defs"]["Song"]["properties"]["swing"] = json!({ "type": "number" });
    let Verdict::Shape(why) = verdict(song_info(), Some(&committed), &generated) else {
        panic!("a new field passed the gate");
    };
    assert!(
        why.contains("the schema of song changed at the same version 1.0"),
        "{why}"
    );
    assert!(
        why.contains("crates/duet-schemas/schemas/song/1.0.json"),
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
    let committed = (song_info().schema)();
    let mut generated = committed.clone();
    generated["$defs"]["Note"]["properties"]["vel"]["description"] = json!("How hard.");
    generated["$defs"]["Song"]["description"] = json!("A song, reworded.");
    let Verdict::Descriptions(why) = verdict(song_info(), Some(&committed), &generated) else {
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
    let Verdict::Missing(why) = verdict(song_info(), None, &(song_info().schema)()) else {
        panic!("a missing file passed the gate");
    };
    assert!(
        why.contains("crates/duet-schemas/schemas/song/1.0.json"),
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
        verdict(song_info(), Some(&committed), &generated),
        Verdict::Shape(_)
    ));
    let removed = json!({ "properties": {} });
    assert!(matches!(
        verdict(song_info(), Some(&committed), &removed),
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

/// The text of the example committed at a kind's current version, when blessing writes it.
fn example_text(info: &KindInfo) -> String {
    match info.framing {
        Framing::Stream => example_stream(info.name),
        _ => example(info.name).write().unwrap(),
    }
}

/// A stream's example: every column, voiced and unvoiced frames, and a short last block.
fn example_stream(name: &str) -> String {
    assert_eq!(name, "performance", "no example is defined for {name}");
    let mut stream = Stream::new(
        Tool::new("duet", "0.2.0"),
        PerformanceHead::new(22_050, 256, 512),
    );
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

/// The example committed at a kind's current version, when blessing writes it.
fn example(name: &str) -> AnyDocument {
    if name == "drum-grid" {
        return AnyDocument::DrumGrid(Document::new(Tool::new("duet", "0.2.0"), example_grid()));
    }
    assert_eq!(name, "song", "no example is defined for {name}");
    let mut doc = Document::new(Tool::new("duet", "0.2.0"), example_song());
    let mut from = From::new("abc");
    from.tune = Some("1".into());
    from.notices.push(Notice::new(7, 9, "slur", "("));
    doc.from = Some(from);
    doc.statements = vec!["the slur on bar 2 is notation only, and is not carried".into()];
    AnyDocument::Song(doc)
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

fn pos(s: &str) -> Position {
    s.parse().unwrap()
}

fn pitch(s: &str) -> Pitch {
    s.parse().unwrap()
}

/// A song that uses every field and every control.
fn example_song() -> Song {
    let mut song = Song::default();
    song.title = Some("The Example Reel".into());
    song.ticks_per_quarter = 480;
    song.tempo_bpm = 112.5;
    song.time_sig = TimeSig::new(4, 4).unwrap();
    song.key = Some("D dorian".parse::<Key>().unwrap());
    song.tempo_changes = vec![TempoChange {
        at: pos("3.1.0"),
        bpm: 108.0,
    }];
    song.time_changes = vec![TimeChange {
        at: pos("4.1.0"),
        time_sig: TimeSig::new(6, 8).unwrap(),
    }];
    song.key_changes = vec![duet_core::KeyChange {
        at: pos("4.1.0"),
        key: "Bb major".parse().unwrap(),
    }];
    song.end = Some(pos("6.1.0"));
    let mut lead = Track::new("fiddle", 1);
    lead.program = Some(40);
    lead.notes = vec![
        Note::new(pos("1.1.0"), pitch("D4"), 0.5, 90),
        Note::new(pos("1.1.240"), pitch("E4"), 0.5, 72),
        Note::new(pos("1.2.0"), pitch("F#4"), 1.0, 80),
        Note::new(pos("2.1.0"), pitch("Bb3"), 1.5, 64),
        Note::new(pos("4.1.0"), pitch("C5"), 0.333, 100),
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
    song.tracks = vec![lead, bass];
    song
}

#[test]
fn the_example_song_is_a_song_duet_can_write() {
    let song = example_song();
    song.check_end().unwrap();
    assert_eq!(song.stated_end().unwrap(), pos("6.1.0"));
    assert_eq!(Song::from_text(&song.to_text()).unwrap(), song);
}

#[test]
fn written_songs_validate_against_the_committed_schema() {
    let schema = committed_schema("song", Version::new(1, 0));
    let validator = validator(&schema);
    let minimal = Song::default();
    let mut untitled = example_song();
    untitled.title = None;
    untitled.key = None;
    untitled.end = None;
    untitled.tracks[0].program = None;
    untitled.tracks[0].controls.clear();
    for song in [example_song(), minimal, untitled] {
        let text = Document::new(Tool::new("duet", "0.2.0"), song)
            .write()
            .unwrap();
        assert_valid(&validator, &serde_json::from_str(&text).unwrap(), &text);
    }
}

#[test]
fn the_schema_refuses_what_the_reader_refuses() {
    let schema = committed_schema("song", Version::new(1, 0));
    let validator = validator(&schema);
    let good: Value = serde_json::from_str(&example(song_info().name).write().unwrap()).unwrap();
    type Break = fn(&mut Value);
    let cases: [(&str, Break); 8] = [
        ("an unknown body field", |d| d["body"]["swing"] = json!(0.6)),
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
    doc["$schema"] = json!(duet_schemas::schema_url("song", Version::new(1, 1)));
    assert!(
        !validator.is_valid(&doc),
        "the schema accepts another $schema"
    );
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
        &["C4", "C#4", "Db4", "Bb2", "c#-1", "G9", "Cb4", "A♯3", "E♭5"],
        &["H4", "C", "4", "C#x4", ""],
        &|s| s.parse::<Pitch>().is_ok(),
    );
    // Every value the writer writes matches too.
    let validator = jsonschema::validator_for(&defs["Pitch"]).unwrap();
    for midi in 0..=127 {
        for flat in [false, true] {
            let p = Pitch::new(midi).unwrap().spelled_flat(flat).to_string();
            assert!(validator.is_valid(&json!(p)), "{p}");
        }
    }
}

#[test]
fn an_unknown_field_inside_a_song_body_is_refused_by_name() {
    let text = example(song_info().name).write().unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    for (path, field) in [
        ("/body", "swing"),
        ("/body/tracks/0", "muted"),
        ("/body/tracks/0/notes/0", "len"),
        ("/body/tracks/0/controls/0", "channel"),
        ("/body/tempo_changes/0", "ramp"),
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
