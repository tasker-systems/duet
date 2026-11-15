//! The key of a melody take, used as given or estimated from its notes.
//!
//! The estimate is Krumhansl–Schmuckler key finding. The notes' pitch classes are counted into
//! a histogram, each note weighted by its length, and the histogram is correlated (Pearson's r)
//! with the Krumhansl–Kessler major and minor profiles turned to each of the twelve tonics. The
//! best of those 24 keys is the estimate, and its correlation, clamped to 0..=1, is its
//! confidence. v1 covers major and minor only; the modes come later.
//!
//! The key changes a song's spelling and signature, never a note's MIDI number.

use duet_core::{Key, Mode};
use serde::Serialize;

use crate::event::Event;
use crate::timing::Origin;

/// The Krumhansl–Kessler major-key profile: how well each pitch class, from the tonic up in
/// semitones, was rated to fit a major key.
const MAJOR_PROFILE: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
/// The Krumhansl–Kessler minor-key profile, the same way.
const MINOR_PROFILE: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// A take's key, given or estimated, with the evidence the estimate was read from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct KeyEstimate {
    /// The key: the one given, or the best-scoring of the 24, with its confidence.
    pub key: Origin<Key>,
    /// The relative major or minor of `key`, which shares its signature: the likeliest other
    /// reading of the same notes.
    pub alternative: Key,
    /// How much of the take's note length falls on each pitch class, C = 0, summing to 1.
    pub histogram: [f64; 12],
    /// Every major and minor key with its correlation with the histogram, from C major and
    /// C minor up by semitones.
    pub scores: Vec<(Key, f64)>,
}

/// The key of the [`Event::Note`]s among `notes` (other events are ignored): `given` when
/// there is one, used exactly, or else the estimate. The histogram and the scores are filled
/// either way.
///
/// `None` when there are no notes, as for a rhythm take.
pub fn estimate_key(notes: &[Event], given: Option<Key>) -> Option<KeyEstimate> {
    let histogram = histogram(notes)?;
    let scores: Vec<(Key, f64)> = (0..12)
        .flat_map(|tonic| {
            [(MAJOR_PROFILE, Mode::Major), (MINOR_PROFILE, Mode::Minor)].map(|(profile, mode)| {
                let turned: [f64; 12] = std::array::from_fn(|pc| profile[(pc + 12 - tonic) % 12]);
                (key_on(tonic, mode), correlation(&histogram, &turned))
            })
        })
        .collect();
    let key = match given {
        Some(key) => Origin::Given(key),
        None => {
            let (best, score) = scores
                .iter()
                .copied()
                .reduce(|best, next| if next.1 > best.1 { next } else { best })?;
            Origin::Estimated {
                value: best,
                confidence: score.clamp(0.0, 1.0) as f32,
            }
        }
    };
    Some(KeyEstimate {
        alternative: relative(*key.value()),
        key,
        histogram,
        scores,
    })
}

/// The share of the notes' total length on each pitch class, or `None` with no notes. A note
/// of no length still counts, as one frame.
fn histogram(notes: &[Event]) -> Option<[f64; 12]> {
    let mut histogram = [0.0; 12];
    let mut any = false;
    for event in notes {
        if let Event::Note {
            start, end, midi, ..
        } = event
        {
            histogram[usize::from(midi % 12)] += end.saturating_sub(*start).max(1) as f64;
            any = true;
        }
    }
    let total: f64 = histogram.iter().sum();
    any.then(|| histogram.map(|h| h / total))
}

/// Pearson's correlation of `a` and `b`, or 0 when either does not vary.
fn correlation(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let mean = |x: &[f64; 12]| x.iter().sum::<f64>() / 12.0;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        ab += (x - ma) * (y - mb);
        aa += (x - ma) * (x - ma);
        bb += (y - mb) * (y - mb);
    }
    if aa > 0.0 && bb > 0.0 {
        ab / (aa * bb).sqrt()
    } else {
        0.0
    }
}

/// The major or minor key on the tonic `tonic` semitones above C, spelled with the signature
/// of fewer accidentals (Db major, not C# major), and sharps for the tie (F# major, D# minor).
fn key_on(tonic: usize, mode: Mode) -> Key {
    // A minor key shares its signature with the major a minor third above it.
    let major = if mode == Mode::Minor {
        tonic + 3
    } else {
        tonic
    };
    // Each step round the circle of fifths is 7 semitones, and adds a sharp.
    let sharps = (major * 7 % 12) as i8;
    Key {
        sharps: if sharps > 6 { sharps - 12 } else { sharps },
        mode,
    }
}

