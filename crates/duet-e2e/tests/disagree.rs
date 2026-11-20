//! `duet capture disagree`: the places two readings of a take disagree, found by kind
//! (`docs/superpowers/specs/2026-11-20-reader-disagreement-design.md`, witnesses W1, W2, W3, W5
//! and W6; W4 is `duet-dsp`'s unit tests, W7 `capture_default.rs`).
//!
//! The takes are the corpus's: `synthetic-disagree.flac` (the witness take, MIT, from
//! `scripts/capture-fixtures.sh`), `synthetic-melody.flac` (MIT) and "Wayfaring Stranger" (CC BY
//! 4.0). Every reading is written to a scratch directory.

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, ok, root, s, scratch};

fn corpus(name: &str) -> PathBuf {
    root().join("crates/duet-capture/corpus").join(name)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `take` captured into `dir` twice, as `STEM` by the HMM and `STEM-rules` by the rules, each
/// with `given` too. Returns the HMM's stem.
fn both_readings(dir: &Path, take: &Path, stem: &str, given: &[&str]) -> PathBuf {
    let hmm = dir.join(stem);
    let rules = dir.join(format!("{stem}-rules"));
    for (out, reader) in [(&hmm, "hmm"), (&rules, "rules")] {
        let mut args = vec![
            "capture",
            "melody",
            s(take),
            "--reader",
            reader,
            "-o",
            s(out),
        ];
        args.extend_from_slice(given);
        ok(
            &duet(dir, &args),
            &format!("capture melody --reader {reader}"),
        );
    }
    hmm
}

fn analysis(stem: &Path) -> PathBuf {
    let mut path = stem.as_os_str().to_owned();
    path.push(".capture.duet");
    PathBuf::from(path)
}

fn rules_of(stem: &Path) -> String {
    let mut rules = stem.as_os_str().to_owned();
    rules.push("-rules");
    format!("rules={}", s(&analysis(Path::new(&rules))))
}

/// `duet capture disagree` on the HMM's stem against the rules' reading.
fn disagree(dir: &Path, take: &Path, stem: &Path, more: &[&str]) -> Output {
    let rules = rules_of(stem);
    let mut args = vec![
        "capture",
        "disagree",
        s(take),
        "-o",
        s(stem),
        "--reading",
        &rules,
    ];
    args.extend_from_slice(more);
    duet(dir, &args)
}

/// The moments in a moments file, as `(from, to, label)`.
fn moments(path: &Path) -> Vec<(f64, f64, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut parts = l.splitn(3, ' ');
            let mut next = || parts.next().expect("a field");
            let from = next().parse().expect("seconds");
            let to = next().parse().expect("seconds");
            (from, to, next().to_owned())
        })
        .collect()
}

fn moments_of(stem: &Path) -> PathBuf {
    let mut path = stem.as_os_str().to_owned();
    path.push(".disagree.moments");
    PathBuf::from(path)
}

/// Each note of an analysis's reading, as (key, centre in cents): for a failure message.
fn centres(analysis: &Path) -> Vec<(u64, f64)> {
    let text = std::fs::read_to_string(analysis).expect("an analysis");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    doc["body"]["reading"]["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .map(|n| {
            (
                n["key"].as_u64().expect("a key"),
                n["centre"].as_f64().expect("a centre"),
            )
        })
        .collect()
}

