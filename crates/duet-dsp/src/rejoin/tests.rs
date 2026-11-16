//! The same-key rejoin's witnesses (the 2026-11-16 arrivals and ornaments design, task B): a
//! dip within a whole tone with no onset is rejoined, under either reader; a split at an onset,
//! a dip deeper than a whole tone and a step between two keys stay as decoded.

use super::*;
use crate::hmm::NoteModel;
use crate::notes::{dip_between_syllables, slower_dip};
use crate::performance::{FrameBlock, Layer1, Performance, PerformanceHead};
use crate::reading::{A440_CENTS, read, read_as_decoded};
use crate::stage::{Mono, Stage};
use crate::synth;

const RATE: u32 = 22_050;
const HOP: u64 = 256;
/// D#3, where "Wayfaring Stranger" dips at 9.54 s.
const D_SHARP_3: u8 = 51;
/// F#3 and G#3, the keys of the arrival and ornament witnesses.
const F_SHARP_3: u8 = 54;
const G_SHARP_3: u8 = 56;

fn hz(key: u8) -> f64 {
    440.0 * ((f64::from(key) - 69.0) / 12.0).exp2()
}

/// A tone on `key` whose pitch is `cents(t)` from it for `secs`, its phase unbroken so nothing
/// in it is attacked afresh, with a quarter second of silence either side.
fn sung(key: u8, secs: f64, cents: impl Fn(f64) -> f64) -> Vec<f32> {
    [
        synth::silence(RATE, 0.25),
        synth::contour(RATE, hz(key), secs, cents, 4, 0.3),
        synth::silence(RATE, 0.25),
    ]
    .concat()
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

/// Which reader, and whether the pass is run after it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum By {
    Rules,
    Hmm,
}

/// The take read by `by` on A440 (the witnesses are sung on it), before the pass and after it.
fn readings(by: By, frames: &PerformanceFrames, onsets: &[Event]) -> (Reading, Reading) {
    let given = Some(A440_CENTS);
    match by {
        By::Rules => (
            read_as_decoded(frames, onsets, given),
            read(frames, onsets, given),
        ),
        By::Hmm => (
            NoteModel::default().read_as_decoded(frames, onsets, given),
            NoteModel::default().read(frames, onsets, given),
        ),
    }
}

fn keys(reading: &Reading) -> Vec<u8> {
    reading.notes.iter().map(|n| n.key).collect()
}

/// A V-shaped dip `cents` deep (below zero for a dip, above for a rise) over `secs`, starting
/// at `at` seconds into the note.
fn v(at: f64, secs: f64, cents: f64) -> impl Fn(f64) -> f64 {
    move |t: f64| {
        let into = (t - at) / (secs / 2.0);
        if (0.0..2.0).contains(&into) {
            cents * (1.0 - (into - 1.0).abs())
        } else {
            0.0
        }
    }
}

/// A held D#3 that dips 170 cents and back over 0.1 s, half a second in, with no onset: the
/// shape of "Wayfaring Stranger" at 9.54 s.
fn d_sharp_dip() -> Vec<f32> {
    sung(D_SHARP_3, 1.1, v(0.5, 0.1, -170.0))
}

