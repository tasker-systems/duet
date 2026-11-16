//! capture-analysis: a real take's analysis, as `duet capture` makes it, its reading and
//! residual included, written as a document, read back exactly, and validated against the
//! committed schema at the current version.

use std::path::Path;

use duet_capture::{
    ANALYSIS_RATE, Analysis, Event, Given, Kind, Layer1, Mono, NoteModel, Performance,
    PerformanceFrames, Reader as ReadingReader, Stage, Thresholds, assemble, estimate,
    quantize_apart, quantize_offsets, read,
};
use duet_schemas::{AnyDocument, Document, Tool};
use serde_json::{Value, json};

const RATE: u32 = ANALYSIS_RATE;

fn read_json(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A validator for the committed capture-analysis 1.10 schema, with the envelope registered.
fn validator() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read_json("schemas/capture-analysis/1.10.json"))
        .unwrap()
}

/// A validator for the committed capture-analysis schema at `version`, an older one.
fn validator_at(version: &str) -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read_json(&format!(
            "schemas/capture-analysis/{version}.json"
        )))
        .unwrap()
}

/// `document`, restamped as a capture-analysis document at `version`.
fn as_version(document: &Value, version: &str) -> Value {
    let mut older = document.clone();
    older["$schema"] = json!(format!(
        "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/capture-analysis/{version}.json"
    ));
    older["duet"]["version"] = json!(version);
    older
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
    sung(|_, _| 0.0)
}

/// The scale, its fourth note (A-flat) turning up a whole tone and back over its first 0.14 s
/// (0.04 s up, held 0.06 s, 0.04 s down): the shape of the turn the note HMM takes in as attack
/// (`duet-dsp`'s `a_turn_at_a_notes_start_is_one_note_with_one_excursion`).
fn take_with_a_turn() -> Vec<f32> {
    let (ramp, held) = (0.04, 0.06);
    let up = move |x: f64| 200.0 * (std::f64::consts::FRAC_PI_2 * x / ramp).sin();
    sung(move |note, t| match t {
        _ if note != 3 => 0.0,
        t if t < ramp => up(t),
        t if t <= ramp + held => 200.0,
        t if t < 2.0 * ramp + held => up(2.0 * ramp + held - t),
        _ => 0.0,
    })
}

/// The scale, its fifth note (B-flat) dipping 170 cents and back over 0.1 s from 0.2 s in: the
/// shape of the dip the same-key rejoin reads as one note (`duet-dsp`'s
/// `a_dip_of_170_cents_with_no_onset_is_rejoined_into_one_note`).
fn take_with_a_dip() -> Vec<f32> {
    sung(|note, t| {
        let into = (t - 0.2) / 0.05;
        if note == 4 && (0.0..2.0).contains(&into) {
            -170.0 * (1.0 - (into - 1.0).abs())
        } else {
            0.0
        }
    })
}