/// W1. The witness take's places: a split at the turn, the HMM's G3 read by the rules as G3,
/// A3, G3, and a key at the midpoint note, G3 against G#3; nothing on the plain notes.
///
/// **Fragile by design.** The key case holds only where the two readers' medians of the note
/// sung at +50 cents fall either side of the G3/G#3 midpoint, as at +50 and +51 cents when this
/// was written. A reader change that moves either median by a few cents ends the disagreement;
/// the failure names both readers' centres, so the cause is visible. W4 carries the
/// classification exactly, on readings built by hand.
#[test]
fn the_witness_take_has_a_split_at_the_turn_and_a_key_at_the_midpoint() {
    let dir = scratch("disagree-witness");
    let take = corpus("synthetic-disagree.flac");
    let stem = both_readings(&dir, &take, "sd", &["--tuning", "440", "--tempo", "120"]);
    let out = disagree(&dir, &take, &stem, &[]);
    ok(&out, "capture disagree");
    let found = moments(&moments_of(&stem));
    let labels: Vec<&str> = found.iter().map(|(_, _, l)| l.as_str()).collect();
    let why = || {
        format!(
            "{labels:?}; the HMM's notes (key, centre) {:?}, the rules' {:?}",
            centres(&analysis(&stem)),
            centres(&analysis(&dir.join("sd-rules")))
        )
    };
    assert_eq!(found.len(), 2, "two places: {}", why());
    assert!(
        labels[0].starts_with("split: hmm G3 at ")
            && labels[0].ends_with(" is rules G3 A3 G3, its key among them"),
        "{}",
        why()
    );
    assert!(
        labels[1].starts_with("key: hmm G3 at ") && labels[1].ends_with(", rules G#3"),
        "{}",
        why()
    );
    // The turn is in the G3 from 1.05 s, the midpoint note from 3.19 s.
    assert!(found[0].0 < 1.05 && found[0].1 > 2.1, "{found:?}");
    assert!(found[1].0 < 3.19 && found[1].1 > 3.79, "{found:?}");
    let said = stdout(&out);
    assert!(
        said.ends_with(
            "2 places: 1 split, 1 key; no stretch differs only in where notes start or end\n"
        ),
        "{said}"
    );
}

/// W2. Nothing where the readers agree: on the synthetic melody, nothing is found and no
/// moments file is written.
#[test]
fn nothing_is_found_where_the_readers_agree_and_no_moments_file_is_written() {
    let dir = scratch("disagree-agree");
    let take = corpus("synthetic-melody.flac");
    let stem = both_readings(&dir, &take, "sm", &["--tempo", "120"]);
    let out = disagree(&dir, &take, &stem, &[]);
    ok(&out, "capture disagree");
    assert_eq!(
        stdout(&out),
        "nothing found: no stretch differs only in where notes start or end; no moments file \
         is written\n"
    );
    assert!(!moments_of(&stem).exists());
}

/// W3. "Wayfaring Stranger", captured with nothing given: the four places of the design, with
/// their kinds, and 16 stretches that differ only in their boundaries. They are pinned as they
/// read when `disagree` landed: a reader change that moves them fails by name, and is ruled by
/// ear, not by editing these lines.
#[test]
fn wayfaring_stranger_has_the_four_places_of_the_design() {
    let dir = scratch("disagree-ws");
    let take = corpus("wayfaring-stranger.flac");
    let stem = both_readings(&dir, &take, "ws", &[]);
    let out = disagree(&dir, &take, &stem, &[]);
    ok(&out, "capture disagree");
    assert_eq!(
        stdout(&out),
        "0.91 1.79 split: hmm D#3 at 1.13 is rules D#3 E3, its key among them\n\
         4.64 5.21 key: hmm F3 at 4.79, rules A3\n\
         11.80 13.51 only: hmm C4 at 13.22\n\
         20.03 20.46 only: hmm D3 at 20.18\n\
         4 places: 1 split, 1 key, 2 only; 16 stretches differ only in where notes start or \
         end, by up to 0.15 s\n"
    );
    let text = std::fs::read_to_string(moments_of(&stem)).expect("the moments file");
    for comment in [
        "# hmm: D#3 | rules: D#3 E3\n",
        "# hmm: F3 | rules: A3\n",
        "# hmm: A3 A#3 C4 | rules: A3 A#3\n",
        "# hmm: D3 | rules: none\n",
    ] {
        assert!(text.contains(comment), "{comment:?} in {text}");
    }
    assert!(
        text.starts_with("# disagree: hmm (")
            && text.lines().next().unwrap().ends_with(
                "take sha256:18148cfcdb5331d7f4c4a9e0e5b00383f463beda7c8c820da872872af1ec7cae"
            ),
        "{text}"
    );
}

