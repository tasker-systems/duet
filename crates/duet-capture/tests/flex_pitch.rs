//! The corpus's outside transcription of "Wayfaring Stranger":
//! `corpus/wayfaring-stranger.logic-flex-pitch.mid`, the same performance transcribed by Logic
//! Pro's Flex Pitch and exported unedited (CC BY 4.0, as the recording is). It is a reference
//! to set duet's readings beside, so this holds only that it is that transcription of that
//! take: read through duet-midi, it has the notes Logic exported, and it starts and ends where
//! the HMM's reading of `corpus/wayfaring-stranger.flac` does. Its notes and key are not
//! asserted; they are evidence, not an answer.

mod support;

use duet_capture::{ANALYSIS_RATE, ReaderName};
use duet_core::Song;
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
fn transcription() -> Song {
    let path = support::corpus_dir().join("wayfaring-stranger.logic-flex-pitch.mid");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Song::from_midi(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each note's start and end in seconds, in order of start. The export has one tempo, so a
/// tick is a fixed length of time.
fn note_secs(song: &Song) -> Vec<(f64, f64)> {
    assert!(
        song.tempo_changes.is_empty(),
        "the export has one tempo: {:?}",
        song.tempo_changes
    );
    let map = song.time_map().expect("a time map");
    let secs_per_quarter = 60.0 / song.tempo_bpm;
    let mut notes: Vec<(f64, f64)> = song
        .tracks
        .iter()
        .flat_map(|track| &track.notes)
        .map(|note| {
            let tick = map.tick(note.at).expect("a position in the song");
            let start = tick as f64 / f64::from(song.ticks_per_quarter) * secs_per_quarter;
            (start, start + note.beats * secs_per_quarter)
        })
        .collect();
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    notes
}

#[test]
fn the_flex_pitch_transcription_reads_through_duet_midi_and_spans_the_take() {
    let song = transcription();
    let notes = note_secs(&song);
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
