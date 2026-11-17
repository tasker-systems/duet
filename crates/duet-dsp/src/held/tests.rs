//! The held-pitch pass's synthetic witnesses (ruled by the owner, 2026-11-17): a pitch held
//! before the note it reaches is a note of its own under either reader, and only a semitone
//! approach that drifts toward its note is suggested a likely approach.

use super::*;
use crate::event::Event;
use crate::hmm::NoteModel;
use crate::performance::{Layer1, Performance};
use crate::reading::{A440_CENTS, read, read_as_decoded};
use crate::stage::{Mono, Stage};
use crate::synth;

const RATE: u32 = 22_050;
/// G3, G#3 and A3: an approach on G3 reaches G#3 a semitone up, or A3 a whole tone up.
const G_3: u8 = 55;
const G_SHARP_3: u8 = 56;
const A_3: u8 = 57;

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum By {
    Rules,
    Hmm,
}

/// The take read by `by` on A440 (the witnesses are sung on it), as decoded and with its
/// interpretive passes.
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

/// A pitch held on G3 for 0.14 s, its last 0.06 s `drift` cents toward the note it reaches,
/// then a glide of 20 ms to `step` semitones up, held there half a second. A semitone with a
/// drift of +40 is the findings' likely approach (12.03 s of "Wayfaring Stranger").
fn approach(drift: f64, step: f64) -> Vec<f32> {
    let target = 100.0 * step;
    sung(G_3, 0.66, move |t| {
        if t < 0.08 {
            0.0
        } else if t < 0.14 {
            drift
        } else if t < 0.16 {
            drift + (target - drift) * (t - 0.14) / 0.02
        } else {
            target
        }
    })
}

#[test]
fn a_semitone_approach_drifting_40_cents_is_a_note_with_the_suggestion() {
    let (frames, onsets) = performance(&approach(40.0, 1.0));
    for by in [By::Rules, By::Hmm] {
        let (decoded, reading) = readings(by, &frames, &onsets);
        assert_eq!(
            keys(&reading),
            [G_3, G_SHARP_3],
            "{by:?}: {:?}",
            reading.notes
        );
        let [suggestion] = reading.suggestions.as_slice() else {
            panic!("{by:?}: one suggestion: {:?}", reading.suggestions);
        };
        assert_eq!(suggestion.start, reading.notes[0].start, "{by:?}");
        assert_eq!(suggestion.suggests, Suggests::LikelyApproach);
        assert_eq!(suggestion.rule, SuggestionRule::SemitoneWithDrift);
        assert_eq!(
            (suggestion.key, suggestion.next_key, suggestion.step),
            (G_3, G_SHARP_3, 1)
        );
        assert!(
            suggestion.drift_cents > APPROACH_DRIFT_CENTS as f32,
            "{by:?}: {suggestion:?}"
        );
        // The suggestion changes no note: the notes are as the pass alone leaves them.
        assert_eq!(
            reading.notes,
            hold_apart(
                crate::rejoin(crate::breathe(decoded, &frames, &onsets), &frames, &onsets),
                &frames
            )
            .notes
        );
    }
}

#[test]
fn the_note_hmm_folds_the_approach_and_the_pass_makes_it_a_note_stating_so() {
    // On main the HMM reads one G#3, the approach its grace.
    let (frames, onsets) = performance(&approach(40.0, 1.0));
    let (decoded, reading) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(keys(&decoded), [G_SHARP_3], "{:?}", decoded.notes);
    let [held] = reading.held.as_slice() else {
        panic!("one note reached: {:?}", reading.held);
    };
    assert_eq!(held.decoded.key, G_SHARP_3);
    assert!(held.decoded.residual.is_none());
    let [made] = held.notes.as_slice() else {
        panic!("one note made: {:?}", held.notes);
    };
    assert_eq!(made.key, G_3);
    assert!(made.residual.is_none());
    // The made note starts where the decoded one did, and the note reached starts where the made
    // one ends, held from where it was.
    let (first, second) = (&reading.notes[0], &reading.notes[1]);
    assert_eq!(first.start, held.decoded.start);
    assert_eq!(second.start, first.end);
    assert_eq!(second.held_from, held.decoded.held_from);
    assert_eq!(second.end, held.decoded.end);
    // Its held frames are no longer the G#3's slide, so its loss changes by more than the
    // note's cost alone: it is stated, and it is the difference of the two readings' totals.
    let total = |r: &Reading| r.residual.as_ref().unwrap().loss.total();
    let before = crate::rejoin(crate::breathe(decoded, &frames, &onsets), &frames, &onsets);
    assert!(
        (held.loss_change - (total(&reading) - total(&before))).abs() < 1e-9,
        "{held:?}"
    );
}

