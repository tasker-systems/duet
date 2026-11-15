//! The note HMM's witnesses: the trap (a held, wobbly note is one note, at `c` and at `c/2`; a
//! semitone step that holds is two) and the owner's three rulings (a note is held pitch, a slide
//! belongs to the note it arrives at, a dip is two notes).

use std::f64::consts::TAU;

use super::*;
use crate::performance::{FrameBlock, Layer1, Performance, PerformanceHead};
use crate::reading::{A440_CENTS, ReaderName, read};
use crate::stage::{Mono, Stage};
use crate::synth;

const RATE: u32 = 22_050;
/// The key the witnesses are sung on: E4.
const KEY: u8 = 64;

fn hz(key: u8) -> f64 {
    440.0 * ((f64::from(key) - 69.0) / 12.0).exp2()
}

/// `body` with a quarter second of silence either side.
fn padded(body: Vec<f32>) -> Vec<f32> {
    [synth::silence(RATE, 0.25), body, synth::silence(RATE, 0.25)].concat()
}

/// A tone on [`KEY`] whose pitch is `cents(t)` from it for `secs`, padded with silence.
fn sung(secs: f64, cents: impl Fn(f64) -> f64) -> Vec<f32> {
    padded(synth::contour(RATE, hz(KEY), secs, cents, 4, 0.3))
}

/// The witness's wobble: ±50 cents of vibrato at 5.5 Hz, and a drift of 15 cents a second,
/// centred on the note's middle.
fn wobble(t: f64, secs: f64) -> f64 {
    50.0 * (TAU * 5.5 * t).sin() + 15.0 * (t - secs / 2.0)
}

/// The take's performance and its onsets, as `duet capture` keeps them.
fn performance(signal: &[f32]) -> (PerformanceFrames, Vec<Event>) {
    let mut stage = Performance::new(RATE);
    let mut out = Vec::new();
    for (i, piece) in signal.chunks(4096).enumerate() {
        let mono = Mono {
            rate: RATE,
            start_frame: (i * 4096) as u64,
            samples: piece.to_vec(),
        };
        stage.push(mono, &mut out).unwrap();
    }
    stage.finish(&mut out).unwrap();
    let mut frames = None;
    let mut onsets = Vec::new();
    for layer in out {
        match layer {
            Layer1::Head(head) => frames = Some(PerformanceFrames::new(&head)),
            Layer1::Frames(block) => frames.as_mut().unwrap().push(&block).unwrap(),
            Layer1::Onset { frame, strength } => onsets.push(Event::Onset { frame, strength }),
        }
    }
    (frames.unwrap(), onsets)
}

/// The take read by the HMM at `note_cost`, on A440 (the witnesses are sung on it).
fn hmm(signal: &[f32], note_cost: f64) -> Reading {
    let (frames, onsets) = performance(signal);
    NoteModel::with_cost(note_cost).read(&frames, &onsets, Some(A440_CENTS))
}

fn keys(reading: &Reading) -> Vec<u8> {
    reading.notes.iter().map(|n| n.key).collect()
}

fn secs(frame: u64) -> f64 {
    frame as f64 / f64::from(RATE)
}

#[test]
fn a_held_note_with_vibrato_and_drift_is_one_note_at_c_and_at_half_c() {
    let signal = sung(2.0, |t| wobble(t, 2.0));
    for cost in [NOTE_COST, NOTE_COST / 2.0] {
        let reading = hmm(&signal, cost);
        assert_eq!(keys(&reading), [KEY], "c = {cost}: {:?}", reading.notes);
        let note = reading.notes[0];
        assert!((secs(note.start) - 0.25).abs() < 0.03, "{note:?}");
        assert!((secs(note.end) - 2.25).abs() < 0.05, "{note:?}");
        assert!(note.centre.abs() < 15.0, "{note:?}");
        assert_eq!(
            reading.reader,
            Some(Reader::hmm(cost)),
            "the reading states its reader and its cost"
        );
        assert_eq!(reading.residual.unwrap().loss.notes, cost);
    }
}