/// Asserts `out` failed, naming `what` on standard error, with no moments file at `stem`.
fn refused(out: &Output, stem: &Path, what: &str) {
    assert!(!out.status.success(), "refused: {}", stdout(out));
    let said = stderr(out);
    assert!(said.contains(what), "{what:?} in {said}");
    assert!(!moments_of(stem).exists(), "nothing written");
}

/// W5. Refusals, before anything is written: readings of different takes, a reading that
/// states no take, a reading whose notes overlap, two tunings, a name given twice, a moments file that exists, and a missing
/// second reading.
#[test]
fn readings_that_cannot_be_compared_are_refused_by_name_before_anything_is_written() {
    let dir = scratch("disagree-refusals");
    let take = corpus("synthetic-disagree.flac");
    let stem = both_readings(&dir, &take, "sd", &["--tuning", "440", "--tempo", "120"]);

    // Another take's reading, on the same tuning.
    let other = corpus("synthetic-melody.flac");
    ok(
        &duet(
            &dir,
            &[
                "capture",
                "melody",
                s(&other),
                "--reader",
                "rules",
                "--tuning",
                "440",
                "-o",
                s(&dir.join("sm")),
            ],
        ),
        "capture the other take",
    );
    let sm = format!("rules={}", s(&analysis(&dir.join("sm"))));
    let out = duet(
        &dir,
        &[
            "capture",
            "disagree",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &sm,
        ],
    );
    refused(&out, &stem, "they are not readings of one take");
    assert!(stderr(&out).contains("sm.capture.duet"), "{}", stderr(&out));

    // A reading that states no take: the same analysis with its take taken out.
    let text = std::fs::read_to_string(analysis(&dir.join("sd-rules"))).expect("an analysis");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    doc["body"].as_object_mut().expect("a body").remove("take");
    let untaken = dir.join("untaken.capture.duet");
    std::fs::write(&untaken, serde_json::to_string_pretty(&doc).unwrap()).expect("write");
    let reading = format!("rules={}", s(&untaken));
    let out = duet(
        &dir,
        &[
            "capture",
            "disagree",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &reading,
        ],
    );
    refused(&out, &stem, "untaken.capture.duet states no take");

    // A reading whose notes overlap, as no reader reads them.
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let notes = doc["body"]["reading"]["notes"]
        .as_array_mut()
        .expect("notes");
    let second_start = notes[1]["start"].as_u64().expect("a start");
    notes[0]["end"] = serde_json::json!(second_start + 10);
    let overlapping = dir.join("overlapping.capture.duet");
    std::fs::write(&overlapping, serde_json::to_string_pretty(&doc).unwrap()).expect("write");
    let reading = format!("rules={}", s(&overlapping));
    let out = duet(
        &dir,
        &[
            "capture",
            "disagree",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &reading,
        ],
    );
    refused(
        &out,
        &stem,
        "overlapping.capture.duet: its notes are not in order",
    );

    // Two tunings: the rules' reading with the tuning estimated.
    ok(
        &duet(
            &dir,
            &[
                "capture",
                "melody",
                s(&take),
                "--reader",
                "rules",
                "--tempo",
                "120",
                "-o",
                s(&dir.join("estimated")),
            ],
        ),
        "capture with the tuning estimated",
    );
    let reading = format!("rules={}", s(&analysis(&dir.join("estimated"))));
    let out = duet(
        &dir,
        &[
            "capture",
            "disagree",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &reading,
        ],
    );
    refused(&out, &stem, "(given) and ");
    assert!(
        stderr(&out).contains("(estimated): a key is only comparable on one tuning")
            && stderr(&out).contains("--tuning 440.00"),
        "{}",
        stderr(&out)
    );

    // A name given twice: the stem's own reading is the HMM's.
    let reading = format!("hmm={}", s(&analysis(&dir.join("sd-rules"))));
    let out = duet(
        &dir,
        &[
            "capture",
            "disagree",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &reading,
        ],
    );
    refused(&out, &stem, "two readings are named hmm");

    // No second reading.
    let out = duet(&dir, &["capture", "disagree", s(&take), "-o", s(&stem)]);
    refused(&out, &stem, "give the other with --reading NAME=ANALYSIS");

    // A moments file that exists is never overwritten.
    let moments = moments_of(&stem);
    std::fs::write(&moments, "kept\n").expect("write");
    let out = disagree(&dir, &take, &stem, &[]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("sd.disagree.moments exists"),
        "{}",
        stderr(&out)
    );
    assert_eq!(std::fs::read_to_string(&moments).unwrap(), "kept\n");
}

