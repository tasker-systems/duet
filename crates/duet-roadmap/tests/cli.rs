//! The `duet-roadmap` binary against a fake `temper`: a shell script that answers from the fixture
//! and logs every command it is given. It runs the real process path: spawning `temper`, reading
//! its exit code and its JSON, and piping the snapshot's body to `resource create`.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
printf '%s\n' "$*" >> "$dir/calls.log"
case "$1 $2 $3 $4" in
  "resource list --type goal") cat "$dir/goals.json" ;;
  "resource list --type task") cat "$dir/tasks.json" ;;
  "resource show "*) cat "$dir/show-$3.json" ;;
  "resource create "*) cat > "$dir/created.md"; echo '{"id": "50000000-0000-7000-8000-000000000001"}' ;;
  *) echo "fake temper: unexpected command: $*" >&2; exit 2 ;;
esac
"#;

/// A directory holding the fake `temper` and the fixture split into the files it serves.
fn fake_temper(name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("duet-roadmap-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear the old run");
    }
    std::fs::create_dir_all(&dir).expect("make the fake's directory");
    let mut records = common::fixture();
    edit(&mut records);
    std::fs::write(dir.join("goals.json"), records["goals"].to_string()).expect("goals");
    std::fs::write(dir.join("tasks.json"), records["tasks"].to_string()).expect("tasks");
    for (id, show) in records["shows"].as_object().expect("shows") {
        std::fs::write(dir.join(format!("show-{id}.json")), show.to_string()).expect("show");
    }
    let script = dir.join("temper");
    std::fs::write(&script, FAKE).expect("the fake temper");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("executable");
    dir
}

fn run(temper: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_duet-roadmap"))
        .arg("snapshot")
        .arg("--temper")
        .arg(temper)
        .args(args)
        .output()
        .expect("duet-roadmap runs")
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_dry_run_prints_the_snapshot_and_writes_nothing() {
    let dir = fake_temper("dry-run", |_| {});
    let out = run(&dir.join("temper"), &["--dry-run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(stdout.starts_with("# duet roadmap snapshot, "), "{stdout}");
    assert!(
        stdout.contains("## What this snapshot leaves out"),
        "{stdout}"
    );
    let calls = calls(&dir);
    assert_eq!(calls.len(), 2 + 19, "{calls:?}");
    assert!(
        calls.iter().all(|c| !c.starts_with("resource create")),
        "{calls:?}"
    );
}

#[test]
fn a_write_creates_one_research_resource_and_nothing_else() {
    let dir = fake_temper("write", |_| {});
    let out = run(&dir.join("temper"), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("50000000-0000-7000-8000-000000000001"),
        "{stdout}"
    );

    let calls = calls(&dir);
    let writes: Vec<&String> = calls
        .iter()
        .filter(|c| !(c.starts_with("resource list ") || c.starts_with("resource show ")))
        .collect();
    assert_eq!(writes.len(), 1, "{calls:?}");
    assert!(
        writes[0].starts_with("resource create --type research --context +making-music/duet-dev ")
    );
    assert!(
        writes[0].contains(r#""tags":["roadmap-snapshot"]"#),
        "{}",
        writes[0]
    );

    let body = std::fs::read_to_string(dir.join("created.md")).expect("the body was piped");
    assert!(body.starts_with("# duet roadmap snapshot, "), "{body}");
}

#[test]
fn a_truncated_listing_exits_non_zero_and_writes_nothing() {
    let dir = fake_temper("truncated", |r| {
        r["tasks"]["truncated"] = serde_json::json!(true)
    });
    let out = run(&dir.join("temper"), &[]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("truncated") && stderr.contains("nothing was written"),
        "{stderr}"
    );
    assert!(
        calls(&dir)
            .iter()
            .all(|c| !c.starts_with("resource create"))
    );
    assert!(!dir.join("created.md").exists());
}

#[test]
fn a_failing_read_exits_non_zero_and_writes_nothing() {
    // One task's edges cannot be read: the fake's `cat` fails, so `temper` exits non-zero.
    let missing = "20000000-0000-7000-8000-000000000007";
    let dir = fake_temper("failing-read", |_| {});
    std::fs::remove_file(dir.join(format!("show-{missing}.json"))).expect("remove one answer");
    let out = run(&dir.join("temper"), &[]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(missing) && stderr.contains("nothing was written"),
        "{stderr}"
    );
    assert!(
        calls(&dir)
            .iter()
            .all(|c| !c.starts_with("resource create"))
    );
}

#[test]
fn an_unreachable_temper_is_never_an_empty_roadmap() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("duet-roadmap-no-temper");
    let out = run(&dir.join("temper"), &["--dry-run"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "no snapshot is printed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not run") && stderr.contains("nothing was written"),
        "{stderr}"
    );
}