#[test]
fn the_same_note_stepped_a_semitone_halfway_is_two_legato_notes() {
    // Held 0.5 s, then a semitone up for 0.5 s, the wobble going on through the step, with no
    // breath and no attack: one unbroken tone.
    let step = |t: f64| wobble(t, 1.0) + if t < 0.5 { 0.0 } else { 100.0 };
    let signal = sung(1.0, step);
    for cost in [NOTE_COST, NOTE_COST / 2.0] {
        let reading = hmm(&signal, cost);
        assert_eq!(
            keys(&reading),
            [KEY, KEY + 1],
            "c = {cost}: {:?}",
            reading.notes
        );
        let (first, second) = (reading.notes[0], reading.notes[1]);
        assert_eq!(first.end, second.start, "legato: no silence between them");
        assert!((secs(second.start) - 0.75).abs() < 0.06, "{second:?}");
    }
}

#[test]
fn a_scoop_into_a_note_belongs_to_the_note_it_arrives_at() {
    // 300 cents below, rising in a straight line over 150 ms, then held for 0.6 s.
    let scoop = |t: f64| -300.0 * (1.0 - t / 0.15).max(0.0);
    let reading = hmm(&sung(0.75, scoop), NOTE_COST);
    assert_eq!(keys(&reading), [KEY], "{:?}", reading.notes);
    let note = reading.notes[0];
    // The note starts where the voice comes in, its slide included, and is held from where
    // the slide arrives.
    assert!((secs(note.start) - 0.25).abs() < 0.03, "{note:?}");
    let held = secs(note.held_from) - 0.25;
    assert!((0.08..0.2).contains(&held), "held from {held} s: {note:?}");
    let scoop = note
        .residual
        .unwrap()
        .scoop
        .expect("the scoop is the note's");
    assert!(scoop.cents < -200.0, "{scoop:?}");
}

#[test]
fn a_slide_between_two_notes_belongs_to_the_one_it_arrives_at() {
    // Held 0.5 s, a glide of a fourth up over 0.15 s, then held 0.5 s, all legato.
    let glide = |t: f64| 500.0 * ((t - 0.5) / 0.15).clamp(0.0, 1.0);
    let reading = hmm(&sung(1.15, glide), NOTE_COST);
    assert_eq!(keys(&reading), [KEY, KEY + 5], "{:?}", reading.notes);
    let (first, second) = (reading.notes[0], reading.notes[1]);
    assert_eq!(first.end, second.start);
    // The boundary is where the glide leaves the first note, in its first half (before 0.825 s
    // into the take), and the second is held only once the glide has arrived.
    let boundary = secs(second.start) - 0.25;
    assert!((0.45..0.6).contains(&boundary), "boundary at {boundary} s");
    let held = secs(second.held_from) - 0.25;
    assert!(
        (0.6..0.72).contains(&held),
        "held from {held} s: {second:?}"
    );
}

#[test]
fn a_dip_that_leaves_a_pitch_and_comes_back_is_two_notes() {
    // Held 0.5 s, a V-shaped dip two semitones down and back over 0.16 s, then held 0.5 s,
    // voiced throughout: a voiced consonant between two syllables.
    let dip = |t: f64| {
        let into = (t - 0.5) / 0.08;
        if (0.0..2.0).contains(&into) {
            -200.0 * (1.0 - (into - 1.0).abs())
        } else {
            0.0
        }
    };
    let reading = hmm(&sung(1.16, dip), NOTE_COST);
    assert_eq!(keys(&reading), [KEY, KEY], "{:?}", reading.notes);
    let (first, second) = (reading.notes[0], reading.notes[1]);
    assert_eq!(first.end, second.start, "legato through the dip");
    // The dip is the second note's slide: no note at its bottom.
    assert!(second.held_from > second.start, "{second:?}");
}

#[test]
fn a_repeated_pitch_splits_at_its_onsets() {
    let tones = [(hz(KEY), 0.4), (hz(KEY), 0.4), (hz(KEY), 0.4)];
    let reading = hmm(
        &padded(synth::note_sequence(RATE, &tones, 4, 0.3)),
        NOTE_COST,
    );
    assert_eq!(keys(&reading), [KEY, KEY, KEY], "{:?}", reading.notes);
}

#[test]
fn splitting_a_held_note_in_two_raises_the_loss() {
    // The trap's second witness: c is never zero in effect, so one note per frame is never the
    // least loss.
    let (frames, onsets) = performance(&sung(2.0, |t| wobble(t, 2.0)));
    let reading = NoteModel::default().read(&frames, &onsets, Some(A440_CENTS));
    let note = reading.notes[0];
    let middle = (note.held_from + note.end) / 2;
    let (mut a, mut b) = (note, note);
    a.end = middle;
    b.start = middle;
    b.held_from = middle;
    let (_, split) = residual_with_cost(&frames, &reading.tuning, &[a, b], NOTE_COST);
    let whole = reading.residual.unwrap().loss;
    assert!(split.loss.total() > whole.total(), "{split:?} {whole:?}");
    assert_eq!(split.loss.pitch, whole.pitch);
}

