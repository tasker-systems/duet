//! What CI can witness of `duet play`: the song → timed-events schedule, an offline render
//! through a SoundFont synth, and the timed sends a MIDI port would get, on a fake clock.
//! Real-time sound and a real port are not covered here; only a local run shows them.

use std::time::Duration;

use duet_core::Song;
use duet_midi::Midi;
use duet_play::{Renderer, Schedule, TAIL, load_soundfont, render, sample_at, send_timed};

const RATE: u32 = 44_100;

/// A SoundFont built in memory: one preset, 0:0, playing a looped sine wave at A4 on
/// every key. No soundfont file is committed; this is just enough of the format for the
/// synth to sound a note where the schedule says.
fn sine_font() -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }
    fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for c in chunks {
            body.extend_from_slice(c);
        }
        chunk(b"LIST", &body)
    }
    fn name(s: &str) -> Vec<u8> {
        let mut out = s.as_bytes().to_vec();
        out.resize(20, 0);
        out
    }
    let u16s = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();

    // 441 Hz at 44100 is exactly 100 samples a period, so the loop is seamless.
    let samples: Vec<i16> = (0..1000)
        .map(|i| ((i as f64 * std::f64::consts::TAU / 100.0).sin() * 12_000.0) as i16)
        .chain(std::iter::repeat_n(0, 46))
        .collect();
    let smpl: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    let preset = |n: &str, bag: u16| [name(n), u16s(&[0, 0, bag]), vec![0; 12]].concat();
    let inst = |n: &str, bag: u16| [name(n), u16s(&[bag])].concat();
    let mut shdr = name("Sine");
    for v in [0u32, 1000, 100, 900, RATE] {
        shdr.extend_from_slice(&v.to_le_bytes());
    }
    shdr.extend_from_slice(&[69, 0]);
    shdr.extend_from_slice(&u16s(&[0, 1]));
    shdr.extend_from_slice(&[name("EOS"), vec![0; 26]].concat());

    let info = list(
        b"INFO",
        &[
            chunk(b"ifil", &u16s(&[2, 1])),
            chunk(b"isng", b"EMU8000\0"),
            chunk(b"INAM", b"duet test\0"),
        ],
    );
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &[preset("Sine", 0), preset("EOP", 1)].concat()),
            chunk(b"pbag", &u16s(&[0, 0, 1, 0])),
            chunk(b"pmod", &[0; 10]),
            // instrument 0, then the terminator
            chunk(b"pgen", &u16s(&[41, 0, 0, 0])),
            chunk(b"inst", &[inst("Sine", 0), inst("EOI", 1)].concat()),
            chunk(b"ibag", &u16s(&[0, 0, 2, 0])),
            chunk(b"imod", &[0; 10]),
            // sampleModes 1 (loop), sampleID 0, then the terminator
            chunk(b"igen", &u16s(&[54, 1, 53, 0, 0, 0])),
            chunk(b"shdr", &shdr),
        ],
    );
    chunk(b"RIFF", &[b"sfbk".to_vec(), info, sdta, pdta].concat())
}

fn song(text: &str) -> Song {
    Song::from_text(text).expect("a song")
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// Every message is the one the song's MIDI holds, at its tick, with its time from the
/// tempo map: notes with pitch, channel and velocity, programs, controls and bends.
#[test]
fn the_schedule_holds_every_event_at_its_time() {
    let s = song(
        "tempo 120\ntime 4/4\nend 3.1.0\n\n\
         [track \"Keys\" ch1 prog5]\n1.1.0 cc64 127\n1.2.0 C4 1 80\n\n\
         [track \"Bass\" ch2 prog33]\n1.1.0 E2 0.5 100\n2.1.0 bend 4096\n",
    );
    let schedule = Schedule::from_song(&s).expect("schedule");
    let got: Vec<(f64, Vec<u8>)> = schedule
        .events()
        .iter()
        .map(|e| (secs(e.at), e.message.bytes().to_vec()))
        .collect();
    let bend = 4096 + 8192;
    assert_eq!(
        got,
        vec![
            (0.0, vec![0xC0, 5]),
            (0.0, vec![0xB0, 64, 127]),
            (0.0, vec![0xC1, 33]),
            (0.0, vec![0x91, 40, 100]),
            (0.25, vec![0x81, 40, 64]),
            (0.5, vec![0x90, 60, 80]),
            (1.0, vec![0x80, 60, 64]),
            (2.0, vec![0xE1, (bend & 0x7F) as u8, (bend >> 7) as u8]),
        ]
    );
    assert_eq!(secs(schedule.end()), 4.0);
}

/// A tempo change moves every later event, and earlier ones not at all.
#[test]
fn a_tempo_change_moves_what_follows_it() {
    let s = song(
        "tempo 120\ntime 4/4\nend 4.1.0\n\n[track \"P\" ch1]\n\
         1.4.0 C4 1 80\n3.1.0 D4 1 80\n\n",
    );
    let mut slower = s.clone();
    slower.tempo_changes.push(duet_core::TempoChange {
        at: duet_core::Position::new(2, 1, 0),
        bpm: 60.0,
    });
    let on = |song: &Song| -> Vec<f64> {
        Schedule::from_song(song)
            .expect("schedule")
            .events()
            .iter()
            .filter(|e| e.message.kind() == 0x90)
            .map(|e| secs(e.at))
            .collect()
    };
    assert_eq!(on(&s), [1.5, 4.0]);
    // Bar 1 at 120 (2 s), then bar 2 at 60 (4 s): bar 3 starts at 6 s.
    assert_eq!(on(&slower), [1.5, 6.0]);
    assert_eq!(secs(Schedule::from_song(&slower).expect("s").end()), 10.0);
}

/// What plays is what `duet convert` writes: the schedule of a song is the schedule of
/// its MIDI file.
#[test]
fn the_schedule_is_the_songs_own_midi() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/sketch.notes");
    let s = song(&std::fs::read_to_string(path).expect("the sample"));
    let from_song = Schedule::from_song(&s).expect("schedule");
    let from_midi = Schedule::from_midi(&s.to_midi().expect("midi")).expect("schedule");
    assert_eq!(from_song, from_midi);
    let notes = s.tracks.iter().map(|t| t.notes.len()).sum::<usize>();
    let ons = from_song
        .events()
        .iter()
        .filter(|e| e.message.kind() == 0x90)
        .count();
    assert_eq!(ons, notes, "a note on for every note");
}

