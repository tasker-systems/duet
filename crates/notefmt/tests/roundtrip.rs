//! Round-trip tests on generated MIDI files: MIDI to song, through the text, JSON and
//! TOML forms, and back to MIDI, checking every kept event survives to the tick.

use std::collections::{HashMap, VecDeque};

use midly::num::{u15, u24, u28, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};
use notefmt::Song;

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

fn check_round_trip(bytes: &[u8], expected_notes: &[RawNote]) {
    let original = events(bytes);
    assert_eq!(
        original.notes, expected_notes,
        "test file is built as intended"
    );

    let song = Song::from_midi(bytes).unwrap();

    // Text, JSON and TOML all give back the same song.
    let text = song.to_text();
    let from_text = Song::from_text(&text).unwrap();
    assert_eq!(from_text, song, "text round-trip; text was:\n{text}");
    assert_eq!(from_text.to_text(), text, "text is stable");
    let json = serde_json::to_string_pretty(&song).unwrap();
    assert_eq!(
        serde_json::from_str::<Song>(&json).unwrap(),
        song,
        "JSON round-trip"
    );
    let toml_text = toml::to_string(&song).unwrap();
    assert_eq!(
        toml::from_str::<Song>(&toml_text).unwrap(),
        song,
        "TOML round-trip"
    );

    // Writing MIDI keeps every note, control and meta event to the tick.
    let written = from_text.to_midi().unwrap();
    let rewritten = events(&written);
    assert_eq!(rewritten.notes, original.notes);
    assert_eq!(rewritten.controls, original.controls);
    assert_eq!(rewritten.tempos, original.tempos);
    assert_eq!(rewritten.sigs, original.sigs);
    assert_eq!(rewritten.keys, original.keys);

    // And reading it again gives the same song.
    assert_eq!(Song::from_midi(&written).unwrap(), song);
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
    let song = Song::from_midi(&bytes).unwrap();
    assert_eq!(song.title.as_deref(), Some("Sketch in D"));
    assert_eq!(song.tempo_bpm, 92.0);
    assert_eq!(song.time_sig.to_string(), "4/4");
    assert_eq!(song.key.unwrap().to_string(), "D minor");
    let changes: Vec<String> = song
        .time_changes
        .iter()
        .map(|c| format!("{} at {}", c.time_sig, c.at))
        .collect();
    // 6/8 at bar 5; 3/4 halfway through bar 6 (the fourth eighth), so bar 7 starts there.
    assert_eq!(changes, ["6/8 at 5.1.0", "3/4 at 6.4.0"]);
    assert_eq!(song.tempo_changes[0].at.to_string(), "6.1.0");
    assert_eq!(song.tempo_changes[0].bpm, 100.0);

    let names: Vec<(&str, u8)> = song
        .tracks
        .iter()
        .map(|t| (t.name.as_str(), t.channel))
        .collect();
    assert_eq!(names, [("Piano", 1), ("Rhythm", 2), ("Rhythm", 10)]);
    let piano = &song.tracks[0];
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
    assert_eq!(song.tracks[1].program, Some(33));
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
    let song = Song::from_text(text).unwrap();
    assert_eq!(song.ticks_per_quarter, 480);
    let lead = &song.tracks[0];
    assert_eq!((lead.channel, lead.program), (3, Some(73)));
    assert_eq!(lead.notes.len(), 5);
    assert_eq!(lead.controls.len(), 2);

    let back = Song::from_midi(&song.to_midi().unwrap()).unwrap();
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
    let err = Song::from_text(text).unwrap_err().to_string();
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
        match Song::from_text(bad) {
            Err(notefmt::Error::Parse { line: l, .. }) => assert_eq!(l, line, "{bad}"),
            other => panic!("{bad}: expected a parse error, got {other:?}"),
        }
    }
}

#[test]
fn sample_sketch_round_trips() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/sketch.txt");
    let text = std::fs::read_to_string(path).unwrap();
    let song = Song::from_text(&text).unwrap();
    let back = Song::from_midi(&song.to_midi().unwrap()).unwrap();
    assert_eq!(back, song);
    assert_eq!(back.to_text(), text, "the sample is in canonical form");
}

#[test]
fn track_names_with_brackets_round_trip() {
    let names = ["Pad [verb]", "]", "[a] \"b\" ]c[", "x\\]"];
    let mut song = Song::default();
    for (i, name) in names.iter().enumerate() {
        let mut track = notefmt::Track::new(*name, i as u8 + 1);
        track.notes.push(notefmt::Note {
            at: "1.1.0".parse().unwrap(),
            pitch: "C4".parse().unwrap(),
            beats: 1.0,
            vel: 80,
        });
        song.tracks.push(track);
    }
    let from_midi = Song::from_midi(&song.to_midi().unwrap()).unwrap();
    let text = from_midi.to_text();
    let back = Song::from_text(&text).unwrap();
    assert_eq!(back, from_midi, "{text}");
    let got: Vec<&str> = back.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(got, names);

    let track = &Song::from_text("[track \"Pad [verb]\" ch2 prog5]  # strings\n")
        .unwrap()
        .tracks[0];
    assert_eq!((track.name.as_str(), track.channel), ("Pad [verb]", 2));
    assert_eq!(track.program, Some(5));

    for bad in ["[track \"Pad [verb]\" ch1", "[track \"Pad ch1]"] {
        match Song::from_text(bad) {
            Err(notefmt::Error::Parse { line: 1, .. }) => {}
            other => panic!("{bad}: expected a parse error on line 1, got {other:?}"),
        }
    }
}

fn notes_of(song: &Song) -> Vec<String> {
    song.tracks[0]
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
        let song = Song::from_text(&text).unwrap();
        let back = Song::from_midi(&song.to_midi().unwrap()).unwrap();
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
    let song = Song::from_text(text).unwrap();
    let back = Song::from_midi(&song.to_midi().unwrap()).unwrap();
    assert_eq!(notes_of(&back), notes_of(&song));

    // A note inside a longer note of the same pitch would come back changed, so writing
    // MIDI refuses it and names both notes.
    for inner in ["1.2.0 C4 0.5 100", "1.2.0 C4 0 100", "1.4.0 C4 0.5 100"] {
        let text = format!("[track \"P\" ch1]\n1.1.0 C4 4 80\n{inner}\n1.2.0 D4 0.5 70\n");
        let song = Song::from_text(&text).unwrap();
        let err = song.to_midi().unwrap_err().to_string();
        assert!(
            err.contains("C4 at 1.2.0") || err.contains("C4 at 1.4.0"),
            "{err}"
        );
        assert!(err.contains("inside the C4 at 1.1.0"), "{err}");
    }
    // The same notes on different channels are separate notes and are fine.
    let text = "[track \"A\" ch1]\n1.1.0 C4 4 80\n[track \"B\" ch2]\n1.2.0 C4 0.5 100\n";
    assert!(Song::from_text(text).unwrap().to_midi().is_ok());
}