#[test]
fn the_fixed_lag_decodes_as_the_whole_take_does() {
    let signals = [
        sung(2.0, |t| wobble(t, 2.0)),
        sung(1.0, |t| wobble(t, 1.0) + if t < 0.5 { 0.0 } else { 100.0 }),
        sung(1.15, |t| 500.0 * ((t - 0.5) / 0.15).clamp(0.0, 1.0)),
    ];
    for signal in &signals {
        let (frames, onsets) = performance(signal);
        let model = NoteModel::default();
        let lagged = model.read(&frames, &onsets, Some(A440_CENTS));
        let whole = model.read_with_lag(&frames, &onsets, Some(A440_CENTS), frames.len());
        assert_eq!(lagged.notes, whole.notes);
    }
}

#[test]
fn a_dropped_frame_stays_inside_its_note_and_noise_is_no_note() {
    // Frames built by hand: a held pitch with one frame the tracker lost, then voiced frames
    // too weakly voiced to be singing.
    let head = PerformanceHead::new(RATE, 256, 0);
    let mut frames = PerformanceFrames::new(&head);
    let cents = (A440_CENTS + 100.0 * (f64::from(KEY) - 69.0)) as f32;
    let mut pitch: Vec<Option<f32>> = vec![None; 20];
    pitch.extend(vec![Some(cents); 40]);
    pitch[40] = None;
    pitch.extend(vec![None; 20]);
    pitch.extend((0..40).map(|k| Some(cents + 37.0 * (k as f32 * 1.7).sin())));
    let n = pitch.len();
    let mut voicing = vec![0.9; n];
    voicing[80..].fill(0.05);
    frames
        .push(&FrameBlock::new(0, pitch, voicing, vec![0.1; n]).unwrap())
        .unwrap();
    let reading = NoteModel::default().read(&frames, &[], Some(A440_CENTS));
    assert_eq!(keys(&reading), [KEY], "{:?}", reading.notes);
}

#[test]
fn the_rules_state_they_are_the_rules() {
    let (frames, onsets) = performance(&sung(0.5, |_| 0.0));
    let reading = read(&frames, &onsets, None);
    assert_eq!(reading.reader.map(|r| r.name), Some(ReaderName::Rules));
}

/// The takes both readers are judged on here: the witnesses, and a melody sung in tune, 44
/// cents sharp and 40 flat, each note a little off the take's tuning as a singer is.
fn takes() -> Vec<(&'static str, Vec<f32>)> {
    const TUNE: [(u8, f64); 9] = [
        (60, 0.4),
        (62, 0.3),
        (64, 0.5),
        (65, 0.3),
        (67, 0.6),
        (69, 0.3),
        (67, 0.4),
        (64, 0.4),
        (60, 0.7),
    ];
    let scatter = [0.0, 12.0, -12.0, 12.0, 0.0, -12.0, 12.0, 0.0, -12.0];
    let tune = |cents: f64| {
        let tones: Vec<(f64, f64)> = TUNE
            .iter()
            .zip(scatter)
            .map(|(&(k, s), off)| (hz(k) * ((cents + off) / 1200.0).exp2(), s))
            .collect();
        padded(synth::note_sequence(RATE, &tones, 4, 0.3))
    };
    let legato: Vec<(f64, f64)> = TUNE.iter().map(|&(k, s)| (hz(k), s)).collect();
    vec![
        ("vibrato+drift 2s", sung(2.0, |t| wobble(t, 2.0))),
        (
            "semitone step",
            sung(1.0, |t| wobble(t, 1.0) + if t < 0.5 { 0.0 } else { 100.0 }),
        ),
        (
            "scoop 300c",
            sung(0.75, |t| -300.0 * (1.0 - t / 0.15).max(0.0)),
        ),
        (
            "glide a fourth",
            sung(1.15, |t| 500.0 * ((t - 0.5) / 0.15).clamp(0.0, 1.0)),
        ),
        (
            "voiced dip",
            sung(1.16, |t| {
                let into = (t - 0.5) / 0.08;
                if (0.0..2.0).contains(&into) {
                    -200.0 * (1.0 - (into - 1.0).abs())
                } else {
                    0.0
                }
            }),
        ),
        (
            "repeated pitch",
            padded(synth::note_sequence(RATE, &[(hz(KEY), 0.4); 3], 4, 0.3)),
        ),
        ("tune in tune", tune(0.0)),
        ("tune +44c", tune(44.0)),
        ("tune -40c", tune(-40.0)),
        (
            "tune legato",
            padded(synth::legato(RATE, &legato, 0.06, 4, 0.3)),
        ),
    ]
}

