//! Round-trip tests on generated MIDI files: MIDI to part, through the text, JSON and
//! TOML forms, and back to MIDI, checking every kept event survives to the tick.

use std::collections::{HashMap, VecDeque};

use duet_core::{Grid, Part, Position};
use duet_midi::Midi;
use duet_schemas::journal::Id;
use duet_schemas::{AnyDocument, Document, PartBody, Source, Tool};
use midly::num::{u4, u7, u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};

/// A note as absolute ticks: (channel, key, start, end, velocity).
type RawNote = (u8, u8, u64, u64, u8);

#[derive(Debug, PartialEq, Default)]
struct Events {
    notes: Vec<RawNote>,
    /// (channel, tick, description), in file order per channel.
    controls: Vec<(u8, u64, String)>,
    tempos: Vec<(u64, u32)>,
    sigs: Vec<(u64, u8, u8)>,
    keys: Vec<(u64, i8, bool)>,
}

fn last_per_tick<T: PartialEq + Clone>(mut v: Vec<(u64, T)>) -> Vec<(u64, T)> {
    v.sort_by_key(|(t, _)| *t);
    let mut out: Vec<(u64, T)> = Vec::new();
    for (t, x) in v {
        match out.last_mut() {
            Some(last) if last.0 == t => last.1 = x,
            _ => out.push((t, x)),
        }
    }
    out
}

/// Every event notefmt promises to keep, read straight from the file.
fn events(bytes: &[u8]) -> Events {
    let smf = Smf::parse(bytes).unwrap();
    let mut ev = Events::default();
    let mut tempos = Vec::new();
    let mut sigs = Vec::new();
    let mut keys = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        let mut open: HashMap<(u8, u8), VecDeque<(u64, u8)>> = HashMap::new();
        for e in track {
            tick += u64::from(e.delta.as_int());
            match e.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => open
                            .entry((ch, key.as_int()))
                            .or_default()
                            .push_back((tick, vel.as_int())),
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some((start, vel)) = open
                                .get_mut(&(ch, key.as_int()))
                                .and_then(|q| q.pop_front())
                            {
                                ev.notes.push((ch, key.as_int(), start, tick, vel));
                            }
                        }
                        MidiMessage::Controller { controller, value } => {
                            ev.controls
                                .push((ch, tick, format!("cc{controller}={value}")))
                        }
                        MidiMessage::PitchBend { bend } => {
                            ev.controls
                                .push((ch, tick, format!("bend={}", bend.as_int())))
                        }
                        MidiMessage::ProgramChange { program } => {
                            ev.controls.push((ch, tick, format!("prog={program}")))
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempos.push((tick, t.as_int())),
                TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, _, _)) => {
                    sigs.push((tick, (n, d)))
                }
                TrackEventKind::Meta(MetaMessage::KeySignature(s, m)) => keys.push((tick, (s, m))),
                _ => {}
            }
        }
        for ((ch, key), starts) in open {
            for (start, vel) in starts {
                ev.notes.push((ch, key, start, tick, vel));
            }
        }
    }
    ev.notes.sort();
    // Order of controls matters only within a channel and tick.
    ev.controls.sort_by_key(|(ch, tick, _)| (*ch, *tick));
    ev.tempos = last_per_tick(tempos);
    ev.sigs = last_per_tick(sigs)
        .into_iter()
        .map(|(t, (n, d))| (t, n, d))
        .collect();
    ev.keys = last_per_tick(keys)
        .into_iter()
        .map(|(t, (s, m))| (t, s, m))
        .collect();
    ev
}

/// Builds track events from (tick, event) pairs in any order (stable by tick).
fn track(mut timed: Vec<(u64, TrackEventKind<'static>)>) -> Vec<TrackEvent<'static>> {
    timed.sort_by_key(|(t, _)| *t);
    let mut last = 0;
    let mut out: Vec<TrackEvent> = timed
        .into_iter()
        .map(|(t, kind)| {
            let e = TrackEvent {
                delta: u28::new((t - last) as u32),
                kind,
            };
            last = t;
            e
        })
        .collect();
    out.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}

fn midi(ch: u8, message: MidiMessage) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(ch),
        message,
    }
}

fn on(ch: u8, key: u8, vel: u8) -> TrackEventKind<'static> {
    midi(
        ch,
        MidiMessage::NoteOn {
            key: u7::new(key),
            vel: u7::new(vel),
        },
    )
}

/// Note-offs alternate between real note-offs and note-ons at velocity 0, as real files do.
fn off(ch: u8, key: u8, style: bool) -> TrackEventKind<'static> {
    if style {
        midi(
            ch,
            MidiMessage::NoteOff {
                key: u7::new(key),
                vel: u7::new(64),
            },
        )
    } else {
        on(ch, key, 0)
    }
}

