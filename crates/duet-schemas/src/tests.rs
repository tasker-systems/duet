//! The envelope and the version policy, witnessed with kinds defined here: no real type
//! implements `Kind` yet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Document, Envelope, Error, From, Kind, Notice, Stream, StreamKind, Tool, Version,
    read_envelope, schema_url,
};

/// A document kind at 1.2, whose `key` was added at 1.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Tune {
    title: String,
    tempo: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

impl Kind for Tune {
    const NAME: &'static str = "tune";
    const VERSION: Version = Version::new(1, 2);
}

/// A stream kind: a head, then blocks of frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Frames {
    rate: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Block {
    start: u64,
    pitch: Vec<Option<f32>>,
}

impl Kind for Frames {
    const NAME: &'static str = "frames";
    const VERSION: Version = Version::new(1, 0);
}

impl StreamKind for Frames {
    type Record = Block;
}

fn tool() -> Tool {
    Tool::new("duet", "0.3.0")
}

fn tune() -> Document<Tune> {
    let mut doc = Document::new(
        tool(),
        Tune {
            title: "Reel".into(),
            tempo: 92.125,
            key: Some("D minor".into()),
        },
    );
    let mut from = From::new("abc");
    from.tune = Some("1".into());
    from.notices.push(Notice::new(4, 12, "slur", "("));
    doc.from = Some(from);
    doc.statements = vec!["tempo 92, estimated (confidence 0.81)".into()];
    doc
}

/// A `tune` document's text at `version`, with `body` and `$schema` as given.
fn tune_text(version: &str, schema: Option<serde_json::Value>, body: serde_json::Value) -> String {
    let mut doc = json!({
        "duet": { "kind": "tune", "version": version, "tool": { "name": "duet", "version": "0.3.0" } },
        "body": body,
    });
    if let Some(schema) = schema {
        doc["$schema"] = schema;
    }
    doc.to_string()
}

fn tune_schema(version: &str) -> Option<serde_json::Value> {
    Some(json!(schema_url("tune", version.parse().unwrap())))
}

fn body() -> serde_json::Value {
    json!({ "title": "Reel", "tempo": 92.0 })
}

// --- The envelope round-trips ---

#[test]
fn a_document_round_trips_exactly() {
    let doc = tune();
    let text = doc.write().unwrap();
    let back = Document::<Tune>::read(&text).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.write().unwrap(), text);
}

#[test]
fn a_document_is_written_schema_then_envelope_then_body() {
    let text = tune().write().unwrap();
    let schema = text.find("\"$schema\"").unwrap();
    let duet = text.find("\"duet\"").unwrap();
    let body = text.find("\"body\"").unwrap();
    assert!(schema < duet && duet < body, "{text}");
    assert!(text.contains(&schema_url("tune", Version::new(1, 2))));
    assert!(text.ends_with("}\n"));
}

#[test]
fn an_envelope_without_from_or_statements_omits_them() {
    let text = Document::new(
        tool(),
        Tune {
            title: "A".into(),
            tempo: 1.0,
            key: None,
        },
    )
    .write()
    .unwrap();
    assert!(
        !text.contains("\"from\"") && !text.contains("\"statements\""),
        "{text}"
    );
    assert!(!text.contains("null"), "{text}");
}

#[test]
fn the_envelope_reads_before_the_body() {
    let envelope = read_envelope(&tune().write().unwrap()).unwrap();
    let mut expected = Envelope::new("tune", Version::new(1, 2), tool());
    expected.from = tune().from;
    expected.statements = tune().statements;
    assert_eq!(envelope, expected);
    // The body is not read: one that is no Tune still gives its envelope.
    let text = tune_text("1.2", tune_schema("1.2"), json!({ "anything": [1, 2] }));
    assert_eq!(read_envelope(&text).unwrap().kind, "tune");
}

fn frames() -> Stream<Frames> {
    let mut stream = Stream::new(tool(), Frames { rate: 22050 });
    stream.statements = vec!["pitch from pYIN".into()];
    stream.records = vec![
        Block {
            start: 0,
            pitch: vec![Some(6900.5), None, Some(-1.25e-3)],
        },
        Block {
            start: 3,
            pitch: vec![],
        },
        Block {
            start: 3,
            pitch: vec![Some(f32::MAX), Some(f32::MIN_POSITIVE)],
        },
    ];
    stream
}

