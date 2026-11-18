//! The corpus's outside transcription of "Wayfaring Stranger":
//! `corpus/wayfaring-stranger.logic-flex-pitch.mid`, the same performance transcribed by Logic
//! Pro's Flex Pitch and exported unedited (CC BY 4.0, as the recording is). It is a reference
//! to set duet's readings beside, so this holds only that it is that transcription of that
//! take: read through duet-midi, it has the notes Logic exported, and it starts and ends where
//! the HMM's reading of `corpus/wayfaring-stranger.flac` does; and duet's key estimate reads
//! its notes as the owner ruled the take, E-flat minor (temper task 01a1e95b), as it reads
//! duet's own readings (`tests/key.rs`). Its notes are not asserted: they are evidence, not an
//! answer.

mod support;

use duet_capture::{ANALYSIS_RATE, Event, KeyProfile, ReaderName, estimate_key, estimate_key_with};
use duet_core::{Key, Part};
use duet_midi::Midi;

/// How many notes Logic's Flex Pitch exported.
const NOTES: usize = 52;

/// How far, in seconds, the transcription's first note start and last note end may sit from
/// the HMM reading's. On 2026-11-17 they sit 0.085 s and 0.018 s apart: Flex Pitch starts a C#3
/// at 1.05 s, where the voice begins its slide up, and the HMM starts its first D#3 at 1.13 s,
/// held from 1.29 s. An export shifted in time by a fifth of a second, or of another take,
/// moves them further than this.
const WITHIN_SECS: f64 = 0.15;

/// The transcription, read through duet-midi.
fn transcription() -> Part {
    let path = support::corpus_dir().join("wayfaring-stranger.logic-flex-pitch.mid");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Part::from_midi(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each note's start and end in seconds and its MIDI key, in order of start. The export has one
/// tempo, so a tick is a fixed length of time.
fn note_secs(part: &Part) -> Vec<(f64, f64, u8)> {
    assert!(
        part.tempo_changes.is_empty(),
        "the export has one tempo: {:?}",
        part.tempo_changes
    );
    let map = part.time_map().expect("a time map");
    let secs_per_quarter = 60.0 / part.tempo_bpm;
    let mut notes: Vec<(f64, f64, u8)> = part
        .tracks
        .iter()
        .flat_map(|track| &track.notes)
        .map(|note| {
            let tick = map.tick(note.at).expect("a position in the part");
            let start = tick as f64 / f64::from(part.ticks_per_quarter) * secs_per_quarter;
            (
                start,
                start + note.beats * secs_per_quarter,
                note.pitch.midi(),
            )
        })
        .collect();
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    notes
}

#[test]
fn the_flex_pitch_transcription_reads_through_duet_midi_and_spans_the_take() {
    let part = transcription();
    let notes = note_secs(&part);
    assert_eq!(notes.len(), NOTES, "the notes Logic exported");

    let path = support::corpus_dir().join("wayfaring-stranger.flac");
    let (_, reading) = support::capture_melody(&path, ReaderName::Hmm).expect("the HMM reading");
    let secs = |frame: u64| frame as f64 / f64::from(ANALYSIS_RATE);
    let read_first = secs(reading.notes.first().expect("a note").start);
    let read_last = secs(reading.notes.iter().map(|n| n.end).max().expect("a note"));

    let first = notes[0].0;
    let last = notes.iter().map(|n| n.1).fold(f64::MIN, f64::max);
    assert!(
        (first - read_first).abs() <= WITHIN_SECS,
        "the transcription starts at {first:.3} s, the HMM reading at {read_first:.3} s"
    );
    assert!(
        (last - read_last).abs() <= WITHIN_SECS,
        "the transcription ends at {last:.3} s, the HMM reading at {read_last:.3} s"
    );
}

#[test]
fn the_flex_pitch_transcription_estimates_e_flat_minor_as_the_take_was_ruled() {
    let frame = |secs: f64| (secs * f64::from(ANALYSIS_RATE)).round() as u64;
    let events: Vec<Event> = note_secs(&transcription())
        .into_iter()
        .map(|(start, end, midi)| Event::Note {
            start: frame(start),
            end: frame(end),
            midi,
            confidence: 1.0,
            strength: 1.0,
        })
        .collect();
    let top = |profile: KeyProfile| {
        let estimate = estimate_key_with(&events, None, profile).expect("notes");
        let mut scores = estimate.scores;
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        scores
            .iter()
            .take(3)
            .map(|(k, r)| format!("{k} {r:.3}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let estimate = estimate_key(&events, None).expect("notes");
    assert_eq!(estimate.profile, Some(KeyProfile::AardenEssen));
    let e_flat_minor: Key = "Eb minor".parse().expect("a key");
    assert_eq!(
        *estimate.key.value(),
        e_flat_minor,
        "Aarden–Essen: {}",
        top(KeyProfile::AardenEssen)
    );
    // For the record: Krumhansl–Kessler, duet's profile until 1.12, reads A-flat major here,
    // as it read duet's own readings.
    eprintln!(
        "Aarden–Essen {}; Krumhansl–Kessler {}",
        top(KeyProfile::AardenEssen),
        top(KeyProfile::KrumhanslKessler)
    );
}
