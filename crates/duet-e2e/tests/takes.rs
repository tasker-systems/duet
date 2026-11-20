//! Takes in a project (the project design's §4, build task 3): `duet take add | locate | list |
//! forget`, `.duet/takes.local.toml`, a take's `given`, and `duet record` into a project.
//!
//! Every take here is audio the test writes itself: nothing comes from `fixtures/`.

#[path = "capture/audio.rs"]
#[expect(
    dead_code,
    reason = "the capture tests' shared generators: these tests use the clicks alone"
)]
mod audio;

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, ok, s, scratch};
use duet_schemas::{Document, Project};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A fresh project at `dir/name`.
fn project(dir: &Path, name: &str) -> PathBuf {
    ok(&duet(dir, &["init", name]), "init");
    dir.join(name)
}

fn read_project(root: &Path) -> Project {
    let text = std::fs::read_to_string(root.join(".duet/project.duet")).unwrap();
    Document::<Project>::read(&text).unwrap().body
}

/// Clicks at `bpm` for 16 beats: a take a rhythm capture reads.
fn clicks(bpm: f64) -> Vec<f32> {
    let period = 60.0 / bpm;
    let times: Vec<(f64, f32)> = (0..16)
        .map(|n| (0.5 + f64::from(n) * period, 0.6))
        .collect();
    audio::clicks(0.5 + 16.0 * period + 1.0, &times)
}

/// A take written to `dir/name`.
fn wav(dir: &Path, name: &str, samples: &[f32]) -> PathBuf {
    let path = dir.join(name);
    audio::write_wav(&path, samples);
    path
}