/// The render runs to the song's end and its tail, is silent before the first note, and
/// sounds within 20 ms of the note's time (the synth works in 64-sample blocks).
#[test]
fn an_offline_render_sounds_each_note_where_it_falls() {
    let font = load_soundfont(&sine_font()).expect("the test font");
    let s = song("tempo 120\ntime 4/4\nend 2.1.0\n\n[track \"P\" ch1]\n1.3.0 A4 1 100\n");
    let schedule = Schedule::from_song(&s).expect("schedule");
    let (left, right) = render(&schedule, &font, RATE).expect("render");

    assert_eq!(left.len() as u64, sample_at(schedule.end() + TAIL, RATE));
    assert_eq!(left.len(), right.len());
    let onset = sample_at(Duration::from_secs(1), RATE) as usize;
    assert!(
        left[..onset].iter().all(|x| *x == 0.0),
        "silent before the note"
    );
    let heard = |from: usize, to: usize| left[from..to].iter().any(|x| x.abs() > 0.01);
    assert!(
        heard(onset, onset + RATE as usize / 50),
        "sounding within 20 ms"
    );
    assert!(
        !heard(left.len() - RATE as usize / 10, left.len()),
        "the release has died away by the end of the tail"
    );
}

/// Louder velocity, louder sound: the render follows the schedule's velocities.
#[test]
fn velocity_reaches_the_synth() {
    let font = load_soundfont(&sine_font()).expect("the test font");
    let peak = |vel: u8| {
        let s = song(&format!(
            "tempo 120\nend 2.1.0\n\n[track \"P\" ch1]\n1.1.0 A4 1 {vel}\n"
        ));
        let (left, _) = render(&Schedule::from_song(&s).expect("s"), &font, RATE).expect("r");
        left.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    };
    assert!(peak(120) > peak(30) * 2.0, "{} vs {}", peak(120), peak(30));
}

/// A device asks for buffers of its own size. Filling in uneven pieces gives the same
/// samples as one render, so each event lands on its own sample however buffers fall.
#[test]
fn the_render_does_not_depend_on_buffer_size() {
    let font = load_soundfont(&sine_font()).expect("the test font");
    let s = song(
        "tempo 97\ntime 3/4\nend 4.1.0\n\n[track \"P\" ch1]\n1.1.0 A4 0.5 90\n1.2.17 E5 0.7 70\n\
         2.3.200 A3 1 110\n",
    );
    let schedule = Schedule::from_song(&s).expect("schedule");
    let (whole, _) = render(&schedule, &font, RATE).expect("render");

    let mut renderer = Renderer::new(&schedule, &font, RATE).expect("renderer");
    let mut pieces = Vec::new();
    for size in [1usize, 7, 64, 333, 512, 4096].iter().cycle() {
        if renderer.is_done() {
            break;
        }
        let (mut l, mut r) = (vec![0.0; *size], vec![0.0; *size]);
        let n = renderer.fill(&mut l, &mut r);
        assert!(l[n..].iter().all(|x| *x == 0.0), "silence past the end");
        pieces.extend_from_slice(&l[..n]);
    }
    assert_eq!(pieces, whole);
}

/// The sends a port gets: every message's bytes, each sent once its time has come, then a
/// wait to the song's end.
#[test]
fn a_port_is_sent_each_message_at_its_time() {
    let s = song(
        "tempo 120\ntime 4/4\nend 2.1.0\n\n[track \"P\" ch3 prog7]\n1.2.0 C4 1 80\n\
         1.4.0 D4 0.5 60\n",
    );
    let schedule = Schedule::from_song(&s).expect("schedule");
    let clock = std::cell::Cell::new(Duration::ZERO);
    let mut sent: Vec<(Duration, Vec<u8>)> = Vec::new();
    send_timed::<()>(
        &schedule,
        || clock.get(),
        |d| clock.set(clock.get() + d),
        |bytes| {
            sent.push((clock.get(), bytes.to_vec()));
            Ok(())
        },
    )
    .expect("sent");
    let want: Vec<(Duration, Vec<u8>)> = schedule
        .events()
        .iter()
        .map(|e| (e.at, e.message.bytes().to_vec()))
        .collect();
    assert_eq!(sent, want);
    assert_eq!(sent[0].1, vec![0xC2, 7]);
    assert_eq!(clock.get(), schedule.end(), "waits to the end");
}

/// A send that fails stops the playing there, and the error comes back.
#[test]
fn a_failed_send_stops_playing() {
    let s = song("tempo 120\nend 2.1.0\n\n[track \"P\" ch1]\n1.1.0 C4 1 80\n1.2.0 D4 1 80\n");
    let schedule = Schedule::from_song(&s).expect("schedule");
    let mut count = 0;
    let got = send_timed(
        &schedule,
        || Duration::ZERO,
        |_| {},
        |_| {
            count += 1;
            if count == 2 { Err("gone") } else { Ok(()) }
        },
    );
    assert_eq!((got, count), (Err("gone"), 2));
}
