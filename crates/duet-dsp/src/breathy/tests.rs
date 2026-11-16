//! The breathy ornament's witnesses (the 2026-11-16 arrivals and ornaments design, task F): a
//! held note's breathy fall is read as the note's to its end, under either reader; a breath
//! between two phrases, and a weak run that fails any one of the ruling's three conditions,
//! stays unread.

use super::*;
use crate::hmm::NoteModel;
use crate::performance::{FrameBlock, PerformanceHead};
use crate::reading::{A440_CENTS, read, read_as_decoded};

const RATE: u32 = 22_050;
const HOP: u64 = 256;
/// E4, as the HMM's hand-built witnesses sing.
const KEY: u8 = 64;
/// A sung note's level, its root mean square.
const SUNG: f32 = 0.1;
/// The noise floor: 40 dB under the note.
const FLOOR: f32 = 0.001;

/// What a hand-built frame is: unvoiced at the floor, or voiced at `cents` from [`KEY`] on
/// A440 with `voicing` and `level`.
#[derive(Clone, Copy)]
enum Hand {
    Unvoiced,
    Voiced {
        cents: f32,
        voicing: f32,
        level: f32,
    },
}

/// Frames at the analysis hop, built by hand from runs of `(count, frame)`; a run's frames may
/// vary with their index in it through `frame`.
fn by_hand(runs: &[(usize, &dyn Fn(usize) -> Hand)]) -> PerformanceFrames {
    let head = PerformanceHead::new(RATE, HOP as u32, 0);
    let mut frames = PerformanceFrames::new(&head);
    let on_key = (A440_CENTS + 100.0 * (f64::from(KEY) - 69.0)) as f32;
    let (mut pitch, mut voicing, mut level) = (Vec::new(), Vec::new(), Vec::new());
    for &(count, frame) in runs {
        for i in 0..count {
            match frame(i) {
                Hand::Unvoiced => {
                    pitch.push(None);
                    voicing.push(0.0);
                    level.push(FLOOR);
                }
                Hand::Voiced {
                    cents,
                    voicing: v,
                    level: l,
                } => {
                    pitch.push(Some(on_key + cents));
                    voicing.push(v);
                    level.push(l);
                }
            }
        }
    }
    frames
        .push(&FrameBlock::new(0, pitch, voicing, level).unwrap())
        .unwrap();
    frames
}

/// Frames in `secs`.
fn hops(secs: f64) -> usize {
    (secs * f64::from(RATE) / HOP as f64).round() as usize
}

/// Where frame index `k` sits, on the take's timeline.
fn at(k: usize) -> u64 {
    k as u64 * HOP
}

/// An onset at frame index `k`.
fn onset(k: usize) -> Event {
    Event::Onset {
        frame: at(k),
        strength: 0.8,
    }
}

/// A run's frames, by their index in it.
type Run<'a> = &'a dyn Fn(usize) -> Hand;

const UNVOICED: &dyn Fn(usize) -> Hand = &|_| Hand::Unvoiced;
/// A note held on [`KEY`], strongly voiced.
const HELD: &dyn Fn(usize) -> Hand = &|_| Hand::Voiced {
    cents: 0.0,
    voicing: 0.95,
    level: SUNG,
};
/// A breath, as #48's witness models it: voicing 0.02, its pitch no line (it jumps 40 to 75
/// cents a frame), its level at the floor.
const BREATH: &dyn Fn(usize) -> Hand = &|i| Hand::Voiced {
    cents: [-40.0, 25.0, -15.0, 35.0][i % 4],
    voicing: 0.02,
    level: FLOOR,
};

/// A breathy fall: voicing 0.05, its pitch going on down 8 cents a frame from the note's, its
/// level half the note's.
fn fall(i: usize) -> Hand {
    Hand::Voiced {
        cents: -8.0 * (i as f32 + 1.0),
        voicing: 0.05,
        level: SUNG / 2.0,
    }
}

/// Which reader.
#[derive(Debug, Clone, Copy, PartialEq)]
enum By {
    Rules,
    Hmm,
}

/// The take read by `by` on A440, before the passes and after them.
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