#[test]
fn a_stream_round_trips_exactly() {
    let stream = frames();
    let text = stream.write().unwrap();
    let back = Stream::<Frames>::read(&text).unwrap();
    assert_eq!(back, stream);
    assert_eq!(back.write().unwrap(), text);
}

#[test]
fn a_streams_first_line_is_the_envelope_and_each_later_line_one_record() {
    let stream = frames();
    let text = stream.write().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1 + stream.records.len());
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        first["$schema"],
        json!(schema_url("frames", Version::new(1, 0)))
    );
    assert_eq!(first["duet"]["kind"], json!("frames"));
    assert_eq!(first["head"], json!({ "rate": 22050 }));
    for (line, record) in lines[1..].iter().zip(&stream.records) {
        assert_eq!(&serde_json::from_str::<Block>(line).unwrap(), record);
    }
    assert_eq!(read_envelope(lines[0]).unwrap(), stream.envelope());
}

#[test]
fn a_stream_cut_at_any_line_still_reads() {
    let text = frames().write().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    for n in 1..=lines.len() {
        let cut = lines[..n].join("\n");
        assert_eq!(Stream::<Frames>::read(&cut).unwrap().records.len(), n - 1);
    }
}

#[test]
fn a_streams_bad_record_is_refused_naming_its_line() {
    let mut text = frames().write().unwrap();
    text.push_str("{\"start\":9,\"pitch\":[],\"level\":[]}\n");
    let err = Stream::<Frames>::read(&text).unwrap_err();
    assert!(matches!(err, Error::Line { line: 5, .. }), "{err:?}");
    assert!(err.to_string().contains("level"), "{err}");

    assert!(matches!(
        Stream::<Frames>::read(""),
        Err(Error::EmptyStream)
    ));
}

#[test]
fn a_document_and_a_stream_are_not_read_as_each_other() {
    let envelope = |kind: &str, version: &str| json!({ "kind": kind, "version": version, "tool": { "name": "duet", "version": "0" } });
    let head = json!({ "duet": envelope("tune", "1.2"), "head": body() });
    let err = Document::<Tune>::read(&head.to_string()).unwrap_err();
    assert!(matches!(err, Error::Framing { .. }), "{err:?}");

    let doc = json!({ "duet": envelope("frames", "1.0"), "body": { "rate": 1 } });
    let err = Stream::<Frames>::read(&doc.to_string()).unwrap_err();
    let Error::Line { line: 1, source } = err else {
        panic!("{err:?}")
    };
    assert!(matches!(*source, Error::Framing { .. }), "{source:?}");
}

// --- Each refusal is made by name ---

#[test]
fn a_newer_minor_is_refused_naming_both_versions() {
    let text = tune_text("1.3", tune_schema("1.3"), body());
    let err = Document::<Tune>::read(&text).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::NewerMinor { kind, found, newest }
                if kind == "tune" && *found == Version::new(1, 3) && *newest == Version::new(1, 2)
        ),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("tune 1.3") && msg.contains("up to 1.2") && msg.contains("update duet"),
        "{msg}"
    );
}

#[test]
fn another_major_is_refused_pointing_at_the_converter() {
    for version in ["2.0", "0.9"] {
        let text = tune_text(version, tune_schema(version), body());
        let err = Document::<Tune>::read(&text).unwrap_err();
        assert!(
            matches!(&err, Error::OtherMajor { found, reads: 1, .. } if found.to_string() == version),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("duet convert") && msg.contains("tune 1.x"),
            "{msg}"
        );
    }
}

#[test]
fn an_unknown_field_in_the_body_is_refused_by_name() {
    let text = tune_text(
        "1.2",
        tune_schema("1.2"),
        json!({ "title": "Reel", "tempo": 92.0, "tempoo": 1 }),
    );
    let err = Document::<Tune>::read(&text).unwrap_err();
    assert!(matches!(err, Error::Body { kind: "tune", .. }), "{err:?}");
    assert!(err.to_string().contains("`tempoo`"), "{err}");
}

