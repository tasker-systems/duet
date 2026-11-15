//! capture-analysis: a real take's analysis, as `duet capture` makes it, its reading and
//! residual included, written as a document, read back exactly, and validated against the
//! committed schema at the current version.

use std::path::Path;

use duet_capture::{
    ANALYSIS_RATE, Analysis, Event, Given, Kind, Layer1, Mono, Performance, PerformanceFrames,
    Stage, assemble, estimate, quantize_apart, quantize_offsets, read,
};
use duet_schemas::{AnyDocument, Document, Tool};
use serde_json::{Value, json};

const RATE: u32 = ANALYSIS_RATE;

fn read_json(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A validator for the committed capture-analysis 1.3 schema, with the envelope registered.
fn validator() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read_json("schemas/capture-analysis/1.3.json"))
        .unwrap()
}

fn assert_valid(document: &Value) {
    let validator = validator();
    let errors: Vec<String> = validator
        .iter_errors(document)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// An E-flat major scale at 120 a minute from half a second in, a quarter note each.
fn take() -> Vec<f32> {
    let mut out = vec![0.0; RATE as usize / 2];
    for midi in [
        63, 65, 67, 68, 70, 72, 74, 75, 74, 72, 70, 68, 67, 65, 63, 63,
    ] {
        let hz = 440.0 * ((f64::from(midi) - 69.0) / 12.0).exp2();
        let len = RATE as usize / 2;
        for i in 0..len {
            let t = i as f64 / f64::from(RATE);
            let tone: f64 = (1..=4)
                .map(|h| (std::f64::consts::TAU * hz * f64::from(h) * t).sin() / f64::from(h))
                .sum();
            let envelope = (i as f64 / 100.0).min((len - i) as f64 / 400.0).min(1.0);
            out.push((0.15 * tone * envelope) as f32);
        }
    }
    out.extend(vec![0.0; RATE as usize / 2]);
    out
}

/// The take's analysis as capture makes it, with `given` timing, quantized to sixteenths.
fn analysis(given: &Given) -> Analysis {
    let mut performance = Performance::new(RATE);
    let mut layers = Vec::new();
    for (i, piece) in take().chunks(4096).enumerate() {
        let mono = Mono {
            rate: RATE,
            start_frame: (i * 4096) as u64,
            samples: piece.to_vec(),
        };
        performance.push(mono, &mut layers).unwrap();
    }
    performance.finish(&mut layers).unwrap();
    let (mut frames, mut events) = (None, Vec::new());
    for layer in layers {
        match layer {
            Layer1::Head(head) => frames = Some(PerformanceFrames::new(&head)),
            Layer1::Frames(block) => frames.as_mut().unwrap().push(&block).unwrap(),
            Layer1::Onset { frame, strength } => events.push(Event::Onset { frame, strength }),
            _ => {}
        }
    }
    let reading = read(&frames.unwrap(), &events, None);
    events.extend(reading.notes.iter().map(|n| n.event()));
    events.sort_by_key(|e| match *e {
        Event::Onset { frame, .. } => frame,
        Event::Note { start, .. } => start,
        _ => u64::MAX,
    });
    let timing = estimate(&events, RATE, given).unwrap();
    let mut captured = assemble(&events, RATE, &timing, None, &Kind::Melody).unwrap();
    let ms_per_quarter = 1000.0 * timing.beat_period_frames / f64::from(RATE);
    let q = quantize_apart(&captured.song, "1/16".parse().unwrap(), &Kind::Melody).unwrap();
    captured.analysis.offsets =
        quantize_offsets(&captured.song, &q.song, q.grid, ms_per_quarter).unwrap();
    captured.analysis.grid = Some(q.grid);
    captured.analysis.reading = Some(reading);
    captured.analysis
}

fn round_trip(analysis: Analysis) -> (String, Value) {
    let doc = Document::new(Tool::new("duet", "0.2.1"), analysis);
    let text = doc.write().unwrap();
    let read = Document::<Analysis>::read(&text).unwrap();
    assert_eq!(read, doc);
    // Every float reads back to the same bits, so writing it again gives the same text.
    assert_eq!(read.write().unwrap(), text);
    let AnyDocument::CaptureAnalysis(any) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(*any, doc);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_valid(&value);
    assert!(
        !text.contains("null"),
        "an absent value is written null:\n{text}"
    );
    (text, value)
}

#[test]
fn an_estimated_analysis_round_trips_exactly_and_validates() {
    let analysis = analysis(&Given::default());
    assert!(analysis.key.is_some() && !analysis.offsets.is_empty());
    let (_, value) = round_trip(analysis);
    let body = &value["body"];
    assert_eq!(body["rate"], 22_050);
    assert!(
        body["timing"]["tempo_bpm"]["estimated"].is_object(),
        "{body}"
    );
    let alternatives = &body["timing"]["tempo_alternatives"];
    let bpm = body["timing"]["tempo_bpm"]["estimated"]["value"]
        .as_f64()
        .unwrap();
    assert_eq!(
        alternatives,
        &json!({ "half": bpm / 2.0, "double": bpm * 2.0 })
    );
    assert_eq!(body["grid"], "1/16");
    // The reading's residual: every note's parts, and the loss as its three terms.
    let reading = &body["reading"];
    assert!(
        reading["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["residual"].is_object())
    );
    for term in ["pitch", "unread", "notes"] {
        assert!(
            reading["residual"]["loss"][term].is_number(),
            "{term}: {reading}"
        );
    }
    // The reader, since 1.3: the rules, with the loss's cost per note.
    assert_eq!(
        reading["reader"],
        json!({ "name": "rules", "note_cost": 4.0 })
    );
    assert!(body["key"]["key"]["estimated"].is_object(), "{body}");
}

#[test]
fn a_given_tempo_has_no_alternatives_and_an_absent_key_and_grid_are_left_out() {
    let given = Given {
        tempo_bpm: Some(120.0),
        ..Given::default()
    };
    let mut analysis = analysis(&given);
    let (_, value) = round_trip(analysis.clone());
    assert_eq!(
        value["body"]["timing"]["tempo_bpm"],
        json!({ "given": 120.0 })
    );
    assert!(value["body"]["timing"].get("tempo_alternatives").is_none());
    // As a rhythm take has no key, and a score that could not be quantized has no grid.
    analysis.key = None;
    analysis.grid = None;
    analysis.offsets.clear();
    let (_, value) = round_trip(analysis);
    assert!(value["body"].get("key").is_none() && value["body"].get("grid").is_none());
}

#[test]
fn the_schema_and_the_reader_refuse_what_capture_never_writes() {
    let text = Document::new(Tool::new("duet", "0.2.1"), analysis(&Given::default()))
        .write()
        .unwrap();
    let good: Value = serde_json::from_str(&text).unwrap();
    let validator = validator();
    type Break = fn(&mut Value);
    let cases: [(&str, Break); 7] = [
        ("a null key", |d| d["body"]["key"] = Value::Null),
        ("a null grid", |d| d["body"]["grid"] = Value::Null),
        ("positional tempo alternatives", |d| {
            d["body"]["timing"]["tempo_alternatives"] = json!([60.0, 240.0])
        }),
        ("an unknown field on an event", |d| {
            d["body"]["events"][0]["lane"] = json!("kick")
        }),
        ("an unknown event", |d| {
            d["body"]["events"][0] = json!({ "type": "breath", "frame": 1 })
        }),
        ("an unknown analysis field", |d| {
            d["body"]["reading"] = json!({})
        }),
        ("a confidence above 1", |d| {
            d["body"]["timing"]["tempo_bpm"]["estimated"]["confidence"] = json!(1.5)
        }),
    ];
    for (what, break_it) in cases {
        let mut doc = good.clone();
        break_it(&mut doc);
        assert!(!validator.is_valid(&doc), "the schema accepts {what}");
        // The schema is the stricter of the two elsewhere: serde reads a null optional field as
        // absent and a struct from an array of its fields in order, and checks no range.
        if what.contains("unknown") {
            assert!(
                AnyDocument::read(&doc.to_string()).is_err(),
                "the reader accepts {what}"
            );
        }
    }
}