#[test]
fn the_same_pitch_held_without_drift_is_a_note_with_no_suggestion() {
    let (frames, onsets) = performance(&approach(0.0, 1.0));
    for by in [By::Rules, By::Hmm] {
        let (_, reading) = readings(by, &frames, &onsets);
        assert_eq!(
            keys(&reading),
            [G_3, G_SHARP_3],
            "{by:?}: {:?}",
            reading.notes
        );
        assert!(
            reading.suggestions.is_empty(),
            "{by:?}: {:?}",
            reading.suggestions
        );
    }
}

#[test]
fn a_whole_tone_held_pitch_drifting_40_cents_is_a_note_with_no_suggestion() {
    let (frames, onsets) = performance(&approach(40.0, 2.0));
    for by in [By::Rules, By::Hmm] {
        let (_, reading) = readings(by, &frames, &onsets);
        assert_eq!(keys(&reading), [G_3, A_3], "{by:?}: {:?}", reading.notes);
        assert!(
            reading.suggestions.is_empty(),
            "{by:?}: {:?}",
            reading.suggestions
        );
    }
}

#[test]
fn a_pitch_held_shorter_than_the_shortest_note_stays_inside_the_note() {
    // G3 held 35 ms, under the shortest note's 50 ms, then G#3: one note, under the HMM.
    let signal = sung(G_3, 0.6, |t| {
        if t < 0.035 {
            0.0
        } else if t < 0.055 {
            100.0 * (t - 0.035) / 0.02
        } else {
            100.0
        }
    });
    let (frames, onsets) = performance(&signal);
    let (_, reading) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(keys(&reading), [G_SHARP_3], "{:?}", reading.notes);
    assert!(reading.held.is_empty(), "{:?}", reading.held);
}

#[test]
fn an_ornament_once_the_voice_is_on_the_note_stays_its_excursion() {
    // G#3 held 0.12 s, then up to A3 -25 for 0.07 s and back, the shape of the upper neighbour
    // at 16.40-16.55 s of "Wayfaring Stranger", where the HMM holds the second F#3 from after
    // the rise. Held from there, the rise is before `held_from`, but the voice is on G#3 before
    // it, so it stays the G#3's excursion.
    let signal = sung(G_SHARP_3, 0.7, |t| {
        if (0.12..0.19).contains(&t) { 75.0 } else { 0.0 }
    });
    let (frames, onsets) = performance(&signal);
    let mut decoded = read_as_decoded(&frames, &onsets, Some(A440_CENTS));
    assert_eq!(keys(&decoded), [G_SHARP_3], "{:?}", decoded.notes);
    decoded.notes[0].held_from = (f64::from(RATE) * (0.25 + 0.2)) as u64;
    let reading = hold_apart(decoded.clone(), &frames);
    assert!(reading.held.is_empty(), "{:?}", reading.held);
    assert_eq!(keys(&reading), [G_SHARP_3]);
    let excursions = &reading.notes[0].residual.as_ref().unwrap().excursions;
    assert!(
        excursions.iter().any(|e| (50.0..100.0).contains(&e.cents)),
        "{excursions:?}"
    );
}

#[test]
fn an_overshoot_after_a_held_approach_is_the_note_it_reaches() {
    // G3 held 0.1 s, then up past G#3 to A3 -30 for 0.06 s, then down to G#3, the shape of
    // 2.28 s and 14.87 s of "Wayfaring Stranger": the G3 is a note, the overshoot the G#3's.
    let signal = sung(G_3, 0.76, |t| {
        if t < 0.1 {
            0.0
        } else if t < 0.12 {
            170.0 * (t - 0.1) / 0.02
        } else if t < 0.18 {
            170.0
        } else if t < 0.2 {
            170.0 - 70.0 * (t - 0.18) / 0.02
        } else {
            100.0
        }
    });
    let (frames, onsets) = performance(&signal);
    let (_, reading) = readings(By::Hmm, &frames, &onsets);
    assert_eq!(keys(&reading), [G_3, G_SHARP_3], "{:?}", reading.notes);
}