#[test]
fn a_field_from_a_newer_minor_in_an_older_body_is_refused_by_name() {
    // Suppose 1.0 had no `key`: a body that claims 1.0 and holds it still reads, since `key` is
    // optional at 1.2. But a field no minor up to 1.2 knows is refused at every version.
    let text = tune_text(
        "1.0",
        tune_schema("1.0"),
        json!({ "title": "R", "tempo": 1.0, "swing": 0.6 }),
    );
    let err = Document::<Tune>::read(&text).unwrap_err();
    assert!(err.to_string().contains("`swing`"), "{err}");
}

#[test]
fn an_unknown_field_in_the_frame_or_the_envelope_is_refused_by_name() {
    let mut doc: serde_json::Value = serde_json::from_str(&tune().write().unwrap()).unwrap();
    doc["comment"] = json!("hi");
    let err = Document::<Tune>::read(&doc.to_string()).unwrap_err();
    assert!(
        matches!(&err, Error::UnknownField { field, .. } if field == "comment"),
        "{err:?}"
    );
    assert!(err.to_string().contains("`comment`"), "{err}");

    let mut doc: serde_json::Value = serde_json::from_str(&tune().write().unwrap()).unwrap();
    doc["duet"]["checksum"] = json!(1);
    let err = Document::<Tune>::read(&doc.to_string()).unwrap_err();
    assert!(
        matches!(&err, Error::UnknownField { field, .. } if field == "checksum"),
        "{err:?}"
    );

    for (path, field) in [("tool", "os"), ("from", "url")] {
        let mut doc: serde_json::Value = serde_json::from_str(&tune().write().unwrap()).unwrap();
        doc["duet"][path][field] = json!("x");
        let err = Document::<Tune>::read(&doc.to_string()).unwrap_err();
        assert!(matches!(err, Error::Envelope(_)), "{err:?}");
        assert!(err.to_string().contains(&format!("`{field}`")), "{err}");
    }
    let mut doc: serde_json::Value = serde_json::from_str(&tune().write().unwrap()).unwrap();
    doc["duet"]["from"]["notices"][0]["bar"] = json!(2);
    let err = Document::<Tune>::read(&doc.to_string()).unwrap_err();
    assert!(err.to_string().contains("`bar`"), "{err}");
}

#[test]
fn a_mismatched_schema_is_refused_naming_both() {
    let other_version = schema_url("tune", Version::new(1, 1));
    let other_kind = schema_url("part", Version::new(1, 2));
    for schema in [
        json!(other_version),
        json!(other_kind),
        json!("tune.json"),
        json!(12),
    ] {
        let text = tune_text("1.2", Some(schema.clone()), body());
        let err = Document::<Tune>::read(&text).unwrap_err();
        let Error::SchemaMismatch {
            found, expected, ..
        } = &err
        else {
            panic!("{err:?}")
        };
        assert_eq!(expected, &schema_url("tune", Version::new(1, 2)));
        assert_eq!(
            *found,
            schema
                .as_str()
                .map_or_else(|| schema.to_string(), str::to_string)
        );
        let msg = err.to_string();
        assert!(
            msg.contains(found.as_str()) && msg.contains("tune 1.2"),
            "{msg}"
        );
        // read_envelope checks it too.
        assert!(matches!(
            read_envelope(&text),
            Err(Error::SchemaMismatch { .. })
        ));
    }
}

#[test]
fn a_document_without_schema_reads_and_is_written_with_it() {
    let text = tune_text("1.2", None, body());
    let doc = Document::<Tune>::read(&text).unwrap();
    assert!(
        doc.write()
            .unwrap()
            .contains(&schema_url("tune", Version::new(1, 2)))
    );
}

#[test]
fn another_kind_is_refused_by_name() {
    let text = frames()
        .head_line()
        .unwrap()
        .replace("\"head\"", "\"body\"");
    let err = Document::<Tune>::read(&text).unwrap_err();
    assert!(
        matches!(&err, Error::WrongKind { found, expected: "tune" } if found == "frames"),
        "{err:?}"
    );
    assert!(err.to_string().contains("frames"), "{err}");
}

