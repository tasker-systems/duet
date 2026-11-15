//! The capture reference corpus: each take in `corpus/manifest.toml` goes through the library
//! pipeline and must match its hand-checked transcription.
//!
//! Clauses (goal): `a-sung-melody-file-reaches-abc`, `a-tapped-rhythm-file-reaches-a-drum-grid`
//! and `a-beatboxed-hit-lands-in-the-drum-it-imitates`. See `support/mod.rs` for the rules.

mod support;

use duet_capture::{Loss, ReaderName};
use support::{
    TakeKind, Verdict, asked_key, compare_melody, compare_rhythm, manifest, played, read, read_by,
};

/// One take's verdict; a refusal to run it at all is a failure too.
fn judge(take: &support::Take) -> Verdict {
    let name = &take.file;
    let run = || -> Result<Verdict, String> {
        let reading = read(take)?;
        let expected = played(&reading.expected, reading.grid)?;
        let found = played(&reading.found, reading.grid)?;
        Ok(match take.kind {
            TakeKind::Melody => compare_melody(name, &expected, &found),
            TakeKind::Tapped | TakeKind::Beatboxed => {
                let asked = if take.kind == TakeKind::Tapped {
                    asked_key(take)?
                } else {
                    None
                };
                compare_rhythm(name, &expected, &found, asked)
            }
        })
    };
    run().unwrap_or_else(|e| Verdict {
        failures: vec![format!("{name}: {e}")],
        reported: Vec::new(),
    })
}