#[test]
fn a_held_notes_breathy_fall_is_read_as_the_note_to_the_end_of_the_fall() {
    // 0.5 s held, then 0.1 s falling at voicing 0.05, its pitch continuing and its level half
    // the note's. On main the HMM ends the note where the voicing drops, and leaves the fall
    // unread.
    let (lead, held, falling) = (20, hops(0.5), hops(0.1));
    let frames = by_hand(&[
        (lead, UNVOICED),
        (held, HELD),
        (falling, &fall),
        (20, UNVOICED),
    ]);
    let last_of_fall = lead + held + falling - 1;
    let to_the_end = |by: By, read: &Reading| {
        assert_eq!(keys(read), [KEY], "{by:?}: {:?}", read.notes);
        let note = &read.notes[0];
        assert!(
            note.end > at(last_of_fall) && note.end <= at(last_of_fall + 1),
            "{by:?}: the note ends at {}, not after the fall's last frame at {}",
            note.end,
            at(last_of_fall)
        );
        let take = read.residual.as_ref().unwrap();
        assert_eq!(take.unread_frames, 0, "{by:?}: {take:?}");
        let part = note.residual.as_ref().unwrap();
        assert!(
            part.fall.is_some(),
            "{by:?}: the fall is the note's: {part:?}"
        );
    };
    let (decoded, read) = readings(By::Hmm, &frames, &[]);
    to_the_end(By::Hmm, &read);
    // The HMM ended it before the fall; the pass states what it kept.
    assert_eq!(keys(&decoded), [KEY], "{:?}", decoded.notes);
    assert!(
        decoded.notes[0].end < at(lead + held + 2),
        "{:?}",
        decoded.notes
    );
    let [breathy] = read.breathy.as_slice() else {
        panic!("one note carried on: {:?}", read.breathy);
    };
    assert_eq!(breathy.decoded.end, decoded.notes[0].end);
    assert!(breathy.decoded.residual.is_none());
    assert_eq!(breathy.end, read.notes[0].end);
    assert!((breathy.lowest_db - (-6.02)).abs() < 0.1, "{breathy:?}");
    // Reading frames voiced at 0.05 raises the loss, and the pass says by how much.
    assert!(breathy.loss_change > 0.0, "{breathy:?}");
    let (total, before) = (
        read.residual.as_ref().unwrap().loss.total(),
        decoded.residual.as_ref().unwrap().loss.total(),
    );
    assert!(
        (total - before - breathy.loss_change).abs() < 1e-9,
        "{total} - {before} is not {}",
        breathy.loss_change
    );
    // The rules already read the fall (they segment by pitch, not by #48's pricing), and the
    // pass leaves their reading as decoded.
    let (decoded, read) = readings(By::Rules, &frames, &[]);
    to_the_end(By::Rules, &read);
    assert_eq!(read, decoded);
}

#[test]
fn a_breath_between_two_phrases_stays_unread() {
    // A phrase on E4, 0.3 s of breath at voicing 0.02 with no pitch line at the floor, then an
    // onset and the next phrase on G4.
    let g: &dyn Fn(usize) -> Hand = &|_| Hand::Voiced {
        cents: 300.0,
        voicing: 0.95,
        level: SUNG,
    };
    let (lead, held, breath) = (20, hops(0.4), hops(0.3));
    let frames = by_hand(&[
        (lead, UNVOICED),
        (held, HELD),
        (breath, BREATH),
        (held, g),
        (20, UNVOICED),
    ]);
    let onsets = [onset(lead), onset(lead + held + breath)];
    for by in [By::Rules, By::Hmm] {
        let (decoded, read) = readings(by, &frames, &onsets);
        assert_eq!(keys(&read), [KEY, KEY + 3], "{by:?}: {:?}", read.notes);
        assert_eq!(read, decoded, "{by:?}: the pass changed the reading");
        assert!(read.breathy.is_empty());
    }
    // The HMM leaves it unread, as #48 prices it. The rules read a breath whose pitch stays
    // near the note into the note, as they did before #48: their limit, which the pass leaves
    // as it is.
    let (_, read) = readings(By::Hmm, &frames, &onsets);
    let take = read.residual.as_ref().unwrap();
    assert_eq!(
        take.unread_frames as usize, breath,
        "the breath is read: {take:?}"
    );
}