#[test]
fn a_dip_of_170_cents_with_no_onset_is_rejoined_into_one_note() {
    let (frames, onsets) = performance(&d_sharp_dip());
    // On main the HMM reads two notes.
    let (decoded, rejoined) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(
        keys(&decoded),
        [D_SHARP_3, D_SHARP_3],
        "{:?}",
        decoded.notes
    );
    assert_eq!(keys(&rejoined), [D_SHARP_3], "{:?}", rejoined.notes);
    let (first, second) = (&decoded.notes[0], &decoded.notes[1]);
    let one = &rejoined.notes[0];
    assert_eq!(
        (one.start, one.held_from, one.end),
        (first.start, first.held_from, second.end)
    );
    // The rejoin is stated, with the notes as decoded, the dip between them and what it does to
    // the loss.
    assert_eq!(rejoined.rejoins.len(), 1, "{:?}", rejoined.rejoins);
    let stated = &rejoined.rejoins[0];
    let bare = |n: &ReadNote| ReadNote {
        residual: None,
        ..n.clone()
    };
    assert_eq!(stated.decoded, [bare(first), bare(second)]);
    let dip = stated.dips[0];
    assert_eq!((dip.start, dip.end), (first.end, second.held_from));
    assert!((-180.0..=-140.0).contains(&dip.cents), "{dip:?}");
    // The dip is the note's one excursion, over the dip.
    let part = one.residual.as_ref().unwrap();
    assert_eq!(part.excursions.len(), 1, "{part:?}");
    let excursion = part.excursions[0];
    assert!(
        excursion.start < dip.end && excursion.end > dip.start,
        "{excursion:?} {dip:?}"
    );
    assert!((-170.0..=-60.0).contains(&excursion.cents), "{excursion:?}");
    let (before, after) = (
        decoded.residual.as_ref().unwrap().loss.total(),
        rejoined.residual.as_ref().unwrap().loss.total(),
    );
    assert!(
        (after - before - stated.loss_change).abs() < 1e-9,
        "{before} {after} {stated:?}"
    );
    // And the rules, whatever they decode, read one note after the pass.
    let (_, rules) = readings(By::Rules, &frames, &onsets);
    assert_eq!(keys(&rules), [D_SHARP_3], "{:?}", rules.notes);
}

#[test]
fn the_dips_frames_are_priced_as_held_and_a_rejoin_that_raises_the_loss_says_so() {
    let (frames, onsets) = performance(&d_sharp_dip());
    let (decoded, rejoined) = readings(By::Hmm, &frames, &onsets);
    let stated = &rejoined.rejoins[0];
    // The decoded notes, and the rejoined one, scored by the residual: one note's cost fewer,
    // and every frame of the dip held, priced by its pitch, where the decoder took it as the
    // second note's attack at a flat cost.
    let loss = |notes: &[ReadNote]| crate::residual(&frames, &decoded.tuning, notes).1.loss;
    let (two, one) = (loss(&decoded.notes), loss(&rejoined.notes));
    assert!((two.notes - one.notes - NOTE_COST).abs() < 1e-9);
    assert_eq!((two.voicing, two.unread), (one.voicing, one.unread));
    let tuning = decoded.tuning.a4.value();
    let key = tuning + 100.0 * (f64::from(D_SHARP_3) - 69.0);
    let second = &decoded.notes[1];
    let dip: Vec<f64> = (0..frames.len())
        .filter(|&k| (second.start..second.held_from).contains(&frames.frame(k)))
        .map(|k| f64::from(frames.cents[k]) - key)
        .collect();
    let held: f64 = dip.iter().map(|c| crate::held_cost(c / 100.0)).sum();
    let attack = dip.len() as f64 * crate::ATTACK_COST;
    assert!(
        (one.pitch - two.pitch - (held - attack)).abs() < 1e-6,
        "{one:?} {two:?} held {held} attack {attack}"
    );
    // So this rejoin raises the loss, and states by how much.
    assert!(stated.loss_change > 0.0, "{stated:?}");
    assert!((stated.loss_change - (one.total() - two.total())).abs() < 1e-9);
}

#[test]
fn a_dip_with_an_onset_at_its_bottom_stays_two_notes() {
    let (frames, mut onsets) = performance(&d_sharp_dip());
    // An onset at the dip's bottom, 0.55 s into the note: a consonant starting a new word.
    let bottom = ((0.25 + 0.55) * f64::from(RATE)).round() as u64;
    onsets.push(Event::Onset {
        frame: bottom,
        strength: 0.3,
    });
    onsets.sort_by_key(|e| match *e {
        Event::Onset { frame, .. } => frame,
        _ => 0,
    });
    for by in [By::Rules, By::Hmm] {
        let (decoded, rejoined) = readings(by, &frames, &onsets);
        assert_eq!(keys(&decoded), [D_SHARP_3, D_SHARP_3], "{by:?}");
        assert_eq!(rejoined.notes, decoded.notes, "{by:?}");
        assert_eq!(rejoined.rejoins, [], "{by:?}");
    }
}