#[test]
fn the_loss_agrees_with_the_decoder_that_each_note_it_reads_is_worth_reading() {
    // The decoder and the loss price the same frames the same way, so a note the decoder
    // chose over leaving its frames unread is one the loss scores better read too.
    for (name, signal) in takes() {
        let (frames, onsets) = performance(&signal);
        let reading = NoteModel::default().read(&frames, &onsets, Some(A440_CENTS));
        let whole = reading.residual.as_ref().unwrap().loss;
        for i in 0..reading.notes.len() {
            let mut fewer = reading.notes.clone();
            fewer.remove(i);
            let (_, less) = residual_with_cost(&frames, &reading.tuning, &fewer, NOTE_COST);
            assert!(
                less.loss.total() > whole.total(),
                "{name}: without note {i}, {:?} against {whole:?}",
                less.loss
            );
        }
    }
}

/// The per-take report: each reader's notes and its loss's terms, and the HMM's note
/// count across costs per note. Run it with `cargo test -p duet-dsp hmm::tests::report --
/// --ignored --nocapture`.
#[test]
#[ignore = "a report, not a witness: run with --ignored --nocapture"]
fn report() {
    let costs = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0];
    eprintln!(
        "| take | reader | notes | keys | unread frames | pitch | voicing | unread | notes term | \
         total |"
    );
    eprintln!("|---|---|---|---|---|---|---|---|---|---|");
    let mut sweep = Vec::new();
    for (name, signal) in takes() {
        let (frames, onsets) = performance(&signal);
        let rules = read(&frames, &onsets, None);
        let hmm = NoteModel::default().read(&frames, &onsets, None);
        for (reader, reading) in [("rules", &rules), ("hmm", &hmm)] {
            let take = reading.residual.as_ref().unwrap();
            let loss = take.loss;
            eprintln!(
                "| {name} | {reader} | {} | {:?} | {} | {:.1} | {:.1} | {:.1} | {:.0} | {:.1} |",
                reading.notes.len(),
                keys(reading),
                take.unread_frames,
                loss.pitch,
                loss.voicing,
                loss.unread,
                loss.notes,
                loss.total()
            );
        }
        let counts: Vec<String> = costs
            .iter()
            .map(|&c| {
                NoteModel::with_cost(c)
                    .read(&frames, &onsets, None)
                    .notes
                    .len()
                    .to_string()
            })
            .collect();
        sweep.push(format!("| {name} | {} |", counts.join(" | ")));
    }
    eprintln!();
    eprintln!("| take | {} |", costs.map(|c| format!("c={c}")).join(" | "));
    eprintln!("|---|{}", "---|".repeat(costs.len()));
    for line in sweep {
        eprintln!("{line}");
    }
}

/// What a hand-built frame is: unvoiced, or voiced at `cents` from [`KEY`] on A440 with
/// `voicing`.
#[derive(Clone, Copy)]
enum Hand {
    Unvoiced,
    Voiced { cents: f32, voicing: f32 },
}

/// Frames at the analysis hop, built by hand from runs of `(count, frame)`; a run's frames may
/// vary with their index in it through `frame`.
fn by_hand(runs: &[(usize, &dyn Fn(usize) -> Hand)]) -> PerformanceFrames {
    let head = PerformanceHead::new(RATE, 256, 0);
    let mut frames = PerformanceFrames::new(&head);
    let on_key = (A440_CENTS + 100.0 * (f64::from(KEY) - 69.0)) as f32;
    let (mut pitch, mut voicing) = (Vec::new(), Vec::new());
    for &(count, frame) in runs {
        for i in 0..count {
            match frame(i) {
                Hand::Unvoiced => {
                    pitch.push(None);
                    voicing.push(0.0);
                }
                Hand::Voiced { cents, voicing: v } => {
                    pitch.push(Some(on_key + cents));
                    voicing.push(v);
                }
            }
        }
    }
    let n = pitch.len();
    frames
        .push(&FrameBlock::new(0, pitch, voicing, vec![0.1; n]).unwrap())
        .unwrap();
    frames
}