/// The scale, each note's pitch `cents(note, t)` from its key at `t` seconds into it.
fn sung(cents: impl Fn(usize, f64) -> f64) -> Vec<f32> {
    let mut out = vec![0.0; RATE as usize / 2];
    for (note, midi) in [
        63, 65, 67, 68, 70, 72, 74, 75, 74, 72, 70, 68, 67, 65, 63, 63,
    ]
    .into_iter()
    .enumerate()
    {
        let len = RATE as usize / 2;
        let mut phase = 0.0;
        for i in 0..len {
            let t = i as f64 / f64::from(RATE);
            let hz = 440.0 * ((f64::from(midi) - 69.0 + cents(note, t) / 100.0) / 12.0).exp2();
            phase += std::f64::consts::TAU * hz / f64::from(RATE);
            let tone: f64 = (1..=4)
                .map(|h| (phase * f64::from(h)).sin() / f64::from(h))
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
    analysis_of(&take(), given, Reader::Rules)
}

/// Which reader reads a take's notes.
#[derive(Clone, Copy)]
enum Reader {
    Rules,
    Hmm,
}

/// `take`'s analysis as capture makes it, read by `reader` with `given` timing, quantized to
/// sixteenths.
fn analysis_of(take: &[f32], given: &Given, reader: Reader) -> Analysis {
    let mut performance = Performance::new(RATE);
    let mut layers = Vec::new();
    for (i, piece) in take.chunks(4096).enumerate() {
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
    let frames = frames.unwrap();
    let reading = match reader {
        Reader::Rules => read(&frames, &events, None),
        Reader::Hmm => NoteModel::default().read(&frames, &events, None),
    };
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
    for term in ["pitch", "voicing", "unread", "notes"] {
        assert!(
            reading["residual"]["loss"][term].is_number(),
            "{term}: {reading}"
        );
    }
    // A scale sung plain states no ornament, so no note writes `excursions` (1.6).
    assert!(
        reading["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["residual"].get("excursions").is_none()),
        "{reading}"
    );
    // The reader, since 1.3: the rules, with the loss's cost per note.
    assert_eq!(
        reading["reader"],
        json!({ "name": "rules", "note_cost": 4.0 })
    );
    assert!(body["key"]["key"]["estimated"].is_object(), "{body}");
}

#[test]
fn a_notes_excursion_round_trips_exactly_and_validates() {
    // The HMM takes the turn into the A-flat, and it is the note's one excursion (1.6).
    let analysis = analysis_of(&take_with_a_turn(), &Given::default(), Reader::Hmm);
    let (_, value) = round_trip(analysis);
    let notes = value["body"]["reading"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 16, "the turn is no note of its own");
    let with: Vec<(u64, &Value)> = notes
        .iter()
        .filter_map(|n| Some((n["key"].as_u64()?, n["residual"].get("excursions")?)))
        .collect();
    assert_eq!(with.len(), 1, "{with:?}");
    let (key, excursions) = with[0];
    assert_eq!(key, 68);
    let excursions = excursions.as_array().unwrap();
    assert_eq!(excursions.len(), 1, "{excursions:?}");
    let cents = excursions[0]["cents"].as_f64().unwrap();
    assert!((cents - 200.0).abs() <= 10.0, "{excursions:?}");
}

#[test]
fn a_readings_rejoin_round_trips_exactly_and_validates() {
    // The HMM splits the B-flat at its dip, and the pass rejoins it, stating so (1.7).
    let analysis = analysis_of(&take_with_a_dip(), &Given::default(), Reader::Hmm);
    let (_, value) = round_trip(analysis);
    let reading = &value["body"]["reading"];
    assert_eq!(reading["notes"].as_array().unwrap().len(), 16, "{reading}");
    let rejoins = reading["rejoins"].as_array().expect("a rejoin is stated");
    assert_eq!(rejoins.len(), 1, "{rejoins:?}");
    let decoded = rejoins[0]["decoded"].as_array().unwrap();
    assert_eq!(decoded.len(), 2, "{decoded:?}");
    assert!(
        decoded
            .iter()
            .all(|n| n["key"] == 70 && n.get("residual").is_none())
    );
    assert!(rejoins[0]["loss_change"].is_f64(), "{rejoins:?}");
    // A scale sung plain has nothing to rejoin, and writes no `rejoins`.
    let (_, plain) = round_trip(analysis_of(&take(), &Given::default(), Reader::Hmm));
    assert!(plain["body"]["reading"].get("rejoins").is_none());
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

#[test]
fn a_basic_pitch_reading_names_its_reader_and_validates_only_from_1_8() {
    let mut analysis = analysis(&Given::default());
    let reading = analysis.reading.as_mut().unwrap();
    // As 1.8 wrote it: a basic-pitch reader, with no thresholds.
    let mut reader = ReadingReader::basic_pitch(Thresholds::BASIC_PITCH);
    reader.thresholds = None;
    reading.reader = Some(reader);
    let (_, value) = round_trip(analysis);
    assert_eq!(
        value["body"]["reading"]["reader"]["name"],
        json!("basic-pitch")
    );
    // capture-analysis 1.7 has no basic-pitch reader: a 1.7 reader refuses the name.
    let envelope = read_json("schemas/envelope/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    let older = jsonschema::options()
        .with_registry(&registry)
        .build(&read_json("schemas/capture-analysis/1.7.json"))
        .unwrap();
    let mut as_1_7 = value.clone();
    as_1_7["$schema"] = json!(
        "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/capture-analysis/1.7.json"
    );
    as_1_7["duet"]["version"] = json!("1.7");
    assert!(!older.is_valid(&as_1_7), "1.7 accepts basic-pitch");
    // The same document read by the rules is a valid 1.7 one: only the reader's name differs.
    as_1_7["body"]["reading"]["reader"]["name"] = json!("rules");
    assert!(older.is_valid(&as_1_7), "the 1.7 control is not valid");
}

#[test]
fn a_basic_pitch_reading_records_its_thresholds_and_validates_only_from_1_10() {
    let mut analysis = analysis(&Given::default());
    let reading = analysis.reading.as_mut().unwrap();
    let thresholds = Thresholds {
        onset_threshold: 0.6,
        frame_threshold: 0.25,
        minimum_note_length_ms: 200.0,
    };
    reading.reader = Some(ReadingReader::basic_pitch(thresholds));
    let (_, value) = round_trip(analysis);
    assert_eq!(
        value["body"]["reading"]["reader"]["thresholds"],
        json!({
            "onset_threshold": 0.6,
            "frame_threshold": 0.25,
            "minimum_note_length_ms": 200.0,
        })
    );
    // capture-analysis 1.9 has no thresholds: a 1.9 reader refuses them.
    let older = validator_at("1.9");
    let mut as_1_9 = as_version(&value, "1.9");
    assert!(!older.is_valid(&as_1_9), "1.9 accepts thresholds");
    // The same document without them is a valid 1.9 one: only the thresholds differ.
    as_1_9["body"]["reading"]["reader"]
        .as_object_mut()
        .unwrap()
        .remove("thresholds");
    assert!(older.is_valid(&as_1_9), "the 1.9 control is not valid");
}

/// A validator for the committed capture-analysis 1.8 schema, the last before `breathy`.
fn validator_1_8() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read_json("schemas/capture-analysis/1.8.json"))
        .unwrap()
}

#[test]
fn a_readings_breathy_notes_round_trip_exactly_and_validate_only_from_1_9() {
    // A tone's last frames, as it dies away, are weakly voiced: the HMM leaves them unread, and
    // the pass carries the note on over those whose pitch goes on, stating so (1.9).
    let analysis = analysis_of(&take(), &Given::default(), Reader::Hmm);
    let (_, value) = round_trip(analysis);
    let reading = &value["body"]["reading"];
    let breathy = reading["breathy"]
        .as_array()
        .expect("a breathy note is stated");
    assert!(!breathy.is_empty());
    for b in breathy {
        assert!(b["decoded"].get("residual").is_none(), "{b}");
        assert!(b["end"].as_u64() > b["decoded"]["end"].as_u64(), "{b}");
        assert!(b["lowest_db"].as_f64().unwrap() >= -20.0, "{b}");
    }
    // capture-analysis 1.8 has no `breathy`: a 1.8 reader refuses it, and takes the same
    // document without it.
    let mut as_1_8 = value.clone();
    as_1_8["$schema"] = json!(
        "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/capture-analysis/1.8.json"
    );
    as_1_8["duet"]["version"] = json!("1.8");
    let older = validator_1_8();
    assert!(!older.is_valid(&as_1_8), "1.8 accepts breathy");
    as_1_8["body"]["reading"]
        .as_object_mut()
        .unwrap()
        .remove("breathy");
    assert!(older.is_valid(&as_1_8), "the 1.8 control is not valid");
}