#[test]
fn a_turn_in_the_middle_of_a_note_is_one_note_with_one_excursion() {
    // G#3, turning up to A#3 and back over 0.14 s in its middle (0.04 s up, held 0.06 s, 0.04 s
    // down), with no onset: the shape of Blood Sport's turn, inside a note.
    let (at, ramp, held) = (0.4, 0.04, 0.06);
    let turn = move |t: f64| {
        let up = |x: f64| 200.0 * (std::f64::consts::FRAC_PI_2 * x / ramp).sin();
        let t = t - at;
        match t {
            t if t < 0.0 => 0.0,
            t if t < ramp => up(t),
            t if t <= ramp + held => 200.0,
            t if t < 2.0 * ramp + held => up(2.0 * ramp + held - t),
            _ => 0.0,
        }
    };
    let (frames, onsets) = performance(&sung(G_SHARP_3, 1.0, turn));
    // On main the HMM reads two G#3: the turn is the second's attack.
    let (decoded, rejoined) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(
        keys(&decoded),
        [G_SHARP_3, G_SHARP_3],
        "{:?}",
        decoded.notes
    );
    assert_eq!(keys(&rejoined), [G_SHARP_3], "{:?}", rejoined.notes);
    let part = rejoined.notes[0].residual.as_ref().unwrap();
    assert_eq!(part.excursions.len(), 1, "{part:?}");
    let turn = part.excursions[0];
    assert!((f64::from(turn.cents) - 200.0).abs() <= 10.0, "{turn:?}");
    let secs = |f: u64| f as f64 / f64::from(RATE);
    let from = 0.25 + at + ramp;
    assert!((secs(turn.start) - from).abs() <= 0.025, "{turn:?}");
    // The rules read G#3, A#3, G#3: no same-key pair, so there is nothing to rejoin. That is the
    // rules' stated limit, as a held approach read as a note is.
    let (decoded, rejoined) = readings(By::Rules, &frames, &onsets);
    assert_eq!(keys(&decoded), [G_SHARP_3, G_SHARP_3 + 2, G_SHARP_3]);
    assert_eq!(rejoined.notes, decoded.notes);
}

#[test]
fn a_v_shaped_dip_that_holds_nowhere_is_stated_as_its_notes_excursion() {
    // Held on key 60, a V 180 cents down and back over 9 frames, built by hand: every frame of
    // the V is 40 cents or more from the next, so the residual finds no excursion in it, and the
    // pass states the frames about its bottom as one.
    let v = [
        -40.0, -80.0, -120.0, -160.0, -180.0, -140.0, -100.0, -60.0, -20.0,
    ];
    let mut column: Vec<Option<f32>> = vec![None; 10];
    column.extend([Some(0.0); 40]);
    column.extend(v.map(Some));
    column.extend([Some(0.0); 40]);
    column.extend([None; 10]);
    let on_key = A440_CENTS as f32 - 900.0;
    let column: Vec<Option<f32>> = column.into_iter().map(|c| c.map(|c| on_key + c)).collect();
    let n = column.len();
    let mut frames = PerformanceFrames::new(&PerformanceHead::new(RATE, HOP as u32, 0));
    frames
        .push(&FrameBlock::new(0, column, vec![0.95; n], vec![0.1; n]).unwrap())
        .unwrap();
    let (decoded, rejoined) = readings(By::Hmm, &frames, &[]);
    assert_eq!(keys(&decoded), [60, 60], "{:?}", decoded.notes);
    assert_eq!(keys(&rejoined), [60], "{:?}", rejoined.notes);
    let part = rejoined.notes[0].residual.as_ref().unwrap();
    // The frames from -80 down and back to -100: the run 50 cents or more below the note's
    // centre, inside the dip, which ends where the second note was held from.
    let bottom = (10 + 40 + 1) as u64 * HOP;
    assert_eq!(
        part.excursions,
        [Excursion {
            start: bottom,
            end: bottom + 6 * HOP,
            cents: -130.0,
        }],
        "{part:?}"
    );
}

