//! The same-key rejoin on the one real take that may be cited in git, "Wayfaring Stranger"
//! (`corpus/wayfaring-stranger.flac`, CC BY 4.0), read as `duet capture melody` reads it with
//! nothing given, under either reader (the 2026-11-16 arrivals and ornaments design, task B).
//! The owner heard 9.54 s as one D#3 with a dip, and 16.39 s as two F#3 at a new word.

mod support;

use duet_capture::{ANALYSIS_RATE, ReadNote, ReaderName, Reading};

/// D#3 and F#3, on the take's tuning.
const D_SHARP_3: u8 = 51;
const F_SHARP_3: u8 = 54;

fn reading(reader: ReaderName) -> Reading {
    let path = support::corpus_dir().join("wayfaring-stranger.flac");
    support::capture_melody(&path, reader).unwrap().1
}

fn secs(frame: u64) -> f64 {
    frame as f64 / f64::from(ANALYSIS_RATE)
}

/// The notes sounding at any time within `from..to` seconds.
fn during(reading: &Reading, from: f64, to: f64) -> Vec<&ReadNote> {
    reading
        .notes
        .iter()
        .filter(|n| secs(n.end) > from && secs(n.start) < to)
        .collect()
}

#[test]
fn a_dip_inside_a_phrase_at_9_54_s_is_one_d_sharp_3_with_the_dip_its_excursion() {
    // On main both readers read D#3, D#3, split at 9.54 s. The dip falls 180 cents and back
    // with no onset and every frame voiced, so the pass rejoins it.
    for reader in [ReaderName::Hmm, ReaderName::Rules] {
        let reading = reading(reader);
        let notes = during(&reading, 9.3, 9.9);
        let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
        assert_eq!(keys, [D_SHARP_3], "{reader:?}: {notes:?}");
        let note = notes[0];
        let rejoin = reading
            .rejoins
            .iter()
            .find(|r| r.decoded[0].start == note.start)
            .unwrap_or_else(|| panic!("{reader:?}: no rejoin stated: {:?}", reading.rejoins));
        assert_eq!(rejoin.decoded.len(), 2, "{reader:?}: {rejoin:?}");
        let dip = rejoin.dips[0];
        assert!((9.5..9.6).contains(&secs(dip.start)), "{reader:?}: {dip:?}");
        assert!(
            (-200.0..=-150.0).contains(&dip.cents),
            "{reader:?}: {dip:?}"
        );
        // One excursion, the dip's: from 9.55 s, about 140 cents down.
        let excursions = &note.residual.as_ref().unwrap().excursions;
        assert_eq!(excursions.len(), 1, "{reader:?}: {excursions:?}");
        let excursion = excursions[0];
        assert!(
            (9.54..9.62).contains(&secs(excursion.start)),
            "{reader:?}: {excursion:?}"
        );
        assert!(
            (-180.0..=-100.0).contains(&excursion.cents),
            "{reader:?}: {excursion:?}"
        );
    }
}

#[test]
fn a_consonant_at_16_39_s_stays_two_f_sharp_3() {
    // A new word on the same F#3: an onset falls at the split, at 16.393 s.
    for reader in [ReaderName::Hmm, ReaderName::Rules] {
        let reading = reading(reader);
        let notes = during(&reading, 16.2, 16.7);
        let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
        assert_eq!(keys, [F_SHARP_3, F_SHARP_3], "{reader:?}: {notes:?}");
        assert!(
            reading
                .rejoins
                .iter()
                .all(|r| !(16.0..16.8).contains(&secs(r.decoded[0].end))),
            "{reader:?}: {:?}",
            reading.rejoins
        );
    }
}

#[test]
fn the_held_approach_at_18_56_s_is_the_f_sharp_3_it_arrives_at() {
    // F3 35 cents flat, held 0.1 s, then F#3: the HMM takes the approach in as its attack.
    let reading = reading(ReaderName::Hmm);
    let notes = during(&reading, 18.5, 18.75);
    let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
    assert_eq!(keys, [F_SHARP_3], "{notes:?}");
}

#[cfg(feature = "basic-pitch")]
#[test]
fn basic_pitch_reads_the_dip_at_9_54_s_as_one_d_sharp_3_once_rejoined() {
    // Basic Pitch splits the D#3 in three (9.24, 9.64 and 9.78 s), as decoded. The rejoin runs
    // after it as after the other readers, and reads one D#3, as the owner hears it.
    let reading = reading(ReaderName::BasicPitch);
    let notes = during(&reading, 9.3, 9.9);
    let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
    assert_eq!(keys, [D_SHARP_3], "{notes:?}");
    let rejoin = reading
        .rejoins
        .iter()
        .find(|r| r.decoded[0].start == notes[0].start)
        .unwrap_or_else(|| panic!("no rejoin stated: {:?}", reading.rejoins));
    assert_eq!(rejoin.decoded.len(), 3, "{rejoin:?}");
}

#[cfg(feature = "basic-pitch")]
#[test]
fn basic_pitch_keeps_the_consonant_at_16_39_s_two_f_sharp_3() {
    // Basic Pitch splits the F#3 at 16.380 s, 13 ms before the onset at 16.393 s. Started at
    // the onset, as the HMM starts its notes, the second F#3 keeps its split through the
    // rejoin; the split inside it at 16.554 s has no onset, and is rejoined.
    let reading = reading(ReaderName::BasicPitch);
    let notes = during(&reading, 16.2, 16.7);
    let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
    assert_eq!(keys, [F_SHARP_3, F_SHARP_3], "{notes:?}");
    assert!((16.38..16.40).contains(&secs(notes[1].start)), "{notes:?}");
}