fn meta(m: MetaMessage<'static>) -> TrackEventKind<'static> {
    TrackEventKind::Meta(m)
}

fn save(format: Format, tpq: u16, tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
    let mut smf = Smf::new(Header::new(format, Timing::Metrical(u15::new(tpq))));
    smf.tracks = tracks;
    let mut out = Vec::new();
    smf.write_std(&mut out).unwrap();
    out
}

/// A small deterministic random source, for played-in timing jitter.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// A "played" take at the given resolution: humanized timing, sustain pedal, bends,
/// tempo and meter changes (one mid-bar), overlapping repeats of a pitch, a zero-length
/// note, a note still held at the end, and a drum part sharing a track with a bass.
fn played_take(tpq: u16, format: Format) -> (Vec<u8>, Vec<RawNote>) {
    let q = u64::from(tpq);
    let mut rng = Lcg(u64::from(tpq));
    let mut conductor = vec![
        (0, meta(MetaMessage::TrackName(b"Sketch in D"))),
        (0, meta(MetaMessage::TimeSignature(4, 2, 24, 8))),
        (0, meta(MetaMessage::KeySignature(-1, true))),
        (0, meta(MetaMessage::Tempo(u24::new(652_174)))), // 92 bpm
        // Bars 1-4 in 4/4, then 6/8 from bar 5.
        (16 * q, meta(MetaMessage::TimeSignature(6, 3, 24, 8))),
        (16 * q + 3 * q, meta(MetaMessage::Tempo(u24::new(600_000)))), // 100 bpm
        // 3/4 starting halfway through a 6/8 bar, cutting it short.
        (
            16 * q + 3 * q + 3 * q / 2,
            meta(MetaMessage::TimeSignature(3, 2, 24, 8)),
        ),
        (24 * q, meta(MetaMessage::KeySignature(0, true))),
        (25 * q + 7, meta(MetaMessage::Tempo(u24::new(612_345)))),
    ];
    let mut piano = vec![
        (0, meta(MetaMessage::TrackName(b"Piano"))),
        (
            0,
            midi(
                0,
                MidiMessage::ProgramChange {
                    program: u7::new(0),
                },
            ),
        ),
        (
            0,
            midi(
                0,
                MidiMessage::Controller {
                    controller: u7::new(7),
                    value: u7::new(100),
                },
            ),
        ),
    ];
    let mut expected = Vec::new();
    let mut add = |events: &mut Vec<(u64, TrackEventKind<'static>)>,
                   ch: u8,
                   key: u8,
                   start: u64,
                   len: u64,
                   vel: u8| {
        events.push((start, on(ch, key, vel)));
        events.push((
            start + len,
            off(ch, key, (start + key as u64).is_multiple_of(2)),
        ));
        expected.push((ch, key, start, start + len, vel));
    };

    // Humanized eighths and triplets across 28 quarters.
    let scale = [62u8, 64, 65, 67, 69, 70, 72, 74];
    let mut t = 0u64;
    let mut i = 0usize;
    while t < 28 * q {
        let step = if (t / (4 * q)).is_multiple_of(2) {
            q / 2
        } else {
            q / 3
        };
        let jitter = rng.next(15);
        let len = step - 3 + rng.next(30);
        let vel = 50 + rng.next(70) as u8;
        add(&mut piano, 0, scale[i % scale.len()], t + jitter, len, vel);
        t += step;
        i += 1;
    }
    // Overlapping repeats of one pitch, a zero-length note, a chord.
    add(&mut piano, 0, 50, 2 * q, 3 * q, 90);
    add(&mut piano, 0, 50, 3 * q, 3 * q, 70);
    add(&mut piano, 0, 81, 5 * q + 11, 0, 30);
    for key in [57u8, 60, 76] {
        add(&mut piano, 0, key, 12 * q, 4 * q, 80);
    }
    // Sustain pedal and a pitch bend sweep.
    for bar in 0..6u64 {
        piano.push((
            bar * 4 * q + 5,
            midi(
                0,
                MidiMessage::Controller {
                    controller: u7::new(64),
                    value: u7::new(127),
                },
            ),
        ));
        piano.push((
            bar * 4 * q + 4 * q - 20,
            midi(
                0,
                MidiMessage::Controller {
                    controller: u7::new(64),
                    value: u7::new(0),
                },
            ),
        ));
    }
    for (n, bend) in [-8192i16, -4000, 0, 2048, 8191, 0].iter().enumerate() {
        piano.push((
            8 * q + n as u64 * 37,
            midi(
                0,
                MidiMessage::PitchBend {
                    bend: PitchBend::from_int(*bend),
                },
            ),
        ));
    }
    // A later program change, and a note held past the end of the track.
    piano.push((
        20 * q,
        midi(
            0,
            MidiMessage::ProgramChange {
                program: u7::new(4),
            },
        ),
    ));
    piano.push((27 * q, on(0, 38, 64)));
    piano.push((29 * q + 13, meta(MetaMessage::Text(b"dropped"))));

    // Drums on channel 10 and bass on channel 2, in one MIDI track.
    let mut rhythm = vec![(0, meta(MetaMessage::TrackName(b"Rhythm")))];
    rhythm.push((
        0,
        midi(
            1,
            MidiMessage::ProgramChange {
                program: u7::new(33),
            },
        ),
    ));
    for beat in 0..16u64 {
        add(
            &mut rhythm,
            9,
            if beat % 2 == 0 { 36 } else { 38 },
            beat * q + rng.next(9),
            q / 8,
            100,
        );
        if beat % 4 == 0 {
            add(&mut rhythm, 1, 38, beat * q, 2 * q - 1, 95);
        }
    }
    expected.push((0, 38, 27 * q, 29 * q + 13, 64));
    expected.sort();

    let bytes = match format {
        Format::SingleTrack => {
            let mut all = conductor;
            all.append(&mut piano);
            all.append(&mut rhythm);
            save(format, tpq, vec![track(all)])
        }
        _ => {
            conductor.sort_by_key(|(t, _)| *t);
            save(
                format,
                tpq,
                vec![track(conductor), track(piano), track(rhythm)],
            )
        }
    };
    (bytes, expected)
}

/// The part as every writer gives it back: with its end stated.
fn stated(part: &Part) -> Part {
    let mut part = part.clone();
    part.end = Some(part.stated_end().unwrap());
    part
}

/// A part's id, as `duet` mints one.
fn id() -> Id {
    Id::from_parts(1_795_446_245_000, [7; 10])
}

/// One source of each kind a part states (decision 01a1ee66): by hand, the reading of one take,
/// and generated by an operation from a document.
fn sources() -> Vec<Source> {
    let digest = format!("sha256:{}", "9f2c".repeat(16)).parse().unwrap();
    let generated = serde_json::from_value(serde_json::json!({
        "by": "generated", "operation": "part quantize", "inputs": ["00ff00ff00ff00ff"]
    }))
    .unwrap();
    vec![
        Source::hand(),
        Source::reading(digest, Some("basic-pitch".into())),
        generated,
    ]
}

/// The part through a `part` document and back, exactly, with each source: the body reads back
/// equal, its id and source with it, the text writes back identical, and the body is its `id`
/// and `source`, then byte for byte what the `.json` writer wrote before documents
/// (`serde_json::to_string_pretty`), one level in.
fn check_document(part: &Part) {
    for source in sources() {
        check_document_with(part, source);
    }
}

fn check_document_with(part: &Part, source: Source) {
    let body = PartBody::new(id(), source.clone(), part.clone());
    let text = Document::new(Tool::new("duet", "test"), body.clone())
        .write()
        .unwrap();
    let back = Document::<PartBody>::read(&text).unwrap();
    assert_eq!(back.body, body, "part document round-trip; it was:\n{text}");
    assert_eq!(back.body.id, id(), "the id survives");
    assert_eq!(back.body.source, source, "the source survives");
    assert_eq!(
        back.write().unwrap(),
        text,
        "the document writes back identical"
    );
    let AnyDocument::Part(any) = AnyDocument::read(&text).unwrap() else {
        panic!("not read as a part");
    };
    assert_eq!(any.body, body, "read by its envelope");

    // The notes view of the document's body reads back to the part: the lossless view is the
    // identity. The view always states the end, as every writer does.
    let view = back.body.part.to_text();
    let mut want = part.clone();
    want.end = Some(want.stated_end().unwrap());
    assert_eq!(
        Part::from_text(&view).unwrap(),
        want,
        "the notes view; it was:\n{view}"
    );
    assert_eq!(
        Part::from_text(&view).unwrap().to_text(),
        view,
        "the view is stable"
    );

    // The body is the id, the source, then the part's own fields as the `.json` writer wrote
    // them.
    let json = serde_json::to_string_pretty(part).unwrap();
    let source = serde_json::to_string_pretty(&source)
        .unwrap()
        .replace('\n', "\n    ");
    let body = format!(
        "  \"body\": {{\n    \"id\": \"{}\",\n    \"source\": {source},\n{}\n}}\n",
        id(),
        json.replace('\n', "\n  ").strip_prefix("{\n").unwrap()
    );
    assert!(
        text.ends_with(&body),
        "the body is not its id, source and the .json writer's bytes:\n{text}\nwanted it to \
         end:\n{body}"
    );
}

/// A part document states its id and its source: one with either missing, or with a source of
/// no kind, is refused by name, never read with a default.
#[test]
fn a_part_document_without_its_id_or_source_is_refused_by_name() {
    let (bytes, _) = played_take(480, Format::Parallel);
    let part = stated(&Part::from_midi(&bytes).unwrap());
    let text = Document::new(
        Tool::new("duet", "test"),
        PartBody::new(id(), Source::hand(), part),
    )
    .write()
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    for (field, says) in [
        ("id", "missing field `id`"),
        ("source", "missing field `source`"),
    ] {
        let mut broken = doc.clone();
        broken["body"].as_object_mut().unwrap().remove(field);
        let err = Document::<PartBody>::read(&broken.to_string()).unwrap_err();
        assert!(err.to_string().contains(says), "{field}: {err}");
    }
    let mut broken = doc.clone();
    broken["body"]["source"] = serde_json::json!({ "by": "magic" });
    let err = Document::<PartBody>::read(&broken.to_string()).unwrap_err();
    assert!(err.to_string().contains("magic"), "{err}");
    let mut broken = doc;
    broken["body"]["source"] = serde_json::json!({ "by": "comped", "takes": [] });
    let err = Document::<PartBody>::read(&broken.to_string()).unwrap_err();
    assert!(err.to_string().contains("at least one"), "{err}");
}

fn check_round_trip(bytes: &[u8], expected_notes: &[RawNote]) {
    let original = events(bytes);
    assert_eq!(
        original.notes, expected_notes,
        "test file is built as intended"
    );

    // The generated file's tracks each end at their last event, so it names no end; every
    // form written from it states the default.
    let read = Part::from_midi(bytes).unwrap();
    assert_eq!(read.end, None);
    let part = stated(&read);

    // Text, JSON and TOML all give back the same part.
    let text = read.to_text();
    let from_text = Part::from_text(&text).unwrap();
    assert_eq!(from_text, part, "text round-trip; text was:\n{text}");
    assert_eq!(from_text.to_text(), text, "text is stable");
    check_document(&part);
    check_document(&read);

    // Writing MIDI keeps every note, control and meta event to the tick.
    let written = from_text.to_midi().unwrap();
    let rewritten = events(&written);
    assert_eq!(rewritten.notes, original.notes);
    assert_eq!(rewritten.controls, original.controls);
    assert_eq!(rewritten.tempos, original.tempos);
    assert_eq!(rewritten.sigs, original.sigs);
    assert_eq!(rewritten.keys, original.keys);

    // And reading it again gives the same part.
    assert_eq!(Part::from_midi(&written).unwrap(), part);
}

#[test]
fn played_take_round_trips_exactly_at_common_resolutions() {
    for tpq in [96, 384, 480, 960] {
        let (bytes, expected) = played_take(tpq, Format::Parallel);
        check_round_trip(&bytes, &expected);
    }
}

#[test]
fn format_0_file_round_trips_exactly() {
    let (bytes, expected) = played_take(480, Format::SingleTrack);
    check_round_trip(&bytes, &expected);
}

#[test]
fn reads_musical_positions() {
    let (bytes, _) = played_take(480, Format::Parallel);
    let part = Part::from_midi(&bytes).unwrap();
    assert_eq!(part.title.as_deref(), Some("Sketch in D"));
    assert_eq!(part.tempo_bpm, 92.0);
    assert_eq!(part.time_sig.to_string(), "4/4");
    assert_eq!(part.key.unwrap().to_string(), "D minor");
    let changes: Vec<String> = part
        .time_changes
        .iter()
        .map(|c| format!("{} at {}", c.time_sig, c.at))
        .collect();
    // 6/8 at bar 5; 3/4 halfway through bar 6 (the fourth eighth), so bar 7 starts there.
    assert_eq!(changes, ["6/8 at 5.1.0", "3/4 at 6.4.0"]);
    assert_eq!(part.tempo_changes[0].at.to_string(), "6.1.0");
    assert_eq!(part.tempo_changes[0].bpm, 100.0);

    let names: Vec<(&str, u8)> = part
        .tracks
        .iter()
        .map(|t| (t.name.as_str(), t.channel))
        .collect();
    assert_eq!(names, [("Piano", 1), ("Rhythm", 2), ("Rhythm", 10)]);
    let piano = &part.tracks[0];
    assert_eq!(piano.program, Some(0));
    // D minor spells black keys with flats.
    assert!(piano.notes.iter().any(|n| n.pitch.to_string() == "Bb4"));
    let chord: Vec<String> = piano
        .notes
        .iter()
        .filter(|n| n.at.to_string() == "4.1.0" && n.beats == 4.0)
        .map(|n| n.pitch.to_string())
        .collect();
    assert_eq!(chord, ["A3", "C4", "E5"]);
    assert_eq!(part.tracks[1].program, Some(33));
}

#[test]
fn hand_written_text_is_read_leniently() {
    let text = r#"
# A sketch written by hand.
tempo 92
time 4/4
key Bb major

[track "Lead" ch3 prog73]   # flute
1.1.0   Bb4   1/3   80
1.1.160 C5    1/3   80
1.1.320 D5    1/3   80
1.2     Eb5   0.5   72      # no tick means tick 0
1.1.960 F5    1     90      # ticks past the beat carry on (this is 1.3.0)
2.1.0   cc1   64
2.1.0   bend  +1024
"#;
    let part = Part::from_text(text).unwrap();
    assert_eq!(part.ticks_per_quarter, 480);
    let lead = &part.tracks[0];
    assert_eq!((lead.channel, lead.program), (3, Some(73)));
    assert_eq!(lead.notes.len(), 5);
    assert_eq!(lead.controls.len(), 2);

    let back = Part::from_midi(&part.to_midi().unwrap()).unwrap();
    let notes: Vec<String> = back.tracks[0]
        .notes
        .iter()
        .map(|n| format!("{} {} {} {}", n.at, n.pitch, n.beats, n.vel))
        .collect();
    assert_eq!(
        notes,
        [
            "1.1.0 Bb4 0.333 80",
            "1.1.160 C5 0.333 80",
            "1.1.320 D5 0.333 80",
            "1.2.0 Eb5 0.5 72",
            "1.3.0 F5 1 90",
        ]
    );
}

#[test]
fn errors_name_the_line() {
    let text = "tempo 92\n\n[track \"Piano\" ch1]\n1.1.0 D4 1 80\n1.2.0 H4 1 80\n";
    let err = Part::from_text(text).unwrap_err().to_string();
    assert!(err.starts_with("line 5:"), "{err}");
    assert!(err.contains("H4"), "{err}");

    for (bad, line) in [
        ("[track \"Piano\" ch17]", 1),
        ("tempo fast", 1),
        ("time 4/3", 1),
        ("key H minor", 1),
        ("[track \"P\" ch1]\n1.0.0 D4 1 80", 2),
        ("[track \"P\" ch1]\n1.1.0 D4 1 0", 2),
        ("[track \"P\" ch1]\n1.1.0 D4 1", 2),
        ("[track \"P\" ch1]\n1.1.0 cc64 128", 2),
    ] {
        match Part::from_text(bad) {
            Err(duet_core::Error::Parse { line: l, .. }) => assert_eq!(l, line, "{bad}"),
            other => panic!("{bad}: expected a parse error, got {other:?}"),
        }
    }
}

#[test]
fn sample_sketch_round_trips() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/sketch.notes");
    let text = std::fs::read_to_string(path).unwrap();
    let part = Part::from_text(&text).unwrap();
    let back = Part::from_midi(&part.to_midi().unwrap()).unwrap();
    assert_eq!(back, part);
    assert_eq!(back.to_text(), text, "the sample is in canonical form");
    assert!(text.contains("\nend 5.1.0\n"), "the sample states its end");
    check_document(&part);
}

#[test]
fn track_names_with_brackets_round_trip() {
    let names = ["Pad [verb]", "]", "[a] \"b\" ]c[", "x\\]"];
    let mut part = Part::default();
    for (i, name) in names.iter().enumerate() {
        let mut track = duet_core::Track::new(*name, i as u8 + 1);
        track.notes.push(duet_core::Note::new(
            "1.1.0".parse().unwrap(),
            "C4".parse().unwrap(),
            1.0,
            80,
        ));
        part.tracks.push(track);
    }
    let from_midi = Part::from_midi(&part.to_midi().unwrap()).unwrap();
    let text = from_midi.to_text();
    let back = Part::from_text(&text).unwrap();
    assert_eq!(back, from_midi, "{text}");
    let got: Vec<&str> = back.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(got, names);

    let track = &Part::from_text("[track \"Pad [verb]\" ch2 prog5]  # strings\n")
        .unwrap()
        .tracks[0];
    assert_eq!((track.name.as_str(), track.channel), ("Pad [verb]", 2));
    assert_eq!(track.program, Some(5));

    for bad in ["[track \"Pad [verb]\" ch1", "[track \"Pad ch1]"] {
        match Part::from_text(bad) {
            Err(duet_core::Error::Parse { line: 1, .. }) => {}
            other => panic!("{bad}: expected a parse error on line 1, got {other:?}"),
        }
    }
}

fn notes_of(part: &Part) -> Vec<String> {
    part.tracks[0]
        .notes
        .iter()
        .map(|n| format!("{} {} {} {}", n.at, n.pitch, n.beats, n.vel))
        .collect()
}

#[test]
fn same_pitch_notes_that_start_together_keep_their_lengths() {
    // Listed long first and short first, with a zero-length note among them: each note
    // keeps its own length and velocity whatever order the text gives.
    for lines in [
        "1.1.0 C4 2 50\n1.1.0 C4 0.5 100\n1.1.0 C4 0 110\n",
        "1.1.0 C4 0 110\n1.1.0 C4 0.5 100\n1.1.0 C4 2 50\n",
    ] {
        let text = format!("[track \"P\" ch1]\n{lines}");
        let part = Part::from_text(&text).unwrap();
        let back = Part::from_midi(&part.to_midi().unwrap()).unwrap();
        let mut got = notes_of(&back);
        got.sort();
        assert_eq!(
            got,
            ["1.1.0 C4 0 110", "1.1.0 C4 0.5 100", "1.1.0 C4 2 50"],
            "{lines}"
        );
    }
}

#[test]
fn overlapping_same_pitch_notes_round_trip_but_nested_ones_are_refused() {
    // Overlapping, ending together, and starting where another ends: all kept exactly.
    let text = "[track \"P\" ch1]\n\
                1.1.0 C4 1 80\n1.1.240 C4 1 90\n\
                2.1.0 D4 2 80\n2.2.0 D4 1 90\n\
                3.1.0 E4 1 80\n3.2.0 E4 0 90\n";
    let part = Part::from_text(text).unwrap();
    let back = Part::from_midi(&part.to_midi().unwrap()).unwrap();
    assert_eq!(notes_of(&back), notes_of(&part));

    // A note inside a longer note of the same pitch would come back changed, so writing
    // MIDI refuses it and names both notes.
    for inner in ["1.2.0 C4 0.5 100", "1.2.0 C4 0 100", "1.4.0 C4 0.5 100"] {
        let text = format!("[track \"P\" ch1]\n1.1.0 C4 4 80\n{inner}\n1.2.0 D4 0.5 70\n");
        let part = Part::from_text(&text).unwrap();
        let err = part.to_midi().unwrap_err().to_string();
        assert!(
            err.contains("C4 at 1.2.0") || err.contains("C4 at 1.4.0"),
            "{err}"
        );
        assert!(err.contains("inside the C4 at 1.1.0"), "{err}");
    }
    // The same notes on different channels are separate notes and are fine.
    let text = "[track \"A\" ch1]\n1.1.0 C4 4 80\n[track \"B\" ch2]\n1.2.0 C4 0.5 100\n";
    assert!(Part::from_text(text).unwrap().to_midi().is_ok());
}

#[test]
fn black_keys_are_spelled_by_the_key_in_force() {
    // D major spells sharps, and spells the A#/Bb outside its scale as A#; from bar 3 the
    // key is Bb major, which spells both as flats. A note right on the change takes the
    // new key.
    let text = "key D major\nkey Bb major at 3.1.0\n\n[track \"P\" ch1]\n\
                1.1.0 F#4 1 80\n2.1.0 A#4 1 80\n3.1.0 Bb4 1 80\n3.2.0 Eb4 1 80\n";
    let part = Part::from_text(text).unwrap();
    let back = Part::from_midi(&part.to_midi().unwrap()).unwrap();
    assert_eq!(
        notes_of(&back),
        [
            "1.1.0 F#4 1 80",
            "2.1.0 A#4 1 80",
            "3.1.0 Bb4 1 80",
            "3.2.0 Eb4 1 80"
        ]
    );
    assert_eq!(back, stated(&part));
}

#[test]
fn six_accidental_keys_and_double_accidentals_keep_their_spelling_through_midi_text_and_documents()
{
    // Each scale written with MIDI's key signature alone: the reader spells the notes as the
    // key does, Cb under six flats, E# under six sharps and C## for D# minor's raised leading
    // tone, and keeps every MIDI note.
    for (key, notes, spelled) in [
        (
            "Eb minor",
            [63, 65, 66, 68, 70, 71, 73, 72],
            "Eb4 F4 Gb4 Ab4 Bb4 Cb5 Db5 C5",
        ),
        (
            "Gb major",
            [66, 68, 70, 71, 73, 75, 77, 78],
            "Gb4 Ab4 Bb4 Cb5 Db5 Eb5 F5 Gb5",
        ),
        (
            "F# major",
            [66, 68, 70, 71, 73, 75, 77, 78],
            "F#4 G#4 A#4 B4 C#5 D#5 E#5 F#5",
        ),
        (
            "D# minor",
            [63, 65, 66, 68, 70, 71, 74, 75],
            "D#4 E#4 F#4 G#4 A#4 B4 C##5 D#5",
        ),
    ] {
        let mut text = format!("key {key}\n\n[track \"P\" ch1]\n");
        for (i, midi) in notes.iter().enumerate() {
            let pitch = duet_core::Pitch::new(*midi).unwrap();
            text.push_str(&format!("{}.1.0 {pitch} 1 80\n", i + 1));
        }
        let back = Part::from_midi(&Part::from_text(&text).unwrap().to_midi().unwrap()).unwrap();
        let got: Vec<String> = back.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch.to_string())
            .collect();
        assert_eq!(got.join(" "), spelled, "{key}");
        let midi: Vec<u8> = back.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch.midi())
            .collect();
        assert_eq!(midi, notes, "{key}: a note changed");

        // The spelled part through MIDI again, the notes view and a part document: exact.
        assert_eq!(Part::from_midi(&back.to_midi().unwrap()).unwrap(), back);
        let view = back.to_text();
        for name in spelled.split(' ') {
            assert!(
                view.contains(&format!(" {name} ")),
                "{name} not in the view:\n{view}"
            );
        }
        assert_eq!(Part::from_text(&view).unwrap(), back, "{view}");
        check_document(&back);
    }

    // A hand-written Cb, B#, E#, Fb, double sharp or double flat keeps its spelling through
    // the text and a document, in any key: the spelling is the part's, and MIDI does not
    // carry it.
    let text = "key C major\n\n[track \"P\" ch1]\n\
                1.1.0 Cb5 1 80\n2.1.0 B#3 1 80\n3.1.0 E#4 1 80\n4.1.0 Fb4 1 80\n\
                5.1.0 F##4 1 80\n6.1.0 Bbb3 1 80\n";
    let part = stated(&Part::from_text(text).unwrap());
    assert_eq!(
        notes_of(&part),
        [
            "1.1.0 Cb5 1 80",
            "2.1.0 B#3 1 80",
            "3.1.0 E#4 1 80",
            "4.1.0 Fb4 1 80",
            "5.1.0 F##4 1 80",
            "6.1.0 Bbb3 1 80"
        ]
    );
    let midi: Vec<u8> = part.tracks[0]
        .notes
        .iter()
        .map(|n| n.pitch.midi())
        .collect();
    assert_eq!(midi, [71, 60, 65, 64, 67, 57]);
    check_document(&part);
}

