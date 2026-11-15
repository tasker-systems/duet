//! performance 1.0: a real take's performance, as duet-dsp makes it, written as a stream, read
//! back exactly, and validated line by line against the committed schema.

use std::path::Path;

use duet_dsp::{ANALYSIS_RATE, FrameBlock, Layer1, Mono, Performance, PerformanceHead, Stage};
use duet_schemas::{AnyDocument, Error, Stream, Tool, read_envelope};
use serde_json::{Value, json};

const RATE: u32 = ANALYSIS_RATE;

fn read_json(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The committed envelope and performance schemas, registered by their `$id`, and a validator
/// for `schema` against them.
fn validator(schema: &Value) -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let performance = read_json("schemas/performance/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .add(performance["$id"].as_str().unwrap(), &performance)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .unwrap()
}

fn head_validator() -> jsonschema::Validator {
    validator(&read_json("schemas/performance/1.0.json"))
}

fn record_validator() -> jsonschema::Validator {
    let id = read_json("schemas/performance/1.0.json")["$id"].clone();
    validator(&json!({ "$ref": format!("{}#/$defs/Record", id.as_str().unwrap()) }))
}

/// A sung-like take: silence, two tones with four partials, silence.
fn take() -> Vec<f32> {
    let tone = |hz: f64, secs: f64| -> Vec<f32> {
        let n = (secs * f64::from(RATE)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(RATE);
                let s: f64 = (1..=4)
                    .map(|h| (std::f64::consts::TAU * hz * f64::from(h) * t).sin() / f64::from(h))
                    .sum();
                (0.15 * s) as f32
            })
            .collect()
    };
    let silence = |secs: f64| vec![0.0; (secs * f64::from(RATE)) as usize];
    [
        silence(0.3),
        tone(442.0, 1.2),
        tone(330.0, 0.9),
        silence(0.4),
    ]
    .concat()
}

/// The take's performance, as a stream.
fn performance(signal: &[f32]) -> Stream<PerformanceHead> {
    let mut stage = Performance::new(RATE);
    let mut out = Vec::new();
    for (i, piece) in signal.chunks(512).enumerate() {
        let mono = Mono {
            rate: RATE,
            start_frame: (i * 512) as u64,
            samples: piece.to_vec(),
        };
        stage.push(mono, &mut out).unwrap();
    }
    stage.finish(&mut out).unwrap();
    let Some(Layer1::Head(head)) = out.first() else {
        panic!("no head first");
    };
    let mut stream = Stream::new(Tool::new("duet", "0.2.1"), *head);
    stream.records = out
        .into_iter()
        .filter_map(|o| match o {
            Layer1::Frames(block) => Some(block),
            _ => None,
        })
        .collect();
    stream
}

#[test]
fn a_performance_round_trips_exactly_through_its_reader() {
    let stream = performance(&take());
    assert!(stream.records.len() > 2);
    let pitches: Vec<Option<f32>> = stream
        .records
        .iter()
        .flat_map(|b| b.pitch.clone())
        .collect();
    assert!(pitches.iter().any(Option::is_none) && pitches.iter().any(Option::is_some));
    let text = stream.write().unwrap();
    let read = Stream::<PerformanceHead>::read(&text).unwrap();
    assert_eq!(read, stream);
    // Every float reads back to the same bits, so writing it again gives the same text.
    assert_eq!(read.write().unwrap(), text);
    // Written a line at a time, as capture writes it, it is the same file.
    let mut streamed = stream.head_line().unwrap();
    for record in &stream.records {
        streamed.push_str(&Stream::<PerformanceHead>::record_line(record).unwrap());
    }
    assert_eq!(streamed, text);
}

#[test]
fn the_first_line_is_the_envelope_and_every_later_line_validates_on_its_own() {
    let text = performance(&take()).write().unwrap();
    let mut lines = text.lines();
    let first = lines.next().unwrap();
    let envelope = read_envelope(first).unwrap();
    assert_eq!(envelope.kind, "performance");
    assert_eq!(envelope.version.to_string(), "1.0");
    let head = head_validator();
    let first: Value = serde_json::from_str(first).unwrap();
    assert!(head.is_valid(&first), "{first}");
    assert_eq!(first["head"]["unit"], "absolute-cents");
    let records = record_validator();
    let mut count = 0;
    for line in lines {
        let record: Value = serde_json::from_str(line).unwrap();
        let errors: Vec<String> = records
            .iter_errors(&record)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{line}: {errors:?}");
        // A record is not a head, and the head is not a record.
        assert!(!head.is_valid(&record));
        count += 1;
    }
    assert!(count > 2);
    assert!(!records.is_valid(&first));
}

#[test]
fn the_record_schema_refuses_what_the_reader_refuses() {
    let good =
        json!({"start": 0, "pitch": [null, 6900.0], "voicing": [0.1, 0.9], "level": [0.0, 0.2]});
    let records = record_validator();
    assert!(records.is_valid(&good));
    type Break = fn(&mut Value);
    let cases: [(&str, Break); 5] = [
        ("an unknown field", |r| r["hz"] = json!([440.0])),
        ("a missing column", |r| {
            r.as_object_mut().unwrap().remove("level");
        }),
        ("a pitch that is a name", |r| r["pitch"][1] = json!("A4")),
        ("a voicing above 1", |r| r["voicing"][0] = json!(1.5)),
        ("a negative level", |r| r["level"][0] = json!(-0.1)),
    ];
    for (what, break_it) in cases {
        let mut record = good.clone();
        break_it(&mut record);
        assert!(!records.is_valid(&record), "the schema accepts {what}");
        let mut text = performance(&take()).head_line().unwrap();
        text.push_str(&record.to_string());
        text.push('\n');
        assert!(
            Stream::<PerformanceHead>::read(&text).is_err(),
            "the reader accepts {what}"
        );
    }
}

#[test]
fn columns_of_different_lengths_are_refused_naming_the_line() {
    let mut text = performance(&take()).head_line().unwrap();
    let block = FrameBlock::new(0, vec![Some(6900.0)], vec![0.9], vec![0.2]).unwrap();
    text.push_str(&Stream::<PerformanceHead>::record_line(&block).unwrap());
    text.push_str(
        "{\"start\":256,\"pitch\":[6900.0,null],\"voicing\":[0.9],\"level\":[0.2,0.1]}\n",
    );
    let err = Stream::<PerformanceHead>::read(&text).unwrap_err();
    assert!(matches!(err, Error::Line { line: 3, .. }), "{err:?}");
    let message = format!("{err}: {}", std::error::Error::source(&err).unwrap());
    assert!(message.contains("pitch 2, voicing 1, level 2"), "{message}");
}

#[test]
fn a_performance_is_not_read_as_a_document() {
    let text = performance(&take()).write().unwrap();
    let first = text.lines().next().unwrap();
    let err = AnyDocument::read(first).unwrap_err();
    assert!(matches!(err, Error::Framing { .. }), "{err:?}");
}