#[test]
fn every_reference_take_matches_its_transcription() {
    let manifest = manifest();
    let mut failures = Vec::new();
    for take in &manifest.take {
        let verdict = judge(take);
        for line in &verdict.reported {
            eprintln!("reported: {line}");
        }
        failures.extend(verdict.failures);
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The loss's three terms, as a row of the per-take table.
fn row(loss: &Loss) -> String {
    format!(
        "pitch {:8.1}  unread {:6.1}  notes {:6.1}  total {:8.1}",
        loss.pitch,
        loss.unread,
        loss.notes,
        loss.total()
    )
}

/// The note HMM must match every melody take's transcription as the rules do, and both
/// readings' losses are reported per take, as their three terms, with each term the HMM is
/// worse on named.
#[test]
fn the_note_hmm_matches_every_melody_transcription_and_both_losses_are_reported() {
    let manifest = manifest();
    let mut failures = Vec::new();
    for take in manifest.take.iter().filter(|t| t.kind == TakeKind::Melody) {
        let name = &take.file;
        let mut losses = Vec::new();
        for reader in [ReaderName::Rules, ReaderName::Hmm] {
            let (reading, read) = match read_by(take, reader) {
                Ok(both) => both,
                Err(e) => {
                    failures.push(format!("{name} ({reader:?}): {e}"));
                    continue;
                }
            };
            let loss = read
                .and_then(|r| r.residual)
                .map(|r| (r.loss, r.read_frames, r.unread_frames));
            if reader == ReaderName::Hmm {
                let found = played(&reading.found, reading.grid);
                let expected = played(&reading.expected, reading.grid);
                match (expected, found) {
                    (Ok(expected), Ok(found)) => {
                        let verdict = compare_melody(name, &expected, &found);
                        failures.extend(verdict.failures.iter().map(|f| format!("hmm: {f}")));
                    }
                    (Err(e), _) | (_, Err(e)) => failures.push(format!("{name}: {e}")),
                }
            }
            losses.extend(loss.map(|l| (reader, l)));
        }
        for (reader, (loss, read, unread)) in &losses {
            eprintln!(
                "loss: {name:<24} {:<5}  {}  (frames read {read}, unread {unread})",
                format!("{reader:?}").to_lowercase(),
                row(loss)
            );
        }
        if let [(_, (rules, ..)), (_, (hmm, ..))] = losses.as_slice() {
            let worse: Vec<&str> = [
                ("pitch", hmm.pitch > rules.pitch),
                ("unread", hmm.unread > rules.unread),
                ("notes", hmm.notes > rules.notes),
            ]
            .into_iter()
            .filter_map(|(term, worse)| worse.then_some(term))
            .collect();
            eprintln!(
                "loss: {name:<24} the hmm is worse on: {}",
                if worse.is_empty() {
                    "nothing".to_owned()
                } else {
                    worse.join(", ")
                }
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "awaiting the owner's reference takes: melody, tapped, beatboxed"]
fn every_kind_has_a_reference_take() {
    let manifest = manifest();
    let missing: Vec<&str> = TakeKind::ALL
        .into_iter()
        .filter(|kind| {
            !manifest
                .take
                .iter()
                .any(|take| take.kind == *kind && !take.is_synthetic())
        })
        .map(TakeKind::name)
        .collect();
    assert!(
        missing.is_empty(),
        "no non-synthetic reference take of kind: {}",
        missing.join(", ")
    );
}

/// The comparisons themselves: the corpus has no rhythm take yet to show they bite.
mod comparisons {
    use super::support::{Played, compare_melody, compare_rhythm};

    fn p(step: u64, midi: u8, beats: f64) -> Played {
        Played {
            step,
            midi,
            milli_beats: (beats * 1000.0).round() as u64,
        }
    }

    #[test]
    fn a_missing_hit_fails_and_names_its_step_and_key() {
        let v = compare_rhythm(
            "t",
            &[p(0, 36, 0.25), p(4, 38, 0.25)],
            &[p(0, 36, 0.25)],
            None,
        );
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("expected key 38 at step 4"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn an_extra_hit_fails() {
        let v = compare_rhythm(
            "t",
            &[p(0, 36, 0.25)],
            &[p(0, 36, 0.25), p(2, 42, 0.25)],
            None,
        );
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("extra hit: found key 42 at step 2"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn a_hit_in_the_wrong_lane_fails_as_missing_and_extra() {
        let v = compare_rhythm("t", &[p(0, 36, 0.25)], &[p(0, 38, 0.25)], None);
        assert_eq!(v.failures.len(), 2, "{:?}", v.failures);
    }

    #[test]
    fn a_tapped_hit_off_the_asked_lane_fails_even_when_the_transcription_agrees() {
        let v = compare_rhythm("t", &[p(0, 38, 0.25)], &[p(0, 38, 0.25)], Some(42));
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(v.failures[0].contains("asked lane"), "{:?}", v.failures);
    }

    #[test]
    fn a_short_melody_note_is_reported_not_failed() {
        let v = compare_melody(
            "t",
            &[p(0, 60, 1.0), p(4, 62, 0.25)],
            &[p(0, 60, 1.0), p(4, 63, 0.25)],
        );
        assert!(v.failures.is_empty(), "{:?}", v.failures);
        assert_eq!(v.reported.len(), 1, "{:?}", v.reported);
    }

    #[test]
    fn a_missing_long_melody_note_fails() {
        let v = compare_melody("t", &[p(0, 60, 1.0), p(4, 62, 1.0)], &[p(0, 60, 1.0)]);
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("expected note: pitch 62 at step 4"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn an_extra_short_note_in_the_middle_fails_no_later_long_note() {
        let expected = [p(0, 60, 1.0), p(4, 62, 1.0), p(8, 64, 1.0)];
        let found = [p(0, 60, 1.0), p(2, 61, 0.25), p(4, 62, 1.0), p(8, 64, 1.0)];
        let v = compare_melody("t", &expected, &found);
        assert!(v.failures.is_empty(), "{:?}", v.failures);
        assert_eq!(v.reported.len(), 1, "{:?}", v.reported);
    }

    #[test]
    fn a_long_note_of_the_wrong_pitch_over_an_expected_short_note_fails() {
        let v = compare_melody(
            "t",
            &[p(0, 60, 1.0), p(4, 62, 0.25)],
            &[p(0, 60, 1.0), p(4, 63, 1.0)],
        );
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("extra note: found pitch 63 at step 4"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn a_second_long_note_on_a_matched_step_fails() {
        let v = compare_melody("t", &[p(0, 60, 1.0)], &[p(0, 60, 1.0), p(0, 60, 1.0)]);
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("extra note: found pitch 60 at step 0"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn duplicate_expected_notes_need_a_found_note_each() {
        let v = compare_melody("t", &[p(0, 60, 1.0), p(0, 60, 1.0)], &[p(0, 60, 1.0)]);
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("expected note: pitch 60 at step 0"),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn an_extra_long_note_fails_as_invented_or_split() {
        let v = compare_melody("t", &[p(0, 60, 2.0)], &[p(0, 60, 1.0), p(4, 60, 1.0)]);
        assert_eq!(v.failures.len(), 1, "{:?}", v.failures);
        assert!(
            v.failures[0].contains("extra note: found pitch 60 at step 4"),
            "{:?}",
            v.failures
        );
    }
}

/// Manifest entries are checked, and a bad one names its take.
mod entries {
    use super::support::Take;

    fn take(kind: &str, lane: &str) -> Take {
        let text = format!(
            "file = \"x.flac\"\nkind = \"{kind}\"\nprovenance = \"p\"\nlicence = \"l\"\nexpected = \"x.notes\"\n{lane}"
        );
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn a_tapped_take_needs_a_lane() {
        let e = take("tapped", "").check().unwrap_err();
        assert!(e.contains("tapped take needs `lane`"), "{e}");
        assert!(take("tapped", "lane = \"hh\"").check().is_ok());
    }

    #[test]
    fn a_lane_is_only_for_a_tapped_take() {
        for kind in ["melody", "beatboxed"] {
            let e = take(kind, "lane = \"hh\"").check().unwrap_err();
            assert!(e.contains(&format!("a {kind} take has none")), "{e}");
        }
    }
}