#[test]
fn format_0_sequence_name_is_the_title() {
    let (bytes, _) = played_take(480, Format::SingleTrack);
    let part = Part::from_midi(&bytes).unwrap();
    assert_eq!(part.title.as_deref(), Some("Sketch in D"));
    // The tracks split from the one MIDI track by channel have no names of their own.
    let names: Vec<(&str, u8)> = part
        .tracks
        .iter()
        .map(|t| (t.name.as_str(), t.channel))
        .collect();
    assert_eq!(
        names,
        [("Channel 1", 1), ("Channel 2", 2), ("Channel 10", 10)]
    );
    // Written as format 1, the title and names come back unchanged.
    assert_eq!(
        Part::from_midi(&part.to_midi().unwrap()).unwrap(),
        stated(&part)
    );
}

#[test]
fn names_that_are_not_utf8_are_read_with_replacement_characters() {
    // Documented in the README: bytes that are not UTF-8 become U+FFFD.
    let bytes = save(
        Format::Parallel,
        480,
        vec![
            track(vec![(0, meta(MetaMessage::TrackName(b"Caf\xe9")))]),
            track(vec![
                (0, meta(MetaMessage::TrackName(b"Pi\xe0no"))),
                (0, on(0, 60, 80)),
                (480, off(0, 60, false)),
            ]),
        ],
    );
    let part = Part::from_midi(&bytes).unwrap();
    assert_eq!(part.title.as_deref(), Some("Caf\u{FFFD}"));
    assert_eq!(part.tracks[0].name, "Pi\u{FFFD}no");
}