/// Every line of the project's journal segments.
fn journal(root: &Path) -> String {
    let mut out = String::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("journal.") && name != "journal.duet.jsonl" {
            out.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    out
}

#[test]
fn take_add_copies_the_file_byte_for_byte_and_journals_the_take() {
    let dir = scratch("take-add");
    let root = project(&dir, "porch");
    let source = wav(&dir, "Idea One.wav", &clicks(100.0));
    let bytes = std::fs::read(&source).unwrap();

    let out = duet(&root, &["take", "add", s(&source)]);
    ok(&out, "take add");
    let copied = root.join("takes/idea-one/Idea One.wav");
    assert_eq!(
        std::fs::read(&copied).unwrap(),
        bytes,
        "copied byte for byte, never transcoded"
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes, "the source is left");
    assert!(
        stdout(&out).contains("copied to takes/idea-one/Idea One.wav"),
        "{}",
        stdout(&out)
    );

    let doc = read_project(&root);
    assert_eq!(doc.takes.len(), 1);
    let take = &doc.takes[0];
    assert_eq!(take.name.as_str(), "idea-one", "named from the file");
    assert_eq!(take.file.as_str(), "Idea One.wav");
    assert_eq!(take.take.channels, 1);
    assert_eq!(take.take.rate, audio::RATE);
    assert_eq!(take.take.frames, clicks(100.0).len() as u64);
    assert!(
        !root.join(".duet/takes.local.toml").exists(),
        "a copied take names no local path"
    );

    let journal = journal(&root);
    assert!(journal.contains("\"take.added\""), "{journal}");
    assert!(journal.contains(&take.take.digest.to_string()), "{journal}");
    assert!(journal.contains("\".duet/project.duet\""), "{journal}");

    let list = duet(&root, &["take", "list"]);
    ok(&list, "take list");
    assert!(
        stdout(&list).contains("idea-one (Idea One.wav)")
            && stdout(&list).contains("found at takes/idea-one/Idea One.wav"),
        "{}",
        stdout(&list)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A take a frame of which the decoder refuses: `take add` says what it reads as silence,
/// where and how long, and the take's frames count it, so what follows keeps its time.
/// `take list` says it again each time it checks the file. The file is made here, a block a
/// reserved subframe under a right CRC16 (`capture/audio.rs`).
#[test]
fn adding_a_take_with_an_undecodable_frame_says_and_states_the_gap() {
    let dir = scratch("take-add-gapped");
    let root = project(&dir, "porch");
    let flac = dir.join("gapped.flac");
    audio::write_flac_block_refused(&flac, 16, 3);

    let out = duet(&root, &["take", "add", s(&flac)]);
    ok(&out, "take add");
    let gap = "186 ms of audio at 0:00.6 could not be decoded and is read as silence";
    assert!(stderr(&out).contains(gap), "{}", stderr(&out));

    let doc = read_project(&root);
    let take = &doc.takes[0].take;
    assert_eq!((take.rate, take.channels, take.frames), (22_050, 1, 65_536));

    let list = duet(&root, &["take", "list"]);
    ok(&list, "take list");
    assert!(stderr(&list).contains(gap), "{}", stderr(&list));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_named_take_copies_nothing_and_is_found_through_the_local_map() {
    let dir = scratch("take-named");
    let root = project(&dir, "porch");
    let source = wav(&dir, "phone.wav", &clicks(110.0));
    let out = duet(
        &root,
        &["take", "add", s(&source), "--named", "--name", "phone"],
    );
    ok(&out, "take add --named");
    assert!(!root.join("takes").exists(), "nothing is copied");
    let local: toml::Table =
        toml::from_str(&std::fs::read_to_string(root.join(".duet/takes.local.toml")).unwrap())
            .unwrap();
    assert_eq!(local["phone"]["path"].as_str(), Some(s(&source)));

    let list = duet(&root, &["take", "list"]);
    ok(&list, "take list");
    assert!(
        stdout(&list).contains(&format!(
            "found at {}, named in .duet/takes.local.toml",
            source.display()
        )),
        "{}",
        stdout(&list)
    );

    // Found again elsewhere: the file moved, and locate is checked against the digest.
    let moved = dir.join("archive.wav");
    std::fs::rename(&source, &moved).unwrap();
    let lost = duet(&root, &["take", "list"]);
    assert!(
        stdout(&lost).contains("not found: looked at takes/phone/phone.wav"),
        "{}",
        stdout(&lost)
    );
    ok(
        &duet(&root, &["take", "locate", "phone", s(&moved)]),
        "take locate",
    );
    let list = duet(&root, &["take", "list"]);
    assert!(
        stdout(&list).contains(&format!("found at {}", moved.display())),
        "{}",
        stdout(&list)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_changed_file_at_a_named_path_is_refused_naming_the_digest() {
    let dir = scratch("take-changed");
    let root = project(&dir, "porch");
    let source = wav(&dir, "phone.wav", &clicks(110.0));
    ok(
        &duet(
            &root,
            &["take", "add", s(&source), "--named", "--name", "phone"],
        ),
        "take add --named",
    );
    let digest = read_project(&root).takes[0].take.digest.to_string();
    wav(&dir, "phone.wav", &clicks(90.0));

    let list = duet(&root, &["take", "list"]);
    ok(&list, "take list reports, and does not refuse");
    assert!(
        stdout(&list).contains(&format!("changed: {} holds sha256:", source.display())),
        "{}",
        stdout(&list)
    );
    let show = duet(&root, &["project", "show"]);
    assert!(stdout(&show).contains("changed:"), "{}", stdout(&show));

    let locate = duet(&root, &["take", "locate", "phone", s(&source)]);
    assert!(!locate.status.success(), "a changed file is refused");
    assert!(
        stderr(&locate).contains(&digest) && stderr(&locate).contains("is not the take phone"),
        "{}",
        stderr(&locate)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_tempo_given_to_a_take_is_remembered_and_stated_as_given_next_time() {
    let dir = scratch("take-given");
    let root = project(&dir, "porch");
    let source = wav(&dir, "beat.wav", &clicks(92.0));
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");
    let take = root.join("takes/beat/beat.wav");

    let first = duet(
        &root,
        &[
            "capture",
            "rhythm",
            s(&take),
            "--tempo",
            "92",
            "-o",
            s(&root.join("first")),
        ],
    );
    ok(&first, "capture with --tempo");
    let given = read_project(&root).takes[0].given.clone().expect("given");
    assert_eq!(given.tempo_bpm, Some(92.0));
    assert!(
        stderr(&first).contains("remembered tempo 92 bpm"),
        "{}",
        stderr(&first)
    );

    let next = duet(
        &root,
        &["capture", "rhythm", s(&take), "-o", s(&root.join("next"))],
    );
    ok(&next, "capture with no flag");
    assert!(
        stderr(&next).contains("tempo: 92 bpm, given for the take beat"),
        "{}",
        stderr(&next)
    );
    assert!(
        !stderr(&next).contains("tempo: estimated"),
        "{}",
        stderr(&next)
    );
    let analysis: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("next.capture.duet")).unwrap())
            .unwrap();
    assert_eq!(
        analysis["body"]["timing"]["tempo_bpm"]["given"], 92.0,
        "{analysis}"
    );

    // A flag still wins, and is remembered in its place.
    let flag = duet(
        &root,
        &[
            "capture",
            "rhythm",
            s(&take),
            "--tempo",
            "184",
            "-o",
            s(&root.join("fast")),
        ],
    );
    ok(&flag, "capture with another --tempo");
    assert!(
        !stderr(&flag).contains("given for the take"),
        "{}",
        stderr(&flag)
    );
    assert_eq!(
        read_project(&root).takes[0]
            .given
            .as_ref()
            .unwrap()
            .tempo_bpm,
        Some(184.0)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn take_forget_leaves_the_files_and_says_which_song_still_links_the_take() {
    let dir = scratch("take-forget");
    let root = project(&dir, "porch");
    let source = wav(&dir, "beat.wav", &clicks(100.0));
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");
    let take = root.join("takes/beat/beat.wav");
    let bytes = std::fs::read(&take).unwrap();
    ok(
        &duet(&root, &["capture", "rhythm", s(&take), "--tempo", "100"]),
        "capture",
    );
    ok(&duet(&root, &["song", "new", "groove"]), "song new");
    std::fs::copy(
        root.join("takes/beat/beat.rhythm.duet"),
        root.join("songs/groove/parts/kit.duet"),
    )
    .unwrap();

    let out = duet(&root, &["take", "forget", "beat"]);
    ok(&out, "take forget");
    assert!(read_project(&root).takes.is_empty(), "the entry is dropped");
    assert_eq!(
        std::fs::read(&take).unwrap(),
        bytes,
        "the take's file is left"
    );
    for kept in [
        "beat.rhythm.duet",
        "beat.rhythm.grid.duet",
        "beat.rhythm.capture.duet",
    ] {
        assert!(
            root.join("takes/beat").join(kept).is_file(),
            "{kept} is left"
        );
    }
    assert!(
        stdout(&out).contains("left in takes/beat/"),
        "{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("songs/groove/parts/kit.duet is still comped from the take beat"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn one_recording_under_another_name_is_refused() {
    let dir = scratch("take-duplicate");
    let root = project(&dir, "porch");
    let first = wav(&dir, "first.wav", &clicks(100.0));
    let again = dir.join("again.wav");
    std::fs::copy(&first, &again).unwrap();
    ok(&duet(&root, &["take", "add", s(&first)]), "take add");
    let before = std::fs::read(root.join(".duet/project.duet")).unwrap();
    let out = duet(&root, &["take", "add", s(&again), "--named"]);
    assert!(
        !out.status.success(),
        "a second name for one recording is refused"
    );
    let digest = read_project(&root).takes[0].take.digest.to_string();
    assert!(
        stderr(&out).contains("is the take first already") && stderr(&out).contains(&digest),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read(root.join(".duet/project.duet")).unwrap(),
        before
    );
    assert!(!root.join(".duet/takes.local.toml").exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn record_takes_a_take_name_only_in_a_project_and_never_a_name_it_has() {
    let dir = scratch("take-record");
    let outside = duet(&dir, &["record", "wayfaring", "--seconds", "1"]);
    assert!(!outside.status.success());
    assert!(
        stderr(&outside).contains("records the take wayfaring into a project"),
        "{}",
        stderr(&outside)
    );

    let root = project(&dir, "porch");
    let source = wav(&dir, "wayfaring.wav", &clicks(100.0));
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");
    let taken = duet(&root, &["record", "wayfaring", "--seconds", "1"]);
    assert!(!taken.status.success());
    assert!(
        stderr(&taken).contains("has a take named wayfaring already"),
        "{}",
        stderr(&taken)
    );
    assert!(!root.join("takes/wayfaring/wayfaring.flac").exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}