#[test]
fn a_weak_run_that_fails_any_one_condition_stays_unread() {
    // The breathy fall's frames, each changed in one way only.
    let jumping: &dyn Fn(usize) -> Hand = &|i| match fall(i) {
        Hand::Voiced { voicing, level, .. } => Hand::Voiced {
            cents: [-40.0, 25.0, -15.0, 35.0][i % 4],
            voicing,
            level,
        },
        Hand::Unvoiced => Hand::Unvoiced,
    };
    let at_the_floor: &dyn Fn(usize) -> Hand = &|i| match fall(i) {
        Hand::Voiced { cents, voicing, .. } => Hand::Voiced {
            cents,
            voicing,
            level: FLOOR,
        },
        Hand::Unvoiced => Hand::Unvoiced,
    };
    let (lead, held, falling) = (20, hops(0.5), hops(0.1));
    let after = lead + held + falling + 10;
    let next: &dyn Fn(usize) -> Hand = &|_| Hand::Voiced {
        cents: -500.0,
        voicing: 0.95,
        level: SUNG,
    };
    let cases: [(&str, Run, Vec<Event>); 3] = [
        ("its pitch no line", jumping, vec![]),
        ("its level at the floor", at_the_floor, vec![]),
        // An onset 0.09 s before the next note: further than a note starts from its onset
        // (50 ms), so it is no note's, and marks what came before it as a breath.
        (
            "an onset before the next note",
            &fall,
            vec![onset(after - 8)],
        ),
    ];
    for (what, weak, onsets) in cases {
        let frames = by_hand(&[
            (lead, UNVOICED),
            (held, HELD),
            (falling, weak),
            (10, UNVOICED),
            (held, next),
            (20, UNVOICED),
        ]);
        for by in [By::Rules, By::Hmm] {
            let (decoded, read) = readings(by, &frames, &onsets);
            assert!(
                read.breathy.is_empty(),
                "{what}, {by:?}: {:?}",
                read.breathy
            );
            assert_eq!(read.notes, decoded.notes, "{what}, {by:?}");
        }
    }
}

#[test]
fn a_weak_glide_into_the_next_note_belongs_to_the_note_it_arrives_at() {
    // E4, then a weak glide up 25 cents a frame to F#4, then F#4: the pass carries E4 on only
    // while the glide is nearer E4 than F#4 (#28's ruling: a slide belongs to where it arrives).
    let glide: &dyn Fn(usize) -> Hand = &|i| Hand::Voiced {
        cents: 25.0 * (i as f32 + 1.0),
        voicing: 0.05,
        level: SUNG / 2.0,
    };
    let f_sharp: &dyn Fn(usize) -> Hand = &|_| Hand::Voiced {
        cents: 200.0,
        voicing: 0.95,
        level: SUNG,
    };
    let (lead, held) = (20, hops(0.4));
    let frames = by_hand(&[
        (lead, UNVOICED),
        (held, HELD),
        (7, glide),
        (held, f_sharp),
        (20, UNVOICED),
    ]);
    for by in [By::Rules, By::Hmm] {
        let (_, read) = readings(by, &frames, &[]);
        assert_eq!(keys(&read), [KEY, KEY + 2], "{by:?}: {:?}", read.notes);
        // The glide's fourth frame, at +100 cents, is as near F#4 as E4: it is never E4's.
        assert!(
            read.notes[0].end <= at(lead + held + 3),
            "{by:?}: {:?}",
            read.notes
        );
    }
}

#[test]
fn a_breathy_dip_inside_a_note_is_carried_over_and_then_rejoined() {
    // E4 held, a dip of 0.08 s at voicing 0.05 that goes 30 cents down and back at half the
    // level, then E4 again strongly, with no onset: the shape of Blood Sport's "-eak". The
    // reader ends the note at the dip; the pass carries it over the dip, so the two halves
    // meet, and the rejoin reads one note, stating both.
    let dip: &dyn Fn(usize) -> Hand = &|i| Hand::Voiced {
        cents: -[10.0, 20.0, 30.0, 30.0, 20.0, 10.0, 5.0][i % 7],
        voicing: 0.05,
        level: SUNG / 2.0,
    };
    let (lead, held) = (20, hops(0.4));
    let frames = by_hand(&[
        (lead, UNVOICED),
        (held, HELD),
        (7, dip),
        (held, HELD),
        (20, UNVOICED),
    ]);
    let (decoded, read) = readings(By::Hmm, &frames, &[]);
    assert_eq!(keys(&decoded), [KEY, KEY], "{:?}", decoded.notes);
    assert_eq!(keys(&read), [KEY], "{:?}", read.notes);
    assert_eq!(read.breathy.len(), 1, "{:?}", read.breathy);
    assert_eq!(read.rejoins.len(), 1, "{:?}", read.rejoins);
    assert_eq!(read.residual.as_ref().unwrap().unread_frames, 0);
}