/// W6. The file reaches the listening: `disagree` then `review` on the witness take writes a
/// page of two moments with both readings, each moment's label starting with its kind.
#[test]
fn the_places_reach_review_as_moments_labelled_by_kind() {
    let dir = scratch("disagree-review");
    let take = corpus("synthetic-disagree.flac");
    let stem = both_readings(&dir, &take, "sd", &["--tuning", "440", "--tempo", "120"]);
    ok(&disagree(&dir, &take, &stem, &[]), "capture disagree");
    let sheet = dir.join("sheet");
    let rules = rules_of(&stem);
    let out = duet(
        &dir,
        &[
            "capture",
            "review",
            s(&take),
            "-o",
            s(&stem),
            "--reading",
            &rules,
            "--moments",
            s(&moments_of(&stem)),
            "--dir",
            s(&sheet),
        ],
    );
    ok(&out, "capture review");
    assert!(
        stdout(&out).contains(": 2 moments, 2 readings (hmm, rules), "),
        "{}",
        stdout(&out)
    );
    let page = std::fs::read_to_string(sheet.join("index.html")).expect("the page");
    let labels: Vec<&str> = page
        .split("data-label=\"")
        .skip(1)
        .map(|rest| &rest[..rest.find('"').expect("a closing quote")])
        .collect();
    assert_eq!(labels.len(), 2, "{labels:?}");
    assert!(labels[0].starts_with("split: "), "{labels:?}");
    assert!(labels[1].starts_with("key: "), "{labels:?}");
}

/// In a project, with no -o and no --reading, the take's two readings are compared and the
/// moments file goes in its directory; a take with one reading is refused, naming it.
#[test]
fn in_a_project_the_takes_two_readings_are_compared() {
    let dir = scratch("disagree-project");
    ok(&duet(&dir, &["init", "song"]), "init");
    let root = dir.join("song");
    let source = dir.join("synthetic-disagree.flac");
    std::fs::copy(corpus("synthetic-disagree.flac"), &source).expect("copy");
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");
    let given = ["--tuning", "440", "--tempo", "120"];
    let mut args = vec!["capture", "melody", "synthetic-disagree"];
    args.extend_from_slice(&given);
    ok(&duet(&root, &args), "capture melody");

    let out = duet(&root, &["capture", "disagree", "synthetic-disagree"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("has one reading in takes/synthetic-disagree/, hmm"),
        "{}",
        stderr(&out)
    );

    args.extend_from_slice(&["--reader", "rules"]);
    ok(&duet(&root, &args), "capture melody --reader rules");
    let out = duet(&root, &["capture", "disagree", "synthetic-disagree"]);
    ok(&out, "capture disagree in a project");
    let moments = root.join("takes/synthetic-disagree/synthetic-disagree.disagree.moments");
    let found = self::moments(&moments);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].2.starts_with("split: hmm G3"), "{found:?}");
}