#[test]
fn header_lines_after_a_track_say_where_they_belong() {
    for (line, kw) in [
        ("tempo 96 at 9.1.0", "tempo"),
        ("time 3/4 at 17.1.0", "time"),
        ("key A minor", "key"),
        ("tpq 960", "tpq"),
        ("title \"Late\"", "title"),
    ] {
        let text = format!("[track \"P\" ch1]\n1.1.0 C4 1 80\n{line}\n");
        match Part::from_text(&text) {
            Err(duet_core::Error::Parse { line: 3, msg }) => {
                assert!(msg.contains(&format!("'{kw}' is a header line")), "{msg}");
                assert!(msg.contains("before the first [track]"), "{msg}");
            }
            other => panic!("{line}: expected a parse error on line 3, got {other:?}"),
        }
    }
}

/// Whether an absolute tick is on `step`'s grid, which restarts at every bar line.
fn on_grid(map: &duet_core::TimeMap, tick: u64, step: u64) -> bool {
    let bar_start = map.tick(Position::bar(map.position(tick).bar)).unwrap();
    (tick - bar_start).is_multiple_of(step)
}

#[test]
fn a_quantized_take_lies_on_its_grid() {
    for tpq in [96, 384, 480, 960] {
        let (bytes, _) = played_take(tpq, Format::Parallel);
        let part = Part::from_midi(&bytes).unwrap();
        let map = part.time_map().unwrap();
        for grid in ["1/4", "1/8", "1/16", "1/8t", "1/16t"] {
            let grid: Grid = grid.parse().unwrap();
            let step = grid.step_ticks(tpq).unwrap();
            let q = part.quantized(grid).unwrap();
            let case = format!("{grid} at {tpq} tpq");

            // Only notes move: everything else is as it was.
            assert_eq!(q.tracks.len(), part.tracks.len(), "{case}");
            assert_eq!(q.tempo_changes, part.tempo_changes, "{case}");
            assert_eq!(q.time_changes, part.time_changes, "{case}");
            assert_eq!(q.key_changes, part.key_changes, "{case}");
            for (qt, t) in q.tracks.iter().zip(&part.tracks) {
                assert_eq!(
                    (&qt.name, qt.channel, qt.program),
                    (&t.name, t.channel, t.program)
                );
                assert_eq!(qt.controls, t.controls, "{case}");
                assert_eq!(qt.notes.len(), t.notes.len(), "{case}: no note is lost");
                for (qn, n) in qt.notes.iter().zip(&t.notes) {
                    // Same note, in the same place in the list.
                    assert_eq!((qn.pitch, qn.vel), (n.pitch, n.vel), "{case}");
                    let start = map.tick(n.at).unwrap();
                    let end = start + (n.beats * f64::from(tpq)).round() as u64;
                    let q_start = map.tick(qn.at).unwrap();
                    let q_len = (qn.beats * f64::from(tpq)).round() as u64;
                    let q_end = q_start + q_len;
                    let at = format!("{case}: {} {} {}", n.at, n.pitch, n.beats);
                    assert!(
                        on_grid(&map, q_start, step),
                        "{at}: start {q_start} off grid"
                    );
                    assert!(on_grid(&map, q_end, step), "{at}: end {q_end} off grid");
                    assert!(
                        start.abs_diff(q_start) * 2 <= step,
                        "{at}: start not nearest"
                    );
                    // Zero length stays zero; a played note never collapses to nothing.
                    assert_eq!(q_len == 0, end == start, "{at}: length {q_len}");
                    if q_len > step {
                        assert!(end.abs_diff(q_end) * 2 <= step, "{at}: end not nearest");
                    }
                }
            }
            assert_eq!(
                q.quantized(grid).unwrap(),
                q,
                "{case}: quantizing twice changes nothing"
            );
        }
    }
}

#[test]
fn quantizing_rounds_to_the_nearest_step_and_keeps_collapsed_notes() {
    let eighths: Grid = "1/8".parse().unwrap();
    let part = Part::from_text(
        "tpq 480\ntime 4/4\n\n[track \"P\" ch1]\n\
         1.1.10 C4 0.48 80\n\
         1.2.120 D4 0.5 81\n\
         1.3.10 E4 0.05 82\n\
         1.4.0 F4 0 83\n",
    )
    .unwrap();
    assert_eq!(
        notes_of(&part.quantized(eighths).unwrap()),
        [
            // Start and end each go to the nearest eighth.
            "1.1.0 C4 0.5 80",
            // Exactly halfway between two eighths, start and end both go to the later one.
            "1.2.240 D4 0.5 81",
            // It would collapse to nothing, so it gets one step.
            "1.3.0 E4 0.5 82",
            // It was zero length already, and stays zero length.
            "1.4.0 F4 0 83",
        ]
    );
    // The part quantizing was called on is unchanged.
    assert_eq!(notes_of(&part)[0], "1.1.10 C4 0.48 80");

    // A 6/8 bar cut short after three eighths: on a quarter grid its last step is one
    // eighth long, because the bar line is always on the grid.
    let cut = Part::from_text(
        "tpq 480\ntime 6/8\ntime 4/4 at 1.4.0\n\n[track \"P\" ch1]\n\
         1.3.200 G4 1 84\n\
         1.2.10 A4 0.05 85\n",
    )
    .unwrap();
    assert_eq!(
        notes_of(&cut.quantized("1/4".parse().unwrap()).unwrap()),
        [
            // Tick 680 lies between the quarter at 480 and the bar line at 720.
            "2.1.0 G4 1 84",
            // It collapses at 480, and the next grid point is the bar line, an eighth on.
            "1.3.0 A4 0.5 85",
        ]
    );
}