/// Frames at the analysis rate in `secs`.
fn hops(secs: f64) -> usize {
    (secs * f64::from(RATE) / 256.0).round() as usize
}

const UNVOICED: &dyn Fn(usize) -> Hand = &|_| Hand::Unvoiced;
/// A voiced consonant: weakly voiced, its pitch anywhere.
const CONSONANT: &dyn Fn(usize) -> Hand = &|i| Hand::Voiced {
    cents: [-310.0, 240.0, -180.0, 90.0, -260.0][i % 5],
    voicing: 0.3,
};
/// A breath: barely voiced, its pitch anywhere near the note.
const BREATH: &dyn Fn(usize) -> Hand = &|i| Hand::Voiced {
    cents: [-40.0, 25.0, -15.0, 35.0][i % 4],
    voicing: 0.02,
};

#[test]
fn a_note_held_45_cents_off_its_key_between_consonants_is_read() {
    // 0.3 s held 45 cents sharp of E4, strongly voiced, with a voiced consonant either side:
    // an attack that never holds would leave it all unread.
    let held: &dyn Fn(usize) -> Hand = &|_| Hand::Voiced {
        cents: 45.0,
        voicing: 0.95,
    };
    let frames = by_hand(&[
        (20, UNVOICED),
        (4, CONSONANT),
        (hops(0.3), held),
        (4, CONSONANT),
        (20, UNVOICED),
    ]);
    let reading = NoteModel::default().read(&frames, &[], Some(A440_CENTS));
    assert_eq!(keys(&reading), [KEY], "{:?}", reading.notes);
    let take = reading.residual.unwrap();
    assert!(
        take.unread_frames <= 8,
        "only the consonants are left unread: {take:?}"
    );
}

#[test]
fn a_short_note_between_two_breaths_is_read() {
    // 0.15 s on its key (which main already read: on its key, nine frames pay a note's cost
    // against an attack's), 0.08 s on its key, and 0.15 s held 33 cents flat: each strongly
    // voiced, with a breath either side.
    for (secs, cents) in [(0.15, 0.0), (0.08, 0.0), (0.15, -33.0)] {
        let held = move |_| Hand::Voiced {
            cents,
            voicing: 0.95,
        };
        let frames = by_hand(&[
            (20, UNVOICED),
            (8, BREATH),
            (hops(secs), &held),
            (8, BREATH),
            (20, UNVOICED),
        ]);
        let reading = NoteModel::default().read(&frames, &[], Some(A440_CENTS));
        assert_eq!(
            keys(&reading),
            [KEY],
            "{secs} s at {cents} cents: {:?}",
            reading.notes
        );
    }
}

#[test]
fn a_breath_is_left_unread() {
    let frames = by_hand(&[(20, UNVOICED), (hops(0.3), BREATH), (20, UNVOICED)]);
    let reading = NoteModel::default().read(&frames, &[], Some(A440_CENTS));
    assert!(reading.notes.is_empty(), "{:?}", reading.notes);
}

#[test]
fn the_slide_into_a_wobble_the_rules_drop_is_read() {
    // The frames of the rules' stated limit (`notes::wobbling_slide`), on KEY: a held note
    // three semitones above, a weakly voiced glide down, a scoop, and a fast wobble. The rules
    // read only the held note; the HMM reads the wobbled note as well.
    let on_key = (A440_CENTS + 100.0 * (f64::from(KEY) - 69.0)) as f32;
    let (pitch, voicing): (Vec<Option<f32>>, Vec<f32>) = crate::notes::wobbling_slide()
        .into_iter()
        .map(|(semitones, voicing)| (semitones.map(|s| on_key + 100.0 * s), voicing))
        .unzip();
    let n = pitch.len();
    let mut frames = PerformanceFrames::new(&PerformanceHead::new(RATE, 256, 0));
    frames
        .push(&FrameBlock::new(0, pitch, voicing, vec![0.1; n]).unwrap())
        .unwrap();
    let rules = read(&frames, &[], Some(A440_CENTS));
    assert_eq!(keys(&rules), [KEY + 3], "{:?}", rules.notes);
    // The wobble is read on its key. How many notes it is read as (the voicing dip mid-note
    // can split it) is not what this records.
    let reading = NoteModel::default().read(&frames, &[], Some(A440_CENTS));
    let read = keys(&reading);
    assert!(
        read.len() >= 2 && read[0] == KEY + 3 && read[1..].iter().all(|&k| k == KEY),
        "{:?}",
        reading.notes
    );
}