/// The relative major or minor: the same signature, in the other of the two modes.
fn relative(key: Key) -> Key {
    Key {
        sharps: key.sharps,
        mode: if key.is_minor() {
            Mode::Major
        } else {
            Mode::Minor
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Key {
        s.parse().unwrap()
    }

    /// Notes one after another, each `(midi, length in frames)`.
    fn melody(notes: &[(u8, u64)]) -> Vec<Event> {
        let mut at = 1_000;
        notes
            .iter()
            .map(|&(midi, len)| {
                let note = Event::Note {
                    start: at,
                    end: at + len,
                    midi,
                    confidence: 0.9,
                    strength: 0.03,
                };
                at += len;
                note
            })
            .collect()
    }

    fn estimated(estimate: &KeyEstimate) -> Key {
        match estimate.key {
            Origin::Estimated { value, .. } => value,
            Origin::Given(k) => panic!("{k} was given, not estimated"),
        }
    }

    #[test]
    fn an_e_flat_major_scale_is_estimated_as_e_flat_major() {
        // Eb F G Ab Bb C D Eb, up and back down, the tonic held longest.
        let scale = [63, 65, 67, 68, 70, 72, 74, 75, 74, 72, 70, 68, 67, 65, 63];
        let notes: Vec<(u8, u64)> = scale
            .iter()
            .enumerate()
            .map(|(i, &m)| {
                (
                    m,
                    if i == 0 || i == scale.len() - 1 {
                        8_000
                    } else {
                        4_000
                    },
                )
            })
            .collect();
        let estimate = estimate_key(&melody(&notes), None).unwrap();
        assert_eq!(estimated(&estimate), Key::new(-3, false).unwrap());
        assert_eq!(estimate.alternative, key("C minor"));
    }

    #[test]
    fn a_b_minor_tune_is_estimated_as_b_minor() {
        // B D F# B, A# leading back to B, then down through the minor scale to B.
        let tune = [
            (59, 8_000),
            (62, 4_000),
            (66, 4_000),
            (71, 8_000),
            (70, 2_000),
            (71, 6_000),
            (69, 4_000),
            (67, 4_000),
            (66, 4_000),
            (64, 4_000),
            (62, 4_000),
            (61, 4_000),
            (58, 2_000),
            (59, 12_000),
        ];
        let estimate = estimate_key(&melody(&tune), None).unwrap();
        assert_eq!(
            estimated(&estimate),
            key("B minor"),
            "{:?}",
            estimate.scores
        );
        assert_eq!(estimate.alternative, key("D major"));
    }

    #[test]
    fn a_given_key_is_used_exactly_and_the_evidence_is_still_kept() {
        let c_major = melody(&[
            (60, 4_000),
            (62, 4_000),
            (64, 4_000),
            (65, 4_000),
            (67, 8_000),
        ]);
        let estimate = estimate_key(&c_major, Some(key("Eb major"))).unwrap();
        assert_eq!(estimate.key, Origin::Given(key("Eb major")));
        assert_eq!(estimate.alternative, key("C minor"));
        assert_eq!(estimate.scores.len(), 24);
        assert!((estimate.histogram.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(
            (estimate.histogram[7] - 8.0 / 24.0).abs() < 1e-9,
            "{:?}",
            estimate.histogram
        );
    }

    #[test]
    fn every_major_and_minor_key_is_scored() {
        let estimate = estimate_key(&melody(&[(60, 4_000), (64, 4_000)]), None).unwrap();
        let keys: Vec<Key> = estimate.scores.iter().map(|(k, _)| *k).collect();
        for tonic in [
            "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
        ] {
            assert!(
                keys.contains(&key(&format!("{tonic} major"))),
                "{tonic} major"
            );
        }
        for tonic in [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "Bb", "B",
        ] {
            assert!(
                keys.contains(&key(&format!("{tonic} minor"))),
                "{tonic} minor"
            );
        }
    }

    #[test]
    fn no_notes_have_no_key() {
        let onsets = [Event::Onset {
            frame: 10,
            strength: 0.5,
        }];
        assert_eq!(estimate_key(&onsets, None), None);
        assert_eq!(estimate_key(&[], Some(key("D minor"))), None);
    }
}