#[test]
fn a_grid_finer_than_the_resolution_is_refused_by_name() {
    let (bytes, _) = played_take(96, Format::Parallel);
    let part = Part::from_midi(&bytes).unwrap();
    let err = part
        .quantized("1/5".parse().unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("1/5 grid") && err.contains("96 ticks per quarter"),
        "{err}"
    );
}

#[test]
fn no_read_write_or_conversion_quantizes_unasked() {
    // A played take is off every grid. Going through every format and back to MIDI keeps
    // it off the grid exactly, and quantizing a copy leaves the original as it was.
    let (bytes, expected) = played_take(480, Format::Parallel);
    let part = Part::from_midi(&bytes).unwrap();
    let before = part.clone();
    let _ = part.quantized("1/16".parse().unwrap()).unwrap();
    assert_eq!(part, before, "quantizing changed the part it was called on");
    let through_all = Part::from_text(&part.to_text()).unwrap();
    let through_all = Document::<PartBody>::read(
        &Document::new(
            Tool::new("duet", "test"),
            PartBody::new(id(), Source::hand(), through_all),
        )
        .write()
        .unwrap(),
    )
    .unwrap()
    .body
    .part;
    assert_eq!(events(&through_all.to_midi().unwrap()).notes, expected);
}

/// The end of track of every track, in file order, as absolute ticks.
fn ends_of_track(bytes: &[u8]) -> Vec<u64> {
    let smf = Smf::parse(bytes).unwrap();
    smf.tracks
        .iter()
        .map(|t| {
            let mut tick = 0u64;
            for e in t {
                tick += u64::from(e.delta.as_int());
                if let TrackEventKind::Meta(MetaMessage::EndOfTrack) = e.kind {
                    return tick;
                }
            }
            panic!("a track with no end of track");
        })
        .collect()
}

/// `track`, ending at `end` instead of at its last event.
fn track_ending_at(
    timed: Vec<(u64, TrackEventKind<'static>)>,
    end: u64,
) -> Vec<TrackEvent<'static>> {
    let mut out = track(timed);
    let last: u64 = out.iter().map(|e| u64::from(e.delta.as_int())).sum();
    out.last_mut().expect("an end of track").delta = u28::new((end - last) as u32);
    out
}

/// Witness for a part's end: a MIDI file's end of track survives MIDI → part → MIDI,
/// through text, JSON and TOML too, whether it falls on a bar line or inside a bar, at
/// several resolutions and meters, and with the tracks ending at different ticks (the
/// file's end is the latest). Every track of the written file ends there.
#[test]
fn a_midi_files_end_of_track_survives() {
    // (tpq, numerator, denominator power, the end in ticks from the last note's end)
    let cases: [(u16, u8, u8, u64); 6] = [
        (480, 4, 2, 4 * 480),       // a whole empty 4/4 bar after the music
        (480, 4, 2, 3 * 480 + 240), // and a dotted half more, so it ends mid-bar
        (96, 3, 2, 7),              // a few ticks: mid-beat
        (960, 6, 3, 6 * 480 * 2),   // two 6/8 bars
        (384, 7, 3, 1),             // one tick
        (192, 5, 2, 5 * 192 * 3),   // three 5/4 bars
    ];
    for (tpq, num, den_power, extra) in cases {
        let note_end = u64::from(tpq) * 2 + 30;
        let file_end = note_end + extra;
        let conductor = track_ending_at(
            vec![
                (0, meta(MetaMessage::TimeSignature(num, den_power, 24, 8))),
                (0, meta(MetaMessage::Tempo(u24::new(600_000)))),
            ],
            file_end,
        );
        let notes = track_ending_at(
            vec![(30, on(0, 60, 90)), (note_end, off(0, 60, true))],
            // A track may end before the file does; the file's end is the latest.
            note_end + extra / 2,
        );
        let bytes = save(Format::Parallel, tpq, vec![conductor, notes]);

        let part = Part::from_midi(&bytes).unwrap();
        let map = part.time_map().unwrap();
        let want = map.position(file_end);
        assert_eq!(part.end, Some(want), "tpq {tpq}, {num}/2^{den_power}");
        assert!(part.to_text().contains(&format!("\nend {want}\n")));

        let from_text = Part::from_text(&part.to_text()).unwrap();
        assert_eq!(from_text, part, "text");
        check_document(&part);

        let written = from_text.to_midi().unwrap();
        assert_eq!(
            ends_of_track(&written),
            vec![file_end, file_end],
            "every track ends there"
        );
        assert_eq!(events(&written).notes, events(&bytes).notes);
        assert_eq!(
            Part::from_midi(&written).unwrap(),
            part,
            "MIDI → part → MIDI → part"
        );
    }
}

/// A part that names an end keeps it through MIDI and back, when the end falls on a bar
/// line, inside a bar, and exactly where the last note ends: the first track holds only
/// the conductor, ends after its last event, and so says where the part ends.
#[test]
fn a_parts_end_survives_midi() {
    for end in ["3.1.0", "2.3.240", "2.2.0", "9.1.0"] {
        let text = format!(
            "tpq 480\ntempo 100\ntime 4/4\nend {end}\n\n[track \"P\" ch1]\n1.1.0 C4 1 80\n1.3.0 D4 1.5 80\n"
        );
        let part = Part::from_text(&text).unwrap();
        assert_eq!(
            part.end.map(|e| e.to_string()),
            Some(end.to_owned()),
            "{end}"
        );
        let midi = part.to_midi().unwrap();
        assert_eq!(Part::from_midi(&midi).unwrap(), part, "end {end}");
    }
    // An end is read like any position: offsets past a beat or a bar carry on.
    let part = Part::from_text("time 4/4\nend 2.4.960\n").unwrap();
    assert_eq!(part.end.map(|e| e.to_string()).as_deref(), Some("3.2.0"));
}

/// Anything past a part's end is refused by name, in every place a part is made or
/// written, and never cut to fit: a note that ends past it, a control, a tempo, meter or
/// key change, and a note that quantizing would push past it.
#[test]
fn what_lies_past_the_end_is_refused_by_name_never_truncated() {
    let head = "tpq 480\ntime 4/4\n";
    let cases = [
        (
            "end 2.1.0\n\n[track \"P\" ch1]\n1.4.0 C4 1.5 80\n",
            "track 'P': the C4 at 1.4.0, which ends at 2.1.240, lies past the part's end at 2.1.0",
        ),
        (
            "end 2.1.0\n\n[track \"P\" ch1]\n2.1.1 cc64 0\n",
            "track 'P': the cc64 at 2.1.1 lies past the part's end at 2.1.0",
        ),
        (
            "end 2.1.0\n\n[track \"P\" ch1]\n3.1.0 bend 10\n",
            "track 'P': the bend at 3.1.0 lies past the part's end",
        ),
        (
            "end 2.1.0\ntempo 90 at 2.2.0\n",
            "the tempo 90 change at 2.2.0 lies past the part's end at 2.1.0",
        ),
        (
            "end 2.1.0\ntime 3/4 at 3.1.0\n",
            "the time 3/4 change at 3.1.0 lies past the part's end at 2.1.0",
        ),
        (
            "end 2.1.0\nkey D minor at 2.1.1\n",
            "the key D minor change at 2.1.1 lies past the part's end at 2.1.0",
        ),
    ];
    for (body, want) in cases {
        let text = format!("{head}{body}");
        let err = Part::from_text(&text).expect_err(want).to_string();
        assert!(err.contains(want), "{err}\nshould say: {want}");

        // The same part built in code is refused by MIDI, JSON's reader path and quantize.
        let mut part =
            Part::from_text(&format!("{head}{}", body.replacen("end 2.1.0\n", "", 1))).unwrap();
        part.end = Some("2.1.0".parse().unwrap());
        let err = part.to_midi().expect_err(want).to_string();
        assert!(err.contains(want), "MIDI: {err}");
        let err = part.clone().normalize().expect_err(want).to_string();
        assert!(err.contains(want), "normalize: {err}");
    }

    // A note ending on the end is fine; quantizing it past the end is refused.
    let part = Part::from_text(&format!(
        "{head}end 2.1.0\n\n[track \"P\" ch1]\n1.4.250 C4 0.479 80\n"
    ))
    .unwrap();
    let before = part.clone();
    let err = part
        .quantized(Grid::new(4, false).unwrap())
        .expect_err("quantizing moves the note to 2.1.0 and gives it one step")
        .to_string();
    assert!(
        err.contains("the C4 at 2.1.0, which ends at 2.2.0, lies past the part's end at 2.1.0"),
        "{err}"
    );
    assert_eq!(part, before, "never changed");
}