#[test]
fn a_malformed_frame_is_refused() {
    let no_body = json!({ "duet": { "kind": "tune", "version": "1.2", "tool": { "name": "d", "version": "0" } } });
    assert!(matches!(
        Document::<Tune>::read(&no_body.to_string()),
        Err(Error::Missing { field: "body", .. })
    ));
    assert!(matches!(
        Document::<Tune>::read(&json!({ "body": {} }).to_string()),
        Err(Error::Missing { field: "duet", .. })
    ));
    assert!(matches!(Document::<Tune>::read("[1]"), Err(Error::Json(_))));
    assert!(matches!(Document::<Tune>::read("{"), Err(Error::Json(_))));
    let bad_version = tune_text("1.2.0", None, body());
    let err = Document::<Tune>::read(&bad_version).unwrap_err();
    assert!(
        matches!(err, Error::Envelope(_)) && err.to_string().contains("1.2.0"),
        "{err}"
    );
}

// --- An older minor of the same major reads, and is written at the version this build speaks ---

#[test]
fn an_older_minor_reads_and_writes_back_at_the_current_version() {
    for version in ["1.0", "1.1"] {
        let text = tune_text(version, tune_schema(version), body());
        let doc = Document::<Tune>::read(&text).unwrap();
        assert_eq!(
            doc.body,
            Tune {
                title: "Reel".into(),
                tempo: 92.0,
                key: None
            }
        );
        let written: serde_json::Value = serde_json::from_str(&doc.write().unwrap()).unwrap();
        assert_eq!(written["duet"]["version"], json!("1.2"));
        assert_eq!(
            written["$schema"],
            json!(schema_url("tune", Version::new(1, 2)))
        );
        assert_eq!(written["body"], body(), "the body is unchanged");
    }
}

/// A kind whose 1.0 named a field differently, read by its own `read_minor`.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Renamed {
    bpm: f64,
}

impl Kind for Renamed {
    const NAME: &'static str = "renamed";
    const VERSION: Version = Version::new(3, 1);

    fn read_minor(minor: u16, mut body: serde_json::Value) -> serde_json::Result<Self> {
        if minor == 0
            && let Some(tempo) = body.as_object_mut().and_then(|b| b.remove("tempo"))
        {
            body["bpm"] = tempo;
        }
        serde_json::from_value(body)
    }
}

#[test]
fn an_older_minor_is_read_by_the_kinds_read_minor() {
    let text = json!({
        "duet": { "kind": "renamed", "version": "3.0", "tool": { "name": "duet", "version": "0" } },
        "body": { "tempo": 120.0 },
    });
    let doc = Document::<Renamed>::read(&text.to_string()).unwrap();
    assert_eq!(doc.body, Renamed { bpm: 120.0 });
    assert!(doc.write().unwrap().contains("\"version\": \"3.1\""));
}

// --- Exact floats ---

/// splitmix64: a small deterministic random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Numbers {
    wide: Vec<f64>,
    narrow: Vec<f32>,
}

impl Kind for Numbers {
    const NAME: &'static str = "numbers";
    const VERSION: Version = Version::new(1, 0);
}

#[test]
fn random_finite_floats_round_trip_to_the_same_bits() {
    let mut rng = Rng(0x5EED_D0E7);
    let mut wide = Vec::new();
    let mut narrow = Vec::new();
    while wide.len() < 20_000 {
        let bits = rng.next();
        let x = f64::from_bits(bits);
        if x.is_finite() {
            wide.push(x);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the low 32 bits are the pattern"
        )]
        let y = f32::from_bits(bits as u32);
        if y.is_finite() {
            narrow.push(y);
        }
    }
    for &x in &wide {
        let back: f64 = serde_json::from_str(&serde_json::to_string(&x).unwrap()).unwrap();
        assert_eq!(back.to_bits(), x.to_bits(), "{x:e}");
    }
    for &y in &narrow {
        let back: f32 = serde_json::from_str(&serde_json::to_string(&y).unwrap()).unwrap();
        assert_eq!(back.to_bits(), y.to_bits(), "{y:e}");
    }

    // And through a document, whose body is read by way of a `serde_json::Value`.
    let doc = Document::new(tool(), Numbers { wide, narrow });
    let back = Document::<Numbers>::read(&doc.write().unwrap()).unwrap();
    let bits = |n: &Numbers| {
        (
            n.wide.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            n.narrow.iter().map(|y| y.to_bits()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(bits(&back.body), bits(&doc.body));
}