#[test]
fn a_dip_deeper_than_a_whole_tone_stays_two_notes() {
    // Both #28 dip witnesses, 8 semitones and 3 down and back, as frames one hop apart, run
    // through either reader and then the pass.
    for (what, pitches) in [
        ("between syllables", dip_between_syllables()),
        ("slower", slower_dip()),
    ] {
        let mut column: Vec<Option<f32>> = vec![None; 10];
        column.extend(pitches);
        column.extend([None; 10]);
        let column: Vec<Option<f32>> = column
            .into_iter()
            .map(|p| p.map(|m| (A440_CENTS as f32) + 100.0 * (m - 69.0)))
            .collect();
        let n = column.len();
        let voicing: Vec<f32> = column
            .iter()
            .map(|p| if p.is_some() { 1.0 } else { 0.0 })
            .collect();
        let mut frames = PerformanceFrames::new(&PerformanceHead::new(RATE, HOP as u32, 0));
        frames
            .push(&FrameBlock::new(0, column, voicing, vec![0.1; n]).unwrap())
            .unwrap();
        for by in [By::Rules, By::Hmm] {
            let (decoded, rejoined) = readings(by, &frames, &[]);
            assert_eq!(
                keys(&decoded),
                [60, 60],
                "{what}, {by:?}: {:?}",
                decoded.notes
            );
            assert_eq!(keys(&rejoined), [60, 60], "{what}, {by:?}");
            assert_eq!(rejoined.rejoins, [], "{what}, {by:?}");
        }
    }
}

#[test]
fn a_sixteenth_note_step_stays_three_notes() {
    // Two sixteenths at 108 a minute, G3 then G#3, each 0.139 s on its key, then a held A#3,
    // legato: the place a pass could most easily join notes on two keys.
    let sixteenth = 60.0 / 108.0 / 4.0;
    let steps = move |t: f64| match t {
        t if t < sixteenth => 0.0,
        t if t < 2.0 * sixteenth => 100.0,
        _ => 300.0,
    };
    let (frames, onsets) = performance(&sung(55, 2.0 * sixteenth + 0.6, steps));
    for by in [By::Rules, By::Hmm] {
        let (decoded, rejoined) = readings(by, &frames, &onsets);
        assert_eq!(
            keys(&rejoined),
            [55, 56, 58],
            "{by:?}: {:?}",
            rejoined.notes
        );
        assert_eq!(rejoined.notes, decoded.notes, "{by:?}");
    }
}

#[test]
fn a_held_approach_is_the_note_it_arrives_at() {
    // 0.1 s held 35 cents flat a semitone below F#3, then F#3: the HMM reads one note on the
    // arrival, the approach its attack. This is the arrival rule, and the pass leaves it. The
    // voice moves to the arrival over 20 ms, as a voice does: a step in no time at all is a
    // change of spectrum the onsets find, and an onset would split any note.
    let approach = |t: f64| match t {
        t if t < 0.1 => -135.0,
        t if t < 0.12 => -135.0 * (1.0 - (t - 0.1) / 0.02),
        _ => 0.0,
    };
    let (frames, onsets) = performance(&sung(F_SHARP_3, 0.8, approach));
    let (decoded, rejoined) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(keys(&rejoined), [F_SHARP_3], "{:?}", rejoined.notes);
    assert_eq!(rejoined, decoded);
}

#[test]
fn a_reading_with_nothing_to_rejoin_is_returned_as_it_was() {
    // A repeated pitch split at its onsets: the same key, but every split has an onset.
    let tones = [(hz(G_SHARP_3), 0.4); 3];
    let signal = [
        synth::silence(RATE, 0.25),
        synth::note_sequence(RATE, &tones, 4, 0.3),
        synth::silence(RATE, 0.25),
    ]
    .concat();
    let (frames, onsets) = performance(&signal);
    for by in [By::Rules, By::Hmm] {
        let (decoded, rejoined) = readings(by, &frames, &onsets);
        assert_eq!(keys(&decoded), [G_SHARP_3; 3], "{by:?}");
        assert_eq!(rejoined, decoded, "{by:?}");
    }
}