/// With no end named, every writer states the default end: the text says `end`, and
/// every MIDI track ends there. A MIDI file whose tracks each end at their last event
/// names no end, so the default applies when it is read.
#[test]
fn a_part_with_no_end_states_its_default_end() {
    let part = Part::from_text(
        "tpq 480\ntempo 90 at 1.3.0\n\n[track \"P\" ch1]\n1.1.0 C4 1 80\n2.1.0 cc64 0\n",
    )
    .unwrap();
    assert_eq!(part.end, None);
    assert!(
        part.to_text().contains("\nend 3.1.0\n"),
        "{}",
        part.to_text()
    );
    let midi = part.to_midi().unwrap();
    assert_eq!(
        ends_of_track(&midi),
        vec![3840, 3840],
        "every track ends at 3.1.0"
    );
    assert_eq!(Part::from_midi(&midi).unwrap(), stated(&part));

    // A file from another tool, each track ending at its last event: no end, so the
    // default applies, and the music is the same.
    let bytes = save(
        Format::Parallel,
        480,
        vec![track(vec![(0, on(0, 60, 80)), (480, off(0, 60, true))])],
    );
    let read = Part::from_midi(&bytes).unwrap();
    assert_eq!(read.end, None);
    assert_eq!(read.stated_end().unwrap().to_string(), "2.1.0");

    // The default never comes before a tempo, meter or key change, so a part that names
    // no end is never refused for one.
    let late = Part::from_text("tempo 90 at 9.1.0\n\n[track \"P\" ch1]\n1.1.0 C4 1 80\n").unwrap();
    let midi = late.to_midi().unwrap();
    assert_eq!(ends_of_track(&midi), vec![8 * 1920, 8 * 1920]);
}

/// Every `duet:key` text event in a file: (tick, text).
fn key_texts(bytes: &[u8]) -> Vec<(u64, String)> {
    let smf = Smf::parse(bytes).unwrap();
    let mut out = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for e in track {
            tick += u64::from(e.delta.as_int());
            if let TrackEventKind::Meta(MetaMessage::Text(text)) = e.kind {
                let text = String::from_utf8_lossy(text).into_owned();
                if text.starts_with("duet:key ") {
                    out.push((tick, text));
                }
            }
        }
    }
    out
}

#[test]
fn a_key_with_a_mode_round_trips_through_text_a_document_and_midi() {
    // D dorian has no sharps or flats, and E phrygian at bar 3 has none either: only the
    // mode tells them from C major, so only the duet:key event carries it through MIDI.
    let text = "key D dorian\nkey E phrygian at 3.1.0\nkey G mixolydian at 4.1.0\n\
                key F lydian at 5.1.0\nkey B locrian at 6.1.0\nkey Bb major at 7.1.0\n\n\
                [track \"P\" ch1]\n1.1.0 D4 1 80\n7.1.0 Bb4 1 80\n";
    let part = Part::from_text(text).unwrap();
    assert_eq!(part.key.unwrap().to_string(), "D dorian");
    let keys: Vec<String> = part.key_changes.iter().map(|c| c.key.to_string()).collect();
    assert_eq!(
        keys,
        [
            "E phrygian",
            "G mixolydian",
            "F lydian",
            "B locrian",
            "Bb major"
        ]
    );
    let part = stated(&part);

    let text = part.to_text();
    assert!(text.contains("key D dorian\n"), "{text}");
    assert_eq!(Part::from_text(&text).unwrap(), part, "text round-trip");
    let doc = Document::new(
        Tool::new("duet", "test"),
        PartBody::new(id(), Source::hand(), part.clone()),
    )
    .write()
    .unwrap();
    assert!(doc.contains("\"key\": \"D dorian\""), "{doc}");
    check_document(&part);

    // MIDI holds each key's own signature, and a duet:key event beside each mode.
    let midi = part.to_midi().unwrap();
    let ev = events(&midi);
    assert_eq!(
        ev.keys,
        [
            (0, 0, false),
            (3840, 0, false),
            (5760, 0, false),
            (7680, 0, false),
            (9600, 0, false),
            (11520, -2, false)
        ]
    );
    assert_eq!(
        key_texts(&midi),
        [
            (0, "duet:key D dorian".to_owned()),
            (3840, "duet:key E phrygian".to_owned()),
            (5760, "duet:key G mixolydian".to_owned()),
            (7680, "duet:key F lydian".to_owned()),
            (9600, "duet:key B locrian".to_owned()),
        ],
        "major and minor need no text event"
    );
    assert_eq!(Part::from_midi(&midi).unwrap(), part, "MIDI round-trip");
}

#[test]
fn a_duet_key_event_that_disagrees_with_the_signature_is_ignored() {
    // A DAW that edits the key signature leaves our text event behind. The signature is
    // what the DAW shows, so it wins: the key reads as the signature's major key.
    let bytes = save(
        Format::Parallel,
        480,
        vec![track(vec![
            (0, meta(MetaMessage::KeySignature(2, false))),
            (0, meta(MetaMessage::Text(b"duet:key D dorian"))),
            (0, on(0, 62, 80)),
            (480, off(0, 62, false)),
        ])],
    );
    let part = Part::from_midi(&bytes).unwrap();
    assert_eq!(part.key.unwrap().to_string(), "D major");

    // The same when only the minor flag moved: D dorian edited to A minor keeps no sharps.
    let bytes = save(
        Format::Parallel,
        480,
        vec![track(vec![
            (0, meta(MetaMessage::KeySignature(0, true))),
            (0, meta(MetaMessage::Text(b"duet:key D dorian"))),
            (0, on(0, 62, 80)),
            (480, off(0, 62, false)),
        ])],
    );
    let part = Part::from_midi(&bytes).unwrap();
    assert_eq!(part.key.unwrap().to_string(), "A minor");
}

#[test]
fn modes_are_named_on_their_first_three_letters() {
    use duet_core::Key;
    for (written, read) in [
        ("D dorian", "D dorian"),
        ("D Dor", "D dorian"),
        ("F#Mix", "F# mixolydian"),
        ("F# MIXOLYDIAN", "F# mixolydian"),
        ("A aeolian", "A minor"),
        ("C ionian", "C major"),
        ("Am", "A minor"),
        ("Eb", "Eb major"),
        ("B#Loc", "B# locrian"),
        ("FbLyd", "Fb lydian"),
    ] {
        assert_eq!(
            written.parse::<Key>().unwrap().to_string(),
            read,
            "{written}"
        );
    }
    // A tonic whose mode needs more than seven sharps or flats has no signature.
    assert!("D# lydian".parse::<Key>().is_err());
    assert!("D blues".parse::<Key>().is_err());
}

