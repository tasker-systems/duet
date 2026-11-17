//! The key of the one real take that may be cited in git, "Wayfaring Stranger"
//! (`corpus/wayfaring-stranger.flac`, CC BY 4.0), read as `duet capture melody` reads it with
//! nothing given. The owner ruled it E-flat minor (2026-11-17, temper task 01a1e95b): three of
//! its four phrases and its last note rest on E-flat, and G-flat is held where G is only an
//! approach. Under the Aarden–Essen profile it reads so under every reader, both with the notes
//! the held-pitch pass made and without them.

mod support;

use duet_capture::{
    Event, KeyEstimate, KeyProfile, ReadNote, ReaderName, Reading, estimate_key, estimate_key_with,
};
use duet_core::Key;

fn readers() -> Vec<ReaderName> {
    let mut readers = vec![ReaderName::Rules, ReaderName::Hmm];
    if cfg!(feature = "basic-pitch") {
        readers.push(ReaderName::BasicPitch);
    }
    readers
}

fn e_flat_minor() -> Key {
    "Eb minor".parse().unwrap()
}

/// The reading's notes as its reader read them before the held-pitch pass: without the notes
/// the pass made, and each note they reach starting where its reader decoded it.
fn before_the_pass(reading: &Reading) -> Vec<ReadNote> {
    let made = |n: &ReadNote| {
        reading
            .held
            .iter()
            .flat_map(|h| &h.notes)
            .any(|m| (m.start, m.end, m.key) == (n.start, n.end, n.key))
    };
    reading
        .notes
        .iter()
        .filter(|n| !made(n))
        .map(|n| {
            reading
                .held
                .iter()
                .find(|h| (h.decoded.end, h.decoded.key) == (n.end, n.key))
                .map_or_else(|| n.clone(), |h| h.decoded.clone())
        })
        .collect()
}

fn events(notes: &[ReadNote]) -> Vec<Event> {
    notes.iter().map(ReadNote::event).collect()
}

fn top(estimate: &KeyEstimate) -> String {
    let mut scores = estimate.scores.clone();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
        .iter()
        .take(3)
        .map(|(k, r)| format!("{k} {r:.3}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn wayfaring_stranger_reads_e_flat_minor_under_every_reader_before_and_after_the_held_pass() {
    let path = support::corpus_dir().join("wayfaring-stranger.flac");
    for reader in readers() {
        let (captured, reading) = support::capture_melody(&path, reader).unwrap();
        let after = captured.analysis.key.as_ref().expect("a melody has a key");
        assert_eq!(after.profile, Some(KeyProfile::AardenEssen), "{reader:?}");
        assert_eq!(
            *after.key.value(),
            e_flat_minor(),
            "{reader:?} after the pass: {}",
            top(after)
        );
        assert_eq!(captured.song.key, Some(e_flat_minor()), "{reader:?}");

        let before = before_the_pass(&reading);
        if reader != ReaderName::Rules {
            assert!(
                before.len() < reading.notes.len(),
                "{reader:?}: the pass made no note, so before is after"
            );
        }
        let estimate = estimate_key(&events(&before), None).unwrap();
        assert_eq!(
            *estimate.key.value(),
            e_flat_minor(),
            "{reader:?} before the pass: {}",
            top(&estimate)
        );

        // For the record: what Krumhansl–Kessler, duet's profile until 1.12, reads.
        let kk = |notes: &[ReadNote]| {
            top(&estimate_key_with(&events(notes), None, KeyProfile::KrumhanslKessler).unwrap())
        };
        eprintln!(
            "{reader:?}: Aarden–Essen {} (before {}); Krumhansl–Kessler {} (before {})",
            top(after),
            top(&estimate),
            kk(&reading.notes),
            kk(&before)
        );
    }
}
