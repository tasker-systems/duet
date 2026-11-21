//! `samples/unce-rework`: a project holding the owner's freestyle beatbox take and the song it
//! was reworked into, one snapshot per step (v0 quantized, v1 a loop, v2 the groove). These tests
//! keep the sample readable by the `duet` in the tree: its take matches the digest the project
//! states, the song and each snapshot read, and each snapshot plays with no disagreement between
//! its part and its composition. What any step sounds like is the owner's, by ear, and is
//! recorded in the sample's README, not here.

use std::path::{Path, PathBuf};

use duet_e2e::{duet_env, ok, root, scratch};

const STEPS: [&str; 3] = ["v0-quantized", "v1-loop", "v2-groove"];

fn duet(cwd: &Path, args: &[&str]) -> std::process::Output {
    duet_env(cwd, &cwd.join("no-config"), args, b"", &[])
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the sample, so no test writes into the repository.
fn sample(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let project = dir.join("unce-rework");
    copy_tree(&root().join("samples/unce-rework"), &project);
    (dir, project)
}

fn snapshot(project: &Path, step: &str) -> String {
    let snapshots = project.join("songs/unce-rework/snapshots");
    let found: Vec<String> = std::fs::read_dir(&snapshots)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(&format!("-{step}")))
        .collect();
    assert_eq!(found.len(), 1, "one snapshot of {step}: {found:?}");
    found.into_iter().next().unwrap()
}

/// The take is found at its stated digest, and the song shows each step as a snapshot.
#[test]
fn the_sample_reads_its_take_and_each_step() {
    let (dir, project) = sample("unce-sample");
    let out = duet(&project, &["project", "show"]);
    ok(&out, "project show");
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(
        shown.contains("sha256:d17c791dbeb8ffbd0a7b5aae799d79cceca7f9ad890dc333192103e35b92ff71")
            && shown.contains("found at takes/unce/unce.flac"),
        "{shown}"
    );
    let out = duet(&project, &["song", "show", "unce-rework"]);
    ok(&out, "song show");
    let shown = String::from_utf8_lossy(&out.stdout);
    for step in STEPS {
        assert!(shown.contains(&snapshot(&project, step)), "{step}: {shown}");
    }
    assert!(shown.contains("v2-groove at 1.1.0"), "{shown}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Each step plays with its composition and its part in agreement: nothing is stated before the
/// player is looked for (here a port no machine has, so nothing sounds). The port fails either
/// by name or, where the machine has no MIDI at all (a CI runner, whose ALSA library also
/// writes its own lines), as MIDI that could not be opened.
#[test]
fn each_step_plays_with_nothing_to_state() {
    let (dir, project) = sample("unce-sample-play");
    for step in STEPS {
        let song = format!(
            "songs/unce-rework/snapshots/{}/song.duet",
            snapshot(&project, step)
        );
        let out = duet(&project, &["play", &song, "--port", "no such port 7f3a"]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{step}");
        let port = err
            .find("duet: no MIDI port is named")
            .or_else(|| err.find("duet: opening MIDI output"))
            .unwrap_or_else(|| panic!("{step}: the port is never looked for: {err}"));
        let said: Vec<&str> = err[..port]
            .lines()
            .filter(|line| !line.starts_with("ALSA lib"))
            .collect();
        assert!(said.is_empty(), "{step}: stated before playing: {said:?}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Restoring the first step puts its part back in place; the digests it checks are the ones the
/// snapshot states.
#[test]
fn the_first_step_restores() {
    let (dir, project) = sample("unce-sample-restore");
    let v0 = snapshot(&project, "v0-quantized");
    ok(
        &duet(&project, &["song", "restore", "unce-rework", &v0]),
        "restore",
    );
    let out = duet(&project, &["song", "show", "unce-rework"]);
    ok(&out, "song show");
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("v0-quantized at 1.1.0"), "{shown}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