/// A committed `capture-analysis` example, by its version.
fn analysis_example(version: &str) -> String {
    let path = format!(
        "{}/../duet-schemas/schemas/capture-analysis/examples/{version}.duet",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

/// Each note's `excursions` in a capture-analysis document's text, `None` where it has none.
fn excursions_in(text: &str) -> Vec<Option<serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value["body"]["reading"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["residual"].get("excursions").cloned())
        .collect()
}

#[test]
fn a_basic_pitch_reader_round_trips_exactly_through_json() {
    // capture-analysis 1.8: a reading by Basic Pitch names its reader `basic-pitch`, and the
    // document writes back as the text it was read from. As 1.8 and 1.9 wrote it, it has no
    // thresholds.
    let text = analysis_example("1.14").replace(r#""name": "hmm""#, r#""name": "basic-pitch""#);
    assert!(
        text.contains(r#""name": "basic-pitch""#),
        "the example has no hmm reader"
    );
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    let written = doc.write().unwrap();
    assert_eq!(written, text);
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        value["body"]["reading"]["reader"]["name"],
        serde_json::json!("basic-pitch")
    );
    assert!(
        value["body"]["reading"]["reader"]
            .get("thresholds")
            .is_none()
    );
}

#[test]
fn a_basic_pitch_readers_thresholds_round_trip_exactly_through_json() {
    // capture-analysis 1.10: the thresholds a basic-pitch reading was decoded with read back to
    // the bit, and the document writes back as the text it was read from.
    let hmm = "\"name\": \"hmm\",\n        \"note_cost\": 4.0\n";
    let basic_pitch = concat!(
        "\"name\": \"basic-pitch\",\n",
        "        \"note_cost\": 4.0,\n",
        "        \"thresholds\": {\n",
        "          \"onset_threshold\": 0.62,\n",
        "          \"frame_threshold\": 0.1875,\n",
        "          \"minimum_note_length_ms\": 211.3\n",
        "        }\n",
    );
    let example = analysis_example("1.14");
    assert!(
        example.contains(hmm),
        "the example's reader is not as written here"
    );
    let text = example.replace(hmm, basic_pitch);
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    let thresholds = doc
        .body
        .reading
        .as_ref()
        .and_then(|r| r.reader)
        .and_then(|r| r.thresholds)
        .unwrap();
    assert_eq!(
        (
            thresholds.onset_threshold.to_bits(),
            thresholds.frame_threshold.to_bits(),
            thresholds.minimum_note_length_ms.to_bits()
        ),
        (
            0.62_f64.to_bits(),
            0.1875_f64.to_bits(),
            211.3_f64.to_bits()
        )
    );
    assert_eq!(doc.write().unwrap(), text);
}

#[test]
fn a_notes_excursions_round_trip_exactly_through_json() {
    // capture-analysis 1.6, at the current version's example: every excursion's start, end and
    // cents read back to the bit, and the document writes back as the text it was read from.
    let text = analysis_example("1.14");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    assert_eq!(
        excursions_in(&text),
        [
            Some(serde_json::json!([
                { "start": 11100, "end": 11868, "cents": -96.5 },
                { "start": 14428, "end": 15452, "cents": 198.0 }
            ])),
            None
        ]
    );
}

#[test]
fn a_document_before_excursions_reads_as_one_with_none_and_writes_none() {
    // A 1.5 document states no ornament: read today, it holds none, and writing it states none,
    // rather than an empty list.
    let text = analysis_example("1.5");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    let reading = doc.body.reading.as_ref().unwrap();
    assert!(
        reading
            .notes
            .iter()
            .all(|n| n.residual.as_ref().unwrap().excursions.is_empty())
    );
    let written = doc.write().unwrap();
    assert_eq!(excursions_in(&written), [None, None]);
    let AnyDocument::CaptureAnalysis(again) = AnyDocument::read(&written).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(again, doc);
}

/// The reading's `rejoins` in a capture-analysis document's text, `None` where it has none.
fn rejoins_in(text: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value["body"]["reading"].get("rejoins").cloned()
}

#[test]
fn a_readings_rejoins_round_trip_exactly_through_json() {
    // capture-analysis 1.7: every decoded note, dip and loss change a rejoin states reads back
    // to the bit, and the document writes back as the text it was read from.
    let text = analysis_example("1.14");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    let reading = doc.body.reading.as_ref().unwrap();
    assert_eq!(reading.rejoins.len(), 1);
    let rejoin = &reading.rejoins[0];
    assert_eq!(rejoin.decoded.len(), 2);
    assert!(rejoin.decoded.iter().all(|n| n.residual.is_none()));
    assert_eq!(rejoin.decoded[1].held_from, 17224);
    assert_eq!(
        (rejoin.dips[0].start, rejoin.dips[0].cents),
        (16200, -172.5)
    );
    assert_eq!(rejoin.loss_change.to_bits(), 6.375f64.to_bits());
    assert!(
        rejoins_in(&text).unwrap()[0]["decoded"][0]
            .get("residual")
            .is_none(),
        "a decoded note is written without its residual"
    );
}

#[test]
fn a_document_before_rejoins_reads_as_one_with_none_and_writes_none() {
    // A 1.6 document's notes are as decoded: read today, its reading states no rejoin, and
    // writing it states none, rather than an empty list.
    let text = analysis_example("1.6");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert!(doc.body.reading.as_ref().unwrap().rejoins.is_empty());
    let written = doc.write().unwrap();
    assert_eq!(rejoins_in(&written), None);
    let AnyDocument::CaptureAnalysis(again) = AnyDocument::read(&written).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(again, doc);
}

/// The reading's `breathy` in a capture-analysis document's text, `None` where it has none.
fn breathy_in(text: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value["body"]["reading"].get("breathy").cloned()
}

#[test]
fn a_readings_breathy_notes_round_trip_exactly_through_json() {
    // capture-analysis 1.9: every note the breathy pass carried on reads back, as decoded, with
    // its end, its quietest kept frame and its loss change to the bit, and the document writes
    // back as the text it was read from.
    let text = analysis_example("1.14");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    let reading = doc.body.reading.as_ref().unwrap();
    let [breathy] = reading.breathy.as_slice() else {
        panic!("one note carried on: {:?}", reading.breathy);
    };
    assert!(breathy.decoded.residual.is_none());
    assert_eq!((breathy.decoded.start, breathy.decoded.end), (22100, 31832));
    assert_eq!(breathy.end, 32600);
    assert_eq!(breathy.lowest_db.to_bits(), (-9.5f32).to_bits());
    assert_eq!(breathy.loss_change.to_bits(), 2.25f64.to_bits());
    assert!(
        breathy_in(&text).unwrap()[0]["decoded"]
            .get("residual")
            .is_none(),
        "a decoded note is written without its residual"
    );
}

#[test]
fn a_document_before_breathy_notes_reads_as_one_with_none_and_writes_none() {
    // A 1.8 document's notes end where their reader ended them: read today, its reading states
    // no breathy note, and writing it states none, rather than an empty list.
    let text = analysis_example("1.8");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert!(doc.body.reading.as_ref().unwrap().breathy.is_empty());
    let written = doc.write().unwrap();
    assert_eq!(breathy_in(&written), None);
    let AnyDocument::CaptureAnalysis(again) = AnyDocument::read(&written).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(again, doc);
}

/// The reading's `field` in a capture-analysis document's text, `None` where it has none.
fn reading_field(text: &str, field: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value["body"]["reading"].get(field).cloned()
}

#[test]
fn a_readings_held_notes_and_suggestions_round_trip_exactly_through_json() {
    // capture-analysis 1.11: every note the held-pitch pass made reads back, with the note it
    // reaches as decoded and its loss change to the bit; every suggestion reads back with its
    // step, drift and rule; and the document writes back as the text it was read from.
    let text = analysis_example("1.14");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    let reading = doc.body.reading.as_ref().unwrap();
    let [held] = reading.held.as_slice() else {
        panic!("one note reached: {:?}", reading.held);
    };
    assert!(held.decoded.residual.is_none());
    assert_eq!((held.decoded.start, held.decoded.held_from), (34100, 36400));
    let [made] = held.notes.as_slice() else {
        panic!("one note made: {:?}", held.notes);
    };
    assert_eq!((made.start, made.end, made.key), (34100, 35900, 69));
    assert_eq!(made.centre.to_bits(), 21.5f32.to_bits());
    assert_eq!(held.loss_change.to_bits(), 1.625f64.to_bits());
    let [suggestion] = reading.suggestions.as_slice() else {
        panic!("one suggestion: {:?}", reading.suggestions);
    };
    assert_eq!(
        (
            suggestion.start,
            suggestion.key,
            suggestion.next_key,
            suggestion.step
        ),
        (34100, 69, 70, 1)
    );
    assert_eq!(suggestion.drift_cents.to_bits(), 43.5f32.to_bits());
    assert_eq!(
        reading_field(&text, "suggestions").unwrap()[0]["rule"],
        serde_json::json!("semitone-with-drift")
    );
    assert!(
        reading_field(&text, "held").unwrap()[0]["notes"][0]
            .get("residual")
            .is_none(),
        "a made note is written without its residual"
    );
}

#[test]
fn a_document_before_held_notes_reads_as_one_with_none_and_writes_none() {
    // A 1.10 document's held pitches are as its reader read them, with nothing suggested: read
    // today, its reading states neither, and writing it states neither, rather than empty lists.
    let text = analysis_example("1.10");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    let reading = doc.body.reading.as_ref().unwrap();
    assert!(reading.held.is_empty() && reading.suggestions.is_empty());
    let written = doc.write().unwrap();
    assert_eq!(reading_field(&written, "held"), None);
    assert_eq!(reading_field(&written, "suggestions"), None);
    let AnyDocument::CaptureAnalysis(again) = AnyDocument::read(&written).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(again, doc);
}

#[test]
fn a_keys_profile_round_trips_exactly_through_json() {
    // capture-analysis 1.12: the key estimate states the profile its scores were correlated
    // with, and the document writes back as the text it was read from, with each profile.
    let text = analysis_example("1.14");
    for (name, other) in [
        ("aarden-essen", None),
        ("krumhansl-kessler", Some("krumhansl-kessler")),
    ] {
        let text = match other {
            Some(other) => text.replace(
                r#""profile": "aarden-essen""#,
                &format!(r#""profile": "{other}""#),
            ),
            None => text.clone(),
        };
        let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
            panic!("read as another kind");
        };
        assert_eq!(doc.write().unwrap(), text);
        let profile = doc.body.key.as_ref().unwrap().profile;
        assert_eq!(
            serde_json::to_value(profile).unwrap(),
            serde_json::json!(name)
        );
    }
}

#[test]
fn a_document_before_key_profiles_reads_as_stating_none_and_writes_none() {
    // A 1.11 document's key states no profile: read today it states none, and writing it
    // states none, rather than the default.
    let text = analysis_example("1.11");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.body.key.as_ref().unwrap().profile, None);
    let written = doc.write().unwrap();
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert!(value["body"]["key"].get("profile").is_none(), "{written}");
    let AnyDocument::CaptureAnalysis(again) = AnyDocument::read(&written).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(again, doc);
}

#[test]
fn a_take_and_its_passes_round_trip_exactly_through_json() {
    // capture-analysis 1.13: the take's digest, rate, channels and length, and the reading's
    // passes in order, read back as written, and the document writes back as the text it was
    // read from, whatever the passes are and with each field left out.
    let text = analysis_example("1.14");
    let passes = r#""passes": [
        "breathy",
        "rejoin",
        "held-pitch"
      ]"#;
    assert!(
        text.contains(passes),
        "the example's passes are not as written here"
    );
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    let take = doc.body.take.expect("the example states its take");
    assert_eq!(
        take.digest.to_string(),
        "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34"
    );
    assert_eq!(
        (take.rate, take.channels, take.frames),
        (44_100, 2, 1_764_000)
    );
    let reordered = text.replace(
        passes,
        r#""passes": [
        "held-pitch",
        "breathy"
      ]"#,
    );
    let none_ran = text.replace(passes, r#""passes": []"#);
    for text in [reordered, none_ran] {
        let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
            panic!("read as another kind");
        };
        assert_eq!(doc.write().unwrap(), text);
    }
    // Absent, each stays absent: "not stated" is not "none ran".
    let take_block = r#"
    "take": {
      "digest": "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34",
      "rate": 44100,
      "channels": 2,
      "frames": 1764000
    },"#;
    let passes_after = format!(",\n      {passes}");
    assert!(text.contains(take_block) && text.contains(&passes_after));
    let without = text.replace(take_block, "").replace(&passes_after, "");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&without).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.body.take, None);
    assert_eq!(doc.body.reading.as_ref().unwrap().passes, None);
    assert_eq!(doc.write().unwrap(), without);
}

