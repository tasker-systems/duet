//! The key of a melody take, used as given or estimated from its notes.
//!
//! The estimate is Krumhansl–Schmuckler key finding. The notes' pitch classes are counted into
//! a histogram, each note weighted by its length, and the histogram is correlated (Pearson's r)
//! with a [`KeyProfile`]'s major and minor profiles turned to each of the twelve tonics. The
//! best of those 24 keys is the estimate, and its correlation, clamped to 0..=1, is its
//! confidence. The profile is Aarden–Essen's, fitted to folk melodies, unless another is asked
//! for. v1 covers major and minor only; the modes come later.
//!
//! The key changes a part's spelling and signature, never a note's MIDI number.

use duet_core::{Key, Mode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::timing::Origin;

/// The profiles a key is estimated with: for each pitch class, from the tonic up in semitones,
/// how strongly it belongs to a major key and to a minor key. Only their shape matters, since
/// the estimate is a correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum KeyProfile {
    /// Aarden–Essen: how often each scale degree sounds in the major and minor folk songs of
    /// the Essen collection. Bret Aarden, *Dynamic Melodic Expectancy*
    /// (PhD dissertation, Ohio State University, 2003); the values are those of Craig Sapp's
    /// Humdrum Extras `keycor` (`majorKeyAarden`, `minorKeyAarden`), the same as music21's
    /// `analysis.discrete.AardenEssen` (BSD-3-Clause). Fitted to melodies, it weighs a missing
    /// third heavily, so a sung minor tune whose fourth is held longest still reads minor on its
    /// tonic. The default since `capture-analysis` 1.12.
    #[default]
    AardenEssen,
    /// Krumhansl–Kessler: how well listeners rated each pitch class to fit a key after a
    /// cadence. Carol L. Krumhansl and Edward J. Kessler, "Tracing the dynamic changes in
    /// perceived tonal organization in a spatial representation of musical keys",
    /// *Psychological Review* 89(4), 1982; also Krumhansl, *Cognitive Foundations of Musical
    /// Pitch* (1990), p. 35. duet's profile before 1.12, kept for comparison.
    KrumhanslKessler,
}

impl KeyProfile {
    /// The major-key profile, from the tonic up in semitones.
    #[must_use]
    pub const fn major(self) -> [f64; 12] {
        match self {
            Self::AardenEssen => [
                17.7661, 0.145624, 14.9265, 0.160186, 19.8049, 11.3587, 0.291248, 22.062, 0.145624,
                8.15494, 0.232998, 4.95122,
            ],
            Self::KrumhanslKessler => [
                6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
            ],
        }
    }

    /// The minor-key profile, the same way.
    #[must_use]
    pub const fn minor(self) -> [f64; 12] {
        match self {
            Self::AardenEssen => [
                18.2648, 0.737619, 14.0499, 16.8599, 0.702494, 14.4362, 0.702494, 18.6161, 4.56621,
                1.93186, 7.37619, 1.75623,
            ],
            Self::KrumhanslKessler => [
                6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
            ],
        }
    }
}

/// A take's key, given or estimated, with the evidence the estimate was read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    /// The profile the scores were correlated with. Added in `capture-analysis` 1.12. A
    /// document without it states no profile; duet wrote none before 1.12, when it always
    /// estimated with [`KeyProfile::KrumhanslKessler`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<KeyProfile>,
}

/// The key of the [`Event::Note`]s among `notes` (other events are ignored): `given` when
/// there is one, used exactly, or else the estimate. The histogram and the scores are filled
/// either way.
///
/// `None` when there are no notes, as for a rhythm take. The profile is the default,
/// [`KeyProfile::AardenEssen`]; [`estimate_key_with`] takes another.
pub fn estimate_key(notes: &[Event], given: Option<Key>) -> Option<KeyEstimate> {
    estimate_key_with(notes, given, KeyProfile::default())
}

