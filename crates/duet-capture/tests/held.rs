//! The held-pitch pass on the one real take that may be cited in git, "Wayfaring Stranger"
//! (`corpus/wayfaring-stranger.flac`, CC BY 4.0), read as `duet capture melody` reads it with
//! nothing given, under every reader (ruled by the owner, 2026-11-17). The owner ruled the held
//! pitches at 2.28, 3.77, 5.17 and 9.07 s notes, wants those at 13.84 and 18.56 s captured as
//! the rules have them, and hears 12.03 and 14.87 s as approaches: the rule suggests one at
//! 12.03 s only, and 14.87 s is its accepted miss.

mod support;

use duet_capture::{ANALYSIS_RATE, ReadNote, ReaderName, Reading, SuggestionRule, Suggests};

/// The keys, on the take's tuning.
const C_3: u8 = 48;
const C_SHARP_3: u8 = 49;
const D_3: u8 = 50;
const D_SHARP_3: u8 = 51;
const F_3: u8 = 53;
const F_SHARP_3: u8 = 54;
const G_SHARP_3: u8 = 56;
const A_3: u8 = 57;
const A_SHARP_3: u8 = 58;

fn readers() -> Vec<ReaderName> {
    let mut readers = vec![ReaderName::Rules, ReaderName::Hmm];
    if cfg!(feature = "basic-pitch") {
        readers.push(ReaderName::BasicPitch);
    }
    readers
}

fn reading(reader: ReaderName) -> Reading {
    let path = support::corpus_dir().join("wayfaring-stranger.flac");
    support::capture_melody(&path, reader).unwrap().1
}

fn secs(frame: u64) -> f64 {
    frame as f64 / f64::from(ANALYSIS_RATE)
}

/// The note sounding at `at` seconds, and the note after it.
fn at(reading: &Reading, at: f64) -> Option<(&ReadNote, &ReadNote)> {
    let i = reading
        .notes
        .iter()
        .position(|n| secs(n.start) <= at && at < secs(n.end))?;
    Some((&reading.notes[i], reading.notes.get(i + 1)?))
}

/// The notes sounding at any time within `from..to` seconds.
fn during(reading: &Reading, from: f64, to: f64) -> Vec<&ReadNote> {
    reading
        .notes
        .iter()
        .filter(|n| secs(n.end) > from && secs(n.start) < to)
        .collect()
}

/// Whether the pass made a note sounding within `from..to` seconds.
fn made_during(reading: &Reading, from: f64, to: f64) -> bool {
    reading
        .held
        .iter()
        .flat_map(|h| &h.notes)
        .any(|n| secs(n.end) > from && secs(n.start) < to)
}

/// Each moment by where its pitch is held, the key held, and the key it reaches.
const HELD: [(f64, u8, u8); 8] = [
    (2.29, G_SHARP_3, A_SHARP_3),
    (3.78, G_SHARP_3, A_SHARP_3),
    (5.30, C_SHARP_3, D_SHARP_3),
    (9.08, C_3, D_SHARP_3),
    (12.05, A_3, A_SHARP_3),
    (13.85, G_SHARP_3, A_SHARP_3),
    (14.89, F_SHARP_3, G_SHARP_3),
    (18.60, F_3, F_SHARP_3),
];

#[test]
fn every_held_pitch_the_owner_ruled_is_a_note_of_its_own_under_every_reader() {
    // On main the HMM folds all eight into the note they reach, and Basic Pitch leaves five
    // unread before it; the rules read each as a note already.
    for reader in readers() {
        let reading = reading(reader);
        for (moment, held, reaches) in HELD {
            let (note, next) = at(&reading, moment)
                .unwrap_or_else(|| panic!("{reader:?} {moment}: no note: {:?}", reading.notes));
            assert_eq!(
                (note.key, next.key),
                (held, reaches),
                "{reader:?} {moment}: {note:?} {next:?}"
            );
            assert!(
                secs(next.start) - secs(note.end) <= 0.1,
                "{reader:?} {moment}: {note:?} {next:?}"
            );
        }
    }
}

#[test]
fn the_note_hmm_states_each_note_it_made_with_the_note_it_reaches_as_decoded() {
    let reading = reading(ReaderName::Hmm);
    for (moment, held, reaches) in HELD {
        let stated = reading
            .held
            .iter()
            .find(|h| {
                h.notes
                    .iter()
                    .any(|n| secs(n.start) <= moment && moment < secs(n.end))
            })
            .unwrap_or_else(|| panic!("{moment}: not stated: {:?}", reading.held));
        assert_eq!(stated.decoded.key, reaches, "{moment}: {stated:?}");
        assert!(stated.notes.iter().any(|n| n.key == held), "{stated:?}");
        assert!(stated.decoded.residual.is_none(), "{stated:?}");
    }
}

#[test]
fn only_12_03_s_is_suggested_a_likely_approach_under_every_reader() {
    for reader in readers() {
        let reading = reading(reader);
        let [suggestion] = reading.suggestions.as_slice() else {
            panic!("{reader:?}: one suggestion: {:?}", reading.suggestions);
        };
        let (note, _) = at(&reading, 12.05).unwrap();
        assert_eq!(suggestion.start, note.start, "{reader:?}: {suggestion:?}");
        assert_eq!(suggestion.suggests, Suggests::LikelyApproach);
        assert_eq!(suggestion.rule, SuggestionRule::SemitoneWithDrift);
        assert_eq!(
            (suggestion.key, suggestion.next_key, suggestion.step),
            (A_3, A_SHARP_3, 1)
        );
        assert!(suggestion.drift_cents > 30.0, "{reader:?}: {suggestion:?}");
    }
}

#[test]
fn the_held_pitch_at_14_87_s_is_a_note_with_no_suggestion_an_accepted_miss() {
    // The owner hears the F#3 as an approach to G#3, but it is a whole tone below: the rule does
    // not fire, and the owner folds it by choice. A change to the rule that suggests it is a
    // decision, so it is pinned.
    for reader in readers() {
        let reading = reading(reader);
        let (note, next) = at(&reading, 14.89).unwrap();
        assert_eq!((note.key, next.key), (F_SHARP_3, G_SHARP_3), "{reader:?}");
        assert!(
            reading.suggestions.iter().all(|s| s.start != note.start),
            "{reader:?}: {:?}",
            reading.suggestions
        );
    }
}

#[test]
fn the_turn_at_1_42_s_and_the_lean_at_20_05_s_are_left_as_they_were() {
    for reader in readers() {
        let reading = reading(reader);
        assert!(
            !made_during(&reading, 1.2, 1.7),
            "{reader:?}: {:?}",
            reading.held
        );
        assert!(
            !made_during(&reading, 19.8, 20.35),
            "{reader:?}: {:?}",
            reading.held
        );
    }
    // The HMM, as the owner rules right at both: one D#3 with the turn in it, its excursion; and
    // the fall from D#3, near silent, returning a half step lower on D3.
    let reading = reading(ReaderName::Hmm);
    let notes = during(&reading, 1.3, 1.6);
    let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
    assert_eq!(keys, [D_SHARP_3], "{notes:?}");
    let excursions = &notes[0].residual.as_ref().unwrap().excursions;
    assert!(
        excursions
            .iter()
            .any(|e| (1.40..1.47).contains(&secs(e.start))),
        "{excursions:?}"
    );
    let keys: Vec<u8> = during(&reading, 19.9, 20.3).iter().map(|n| n.key).collect();
    assert_eq!(keys, [D_SHARP_3, D_3]);
}