#[test]
fn a_performances_take_round_trips_exactly_through_json() {
    // performance 1.1: the head's take reads back as written, and the stream writes back as
    // the text it was read from; a 1.0 stream, with none, writes none.
    use duet_dsp::PerformanceHead;
    use duet_schemas::Stream;
    let example = |version: &str| {
        std::fs::read_to_string(format!(
            "{}/../duet-schemas/schemas/performance/examples/{version}.duet.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let text = example("1.1");
    let stream = Stream::<PerformanceHead>::read(&text).unwrap();
    assert_eq!(stream.write().unwrap(), text);
    let take = stream.head.take.expect("the example states its take");
    assert_eq!(
        (take.rate, take.channels, take.frames),
        (44_100, 2, 1_764_000)
    );
    let older = Stream::<PerformanceHead>::read(&example("1.0")).unwrap();
    assert_eq!(older.head.take, None);
    let written = older.write().unwrap();
    assert!(!written.contains(r#""take""#), "{written}");
    assert_eq!(Stream::<PerformanceHead>::read(&written).unwrap(), older);
}

/// A project's journal round-trips exactly: a segment and the index, read, then written, are
/// byte-identical. Every entry
/// this build reads is written again from its event, so its text is duet's own; an entry of an
/// event kind it does not read, or of a newer minor, is kept as its text, spacing and key order
/// included.
#[test]
fn a_journal_round_trips_exactly_with_entries_this_duet_does_not_read() {
    use duet_schemas::Journal;
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../duet-schemas/schemas/journal/examples/1.0.duet.jsonl"
    ))
    .unwrap();
    let journal = Journal::read(&example).unwrap();
    assert!(journal.lines.iter().all(|l| l.read().is_some()));
    assert_eq!(journal.write().unwrap(), example);

    let unknown = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"app"},"event":"lyric.sung","v":"1.0","body":{ "z": 1, "a": [2, 3] }}"#;
    let newer = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"mcp"},"event":"ask.asked","v":"1.7","body":{"urgency":"high","question":"Louder?"}}"#;
    let mixed = format!("{example}{unknown}\n{newer}\n");
    let journal = Journal::read(&mixed).unwrap();
    assert_eq!(
        journal.lines.iter().filter(|l| l.read().is_none()).count(),
        2
    );
    assert_eq!(journal.write().unwrap(), mixed);

    // The index over the segments round-trips the same way.
    let index = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../duet-schemas/schemas/journal-index/examples/1.0.duet.jsonl"
    ))
    .unwrap();
    let read = duet_schemas::JournalIndex::read(&index).unwrap();
    assert_eq!(read.write().unwrap(), index);
}

/// The songs build's events round-trip exactly and are read as themselves: a `song.status-set`,
/// a `snapshot.taken`, and a `doc.written` carrying the ids of its song and part. An index line
/// whose summary names songs and parts by id, and one by its name alone, round-trips too, and
/// reads back the ids it names.
#[test]
fn the_songs_events_and_an_index_naming_songs_by_id_round_trip_exactly() {
    use duet_schemas::journal::{Event, IndexLine, read_line};
    use duet_schemas::song::Status;
    let lines = [
        r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"cli"},"cause":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f5","event":"song.status-set","v":"1.0","body":{"song":"0196f1a2-7b3c-7000-8000-000000000001","name":"wayfaring","from":"arranged","to":"developing"}}"#,
        r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"cli"},"event":"snapshot.taken","v":"1.0","body":{"song":"0196f1a2-7b3c-7000-8000-000000000001","name":"wayfaring","snapshot":"2026-11-20-first-verse","digest":"00ff00ff00ff00ff"}}"#,
        r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f8","at":"2026-11-17T15:04:07.000Z","by":{"who":"claude","face":"cli"},"event":"doc.written","v":"1.0","body":{"path":"songs/wayfaring/parts/bass.duet","kind":"part","version":"1.0","digest":"0123456789abcdef","replaced":false,"song":"0196f1a2-7b3c-7000-8000-000000000001","part":"0196f1a2-7b3c-7000-8000-000000000002"}}"#,
    ];
    for text in lines {
        let line = read_line(text).unwrap();
        let event = line
            .read()
            .unwrap_or_else(|| panic!("read as itself: {text}"));
        assert_eq!(line.write().unwrap(), format!("{text}\n"));
        let concerns = event.concerns();
        assert_eq!(
            concerns.songs[0].id.as_ref().map(Id::as_str),
            Some("0196f1a2-7b3c-7000-8000-000000000001"),
            "{text}"
        );
        if let Event::SongStatusSet(set) = event {
            assert_eq!((set.from, set.to), (Status::Arranged, Status::Developing));
        }
    }

    let index = r#"{"opens":{"segment":"20261124T150405000Z","id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f9","from":"2026-11-24T15:04:05.000Z"},"closes":{"segment":"20261123T150405000Z","to":"2026-11-23T15:04:13.000Z","entries":3,"last":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f8","songs":[{"id":"0196f1a2-7b3c-7000-8000-000000000001","name":"wayfaring"}],"parts":[{"id":"0196f1a2-7b3c-7000-8000-000000000002","name":"wayfaring/bass"},{"name":"wayfaring/kit"}]}}"#;
    let line: IndexLine = serde_json::from_str(index).unwrap();
    assert_eq!(line.line().unwrap(), format!("{index}\n"));
    let closes = line.closes.unwrap();
    assert!(closes.concerns_song(&"0196f1a2-7b3c-7000-8000-000000000001".parse().unwrap()));
    assert!(closes.concerns_part(&"0196f1a2-7b3c-7000-8000-000000000002".parse().unwrap()));
}

/// A snapshot's document round-trips exactly: read, then written, it is byte-identical, and it
/// reads back by its envelope as itself. Its digests are of bytes no text could hold, the
/// owner's words need escaping, and a part's file keeps its case.
#[test]
fn a_snapshot_round_trips_exactly() {
    use duet_schemas::Snapshot;
    use duet_schemas::journal::TakeRef;
    use duet_schemas::snapshot::{Copied, FileDigest};
    let copied =
        |path: &str, bytes: &[u8]| Copied::new(path.parse().unwrap(), FileDigest::of(bytes));
    let mut body = Snapshot::new(
        "take-2".parse().unwrap(),
        "2028-02-29".parse().unwrap(),
        vec![
            copied("song.duet", &[0xff, 0xfe, 0x00]),
            copied("parts/Kit.DUET", b""),
        ],
    );
    body.said = Some("tab\there, \"quoted\", and é".into());
    body.takes = vec![TakeRef::new(
        "porch-2",
        "sha256:00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff"
            .parse()
            .unwrap(),
    )];
    let doc = Document::new(Tool::new("duet", "0.3.0"), body);
    let text = doc.write().unwrap();
    let back = Document::<Snapshot>::read(&text).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.write().unwrap(), text, "it writes back identical");
    let AnyDocument::Snapshot(any) = AnyDocument::read(&text).unwrap() else {
        panic!("not read as a snapshot");
    };
    assert_eq!(any, doc, "read by its envelope");

    // The committed example round-trips too.
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../duet-schemas/schemas/snapshot/examples/1.0.duet"
    ))
    .unwrap();
    let read = Document::<Snapshot>::read(&example).unwrap();
    assert_eq!(format!("{}\n", read.write().unwrap().trim_end()), example);
}

#[test]
fn a_takes_gaps_round_trip_exactly_through_json() {
    // capture-analysis 1.14: each span the decoder left out, its start and length in frames
    // at the take's own rate, reads back as written, and the document writes back as the text
    // it was read from. Absent, it stays absent: a take that decoded whole states no gaps.
    let text = analysis_example("1.14");
    let gaps_block = r#"
    "gaps": [
      {
        "start": 88200,
        "frames": 22050
      }
    ],"#;
    assert!(
        text.contains(gaps_block),
        "the example's gaps are not as written here"
    );
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&text).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), text);
    let gaps: Vec<(u64, u64)> = doc.body.gaps.iter().map(|g| (g.start, g.frames)).collect();
    assert_eq!(gaps, [(88_200, 22_050)]);
    let two = text.replace(
        gaps_block,
        r#"
    "gaps": [
      {
        "start": 88200,
        "frames": 22050
      },
      {
        "start": 441000,
        "frames": 4096
      }
    ],"#,
    );
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&two).unwrap() else {
        panic!("read as another kind");
    };
    assert_eq!(doc.write().unwrap(), two);
    let without = text.replace(gaps_block, "");
    let AnyDocument::CaptureAnalysis(doc) = AnyDocument::read(&without).unwrap() else {
        panic!("read as another kind");
    };
    assert!(doc.body.gaps.is_empty());
    assert_eq!(doc.write().unwrap(), without);
}