/// [`estimate_key`], correlating with `profile`'s major and minor profiles.
pub fn estimate_key_with(
    notes: &[Event],
    given: Option<Key>,
    profile: KeyProfile,
) -> Option<KeyEstimate> {
    let histogram = histogram(notes)?;
    let scores: Vec<(Key, f64)> = (0..12)
        .flat_map(|tonic| {
            [
                (profile.major(), Mode::Major),
                (profile.minor(), Mode::Minor),
            ]
            .map(|(profile, mode)| {
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
        profile: Some(profile),
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
/// of fewer accidentals (Db major, not C# major). At the tie of six, a major key takes sharps
/// (F# major) and a minor key flats (Eb minor, not D# minor), the usual spellings of each.
fn key_on(tonic: usize, mode: Mode) -> Key {
    // A minor key shares its signature with the major a minor third above it.
    let major = if mode == Mode::Minor {
        tonic + 3
    } else {
        tonic
    };
    // Each step round the circle of fifths is 7 semitones, and adds a sharp.
    let sharps = (major * 7 % 12) as i8;
    let flats = sharps > 6 || (sharps == 6 && mode == Mode::Minor);
    Key {
        sharps: if flats { sharps - 12 } else { sharps },
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

    /// Eb F G Ab Bb C D Eb, up and back down, the tonic held longest.
    fn e_flat_major_scale() -> Vec<Event> {
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
        melody(&notes)
    }

    #[test]
    fn the_aarden_essen_profiles_are_the_published_ones() {
        // Humdrum Extras `keycor.cpp`, `majorKeyAarden` and `minorKeyAarden` ("from Aarden's
        // dissertation"), C up to B; music21's `AardenEssen.getWeights` gives the same.
        assert_eq!(
            KeyProfile::AardenEssen.major(),
            [
                17.7661, 0.145624, 14.9265, 0.160186, 19.8049, 11.3587, 0.291248, 22.062, 0.145624,
                8.15494, 0.232998, 4.95122,
            ]
        );
        assert_eq!(
            KeyProfile::AardenEssen.minor(),
            [
                18.2648, 0.737619, 14.0499, 16.8599, 0.702494, 14.4362, 0.702494, 18.6161, 4.56621,
                1.93186, 7.37619, 1.75623,
            ]
        );
        assert_eq!(KeyProfile::default(), KeyProfile::AardenEssen);
    }

    #[test]
    fn the_krumhansl_kessler_profiles_are_the_published_ones() {
        // Krumhansl, *Cognitive Foundations of Musical Pitch* (1990), p. 35, C up to B, as
        // `keycor.cpp`'s `majorKeyKrumhansl` and `minorKeyKrumhansl` give them.
        assert_eq!(
            KeyProfile::KrumhanslKessler.major(),
            [
                6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88
            ]
        );
        assert_eq!(
            KeyProfile::KrumhanslKessler.minor(),
            [
                6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17
            ]
        );
    }

    #[test]
    fn an_estimate_states_its_profile() {
        let notes = e_flat_major_scale();
        assert_eq!(
            estimate_key(&notes, None).unwrap().profile,
            Some(KeyProfile::AardenEssen)
        );
        assert_eq!(
            estimate_key_with(&notes, None, KeyProfile::KrumhanslKessler)
                .unwrap()
                .profile,
            Some(KeyProfile::KrumhanslKessler)
        );
        assert_eq!(
            estimate_key(&notes, Some(key("D major"))).unwrap().profile,
            Some(KeyProfile::AardenEssen),
            "a given key's scores are still the default profile's"
        );
    }

    #[test]
    fn an_e_flat_major_tune_is_estimated_as_e_flat_major() {
        // Eb G Bb G, Ab Bb C Bb, Ab G F D, Eb: the tonic triad leads, and it ends on the tonic.
        let tune = [
            (63, 8_000),
            (67, 4_000),
            (70, 4_000),
            (67, 4_000),
            (68, 4_000),
            (70, 8_000),
            (72, 4_000),
            (70, 4_000),
            (68, 4_000),
            (67, 4_000),
            (65, 4_000),
            (62, 2_000),
            (63, 12_000),
        ];
        let estimate = estimate_key(&melody(&tune), None).unwrap();
        assert_eq!(
            estimated(&estimate),
            key("Eb major"),
            "{:?}",
            estimate.scores
        );
        assert_eq!(estimate.alternative, key("C minor"));
    }

    #[test]
    fn an_e_flat_major_scale_has_the_e_flat_signature_under_either_profile() {
        // Krumhansl–Kessler reads the scale as E-flat major. Aarden–Essen, fitted to melodies
        // that dwell on the tonic triad, reads an evenly walked scale as its relative, C minor
        // (0.782 against 0.775): every degree but the tonic is as long, so nothing marks E-flat
        // as the tonic but its two longer notes. Either way the signature, and so every note's
        // spelling, is E-flat's, and the other reading is the alternative.
        let notes = e_flat_major_scale();
        let kk = estimate_key_with(&notes, None, KeyProfile::KrumhanslKessler).unwrap();
        assert_eq!(estimated(&kk), key("Eb major"), "{:?}", kk.scores);
        assert_eq!(kk.alternative, key("C minor"));
        let ae = estimate_key(&notes, None).unwrap();
        assert_eq!(estimated(&ae), key("C minor"), "{:?}", ae.scores);
        assert_eq!(ae.alternative, key("Eb major"));
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
            "C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B",
        ] {
            assert!(
                keys.contains(&key(&format!("{tonic} minor"))),
                "{tonic} minor"
            );
        }
    }

    #[test]
    fn six_accidentals_are_f_sharp_major_and_e_flat_minor() {
        assert_eq!(key_on(6, Mode::Major), key("F# major"));
        assert_eq!(key_on(3, Mode::Minor), key("Eb minor"));
        assert_eq!(relative(key("Eb minor")), key("Gb major"));
        assert_eq!(relative(key("F# major")), key("D# minor"));
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
