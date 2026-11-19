//! Commands that default into a project (the project design's §5, build task 4): inside one,
//! `capture`, `compare` and `review` take a take by its name, read it through `take::find`, and
//! write under `takes/<take>/`; a capture of a path that is not a project take is refused with
//! the `duet take add` line; and the commands that make music without a recording use the
//! project's `defaults`, each saying so. Outside a project, and in a `.duet/` with no project
//! document, nothing changes.
//!
//! Every take here is audio the test writes itself: nothing comes from `fixtures/`.

#[path = "capture/audio.rs"]
#[expect(
    dead_code,
    reason = "the capture tests' shared generators: these tests use the tones and clicks alone"
)]
mod audio;

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, grid_doc, ok, s, scratch};

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A fresh project at `dir/name`.
fn project(dir: &Path, name: &str) -> PathBuf {
    ok(&duet(dir, &["init", name]), "init");
    dir.join(name)
}

/// Sets the project's `defaults`, as a hand would: its body edited as JSON.
fn set_defaults(root: &Path, defaults: serde_json::Value) {
    let path = root.join(".duet/project.duet");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["body"]["defaults"] = defaults;
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

/// A short melody written to `dir/name`: a take a melody capture reads.
fn melody(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    audio::write_wav(
        &path,
        &audio::notes_off(0.5, &[(60, 0.6), (64, 0.6), (67, 0.9), (64, 0.6)], 0.0),
    );
    path
}

/// Every file name in `dir`, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn in_a_project_capture_melody_by_name_writes_under_the_takes_directory() {
    let dir = scratch("defaults-capture");
    let root = project(&dir, "porch");
    let source = melody(&dir, "wayfaring.wav");
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");

    let out = duet(&root, &["capture", "melody", "wayfaring", "--tempo", "120"]);
    ok(&out, "capture melody wayfaring");
    let takes = root.join("takes/wayfaring");
    assert_eq!(
        files(&takes),
        [
            "wayfaring.hmm.abc",
            "wayfaring.hmm.capture.duet",
            "wayfaring.hmm.duet",
            "wayfaring.hmm.performance.duet.jsonl",
            "wayfaring.wav",
        ],
        "the reading's stem is <take>.<reader>, in the take's directory"
    );
    assert!(
        stderr(&out).contains("take wayfaring: found at takes/wayfaring/wayfaring.wav"),
        "{}",
        stderr(&out)
    );
    assert!(
        !dir.join("wayfaring.duet").exists(),
        "nothing beside the source"
    );

    // Another reader's reading sits beside it, from a directory below the root too.
    let below = root.join("songs");
    std::fs::create_dir_all(&below).unwrap();
    ok(
        &duet(
            &below,
            &[
                "capture",
                "melody",
                "wayfaring",
                "--tempo",
                "120",
                "--reader",
                "rules",
            ],
        ),
        "capture melody wayfaring --reader rules",
    );
    assert!(takes.join("wayfaring.rules.capture.duet").is_file());

    // The take given by its path in the project defaults the same way.
    let copied = takes.join("wayfaring.wav");
    let out = duet(&dir, &["capture", "rhythm", s(&copied), "--tempo", "120"]);
    ok(
        &out,
        "capture rhythm by the copied path, from outside the root",
    );
    // Outside the project's directory no project is found, so this is today's capture.
    assert!(takes.join("wayfaring.duet").is_file(), "{}", stderr(&out));
    let out = duet(&root, &["capture", "rhythm", s(&copied), "--tempo", "120"]);
    ok(&out, "capture rhythm by the copied path, in the project");
    assert!(
        takes.join("wayfaring.rhythm.grid.duet").is_file(),
        "a rhythm reading's label is rhythm: {}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_named_take_is_captured_by_name_into_a_takes_directory_made_for_it() {
    let dir = scratch("defaults-named");
    let root = project(&dir, "porch");
    let source = melody(&dir, "phone.wav");
    ok(
        &duet(&root, &["take", "add", s(&source), "--named"]),
        "take add --named",
    );
    assert!(!root.join("takes").exists(), "nothing is copied");
    let out = duet(&root, &["capture", "melody", "phone", "--tempo", "120"]);
    ok(&out, "capture melody phone");
    assert!(
        root.join("takes/phone/phone.hmm.capture.duet").is_file(),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("named in .duet/takes.local.toml"),
        "{}",
        stderr(&out)
    );

    // A changed file is refused by take::find, naming the take, before anything is written.
    audio::write_wav(&source, &audio::notes_off(0.5, &[(62, 1.0)], 0.0));
    let changed = duet(
        &root,
        &[
            "capture", "melody", "phone", "--tempo", "120", "--reader", "rules",
        ],
    );
    assert!(!changed.status.success(), "a changed take is refused");
    assert!(
        stderr(&changed).contains("take phone (sha256:")
            && stderr(&changed).contains("has changed")
            && stderr(&changed).contains("duet take locate phone"),
        "{}",
        stderr(&changed)
    );
    assert!(!root.join("takes/phone/phone.rules.duet").exists());

    // A name the project has no take of is refused, naming the takes it has.
    let missing = duet(&root, &["capture", "melody", "wayfaring"]);
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("the project has no take wayfaring; its takes are phone"),
        "{}",
        stderr(&missing)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_capture_of_a_path_not_in_the_project_is_refused_with_the_take_add_line() {
    let dir = scratch("defaults-refused");
    let root = project(&dir, "porch");
    let stray = melody(&root, "Stray Idea.wav");

    let out = duet(&root, &["capture", "melody", s(&stray), "--tempo", "120"]);
    assert!(!out.status.success(), "refused: {}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!("`duet take add {}`", stray.display()))
            && stderr(&out).contains("`duet capture melody stray-idea`"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        files(&root),
        [".duet", "Stray Idea.wav"],
        "nothing is written"
    );

    // -o under takes/ is refused too: the take is not the project's.
    let under = root.join("takes/stray/stray");
    std::fs::create_dir_all(under.parent().unwrap()).unwrap();
    let out = duet(
        &root,
        &[
            "capture",
            "melody",
            s(&stray),
            "--tempo",
            "120",
            "-o",
            s(&under),
        ],
    );
    assert!(!out.status.success(), "refused under takes/");
    assert!(stderr(&out).contains("duet take add"), "{}", stderr(&out));
    assert!(files(under.parent().unwrap()).is_empty());

    // -o outside takes/ wins, and duet says the take is not in the project.
    let elsewhere = root.join("sketches/stray");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    let out = duet(
        &root,
        &[
            "capture",
            "melody",
            s(&stray),
            "--tempo",
            "120",
            "-o",
            s(&elsewhere),
        ],
    );
    ok(&out, "capture with -o outside takes/");
    assert!(root.join("sketches/stray.capture.duet").is_file());
    assert!(
        stderr(&out).contains("is not a take in this project"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn compare_and_review_take_a_take_by_name_and_write_under_its_directory() {
    let dir = scratch("defaults-compare");
    let root = project(&dir, "porch");
    let source = melody(&dir, "wayfaring.wav");
    ok(&duet(&root, &["take", "add", s(&source)]), "take add");
    ok(
        &duet(&root, &["capture", "melody", "wayfaring", "--tempo", "120"]),
        "capture melody wayfaring",
    );
    let takes = root.join("takes/wayfaring");

    let out = duet(&root, &["capture", "compare", "wayfaring"]);
    ok(&out, "capture compare wayfaring");
    for file in ["wayfaring.hmm.compare.svg", "wayfaring.hmm.compare.wav"] {
        assert!(takes.join(file).is_file(), "{file}: {}", stderr(&out));
    }

    let moments = root.join("review/wayfaring.moments");
    std::fs::create_dir_all(moments.parent().unwrap()).unwrap();
    std::fs::write(&moments, "0.4 1.4 the first two notes\n").unwrap();
    let out = duet(
        &root,
        &["capture", "review", "wayfaring", "--moments", s(&moments)],
    );
    ok(&out, "capture review wayfaring");
    assert!(
        takes.join("review/index.html").is_file(),
        "the sheet goes to takes/<take>/review/: {}",
        stderr(&out)
    );

    // With two readings, compare asks which.
    ok(
        &duet(
            &root,
            &[
                "capture",
                "melody",
                "wayfaring",
                "--tempo",
                "120",
                "--reader",
                "rules",
            ],
        ),
        "a second reading",
    );
    let out = duet(&root, &["capture", "compare", "wayfaring", "--from", "0.5"]);
    assert!(!out.status.success(), "two readings: which is asked");
    assert!(
        stderr(&out).contains("hmm")
            && stderr(&out).contains("rules")
            && stderr(&out).contains("-o"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A `.duet/` with no project document is a config directory, as before: a capture of a path
/// is written beside it, and is never refused as no project take.
#[test]
fn a_duet_directory_without_a_project_document_changes_nothing() {
    let dir = scratch("defaults-config-only");
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    std::fs::write(
        dir.join(".duet/duet.toml"),
        "[part]\nend = { round-up = 2 }\n",
    )
    .unwrap();
    let take = melody(&dir, "idea.wav");
    let out = duet(&dir, &["capture", "melody", s(&take), "--tempo", "120"]);
    ok(&out, "capture melody beside a config-only .duet/");
    for file in ["idea.duet", "idea.abc", "idea.capture.duet"] {
        assert!(dir.join(file).is_file(), "{file}: {}", stderr(&out));
    }
    assert!(!dir.join("takes").exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn song_new_says_where_each_value_of_its_composition_came_from() {
    let dir = scratch("defaults-song");
    let root = project(&dir, "porch");
    set_defaults(
        &root,
        serde_json::json!({ "tempo_bpm": 92.0, "meter": "3/4" }),
    );
    let out = duet(&root, &["song", "new", "lift", "--key", "D dorian"]);
    ok(&out, "song new");
    for said in [
        "tempo: 92 bpm, from the project's defaults in .duet/project.duet",
        "meter: 3/4, from the project's defaults in .duet/project.duet",
        "key: D dorian, given",
    ] {
        assert!(stderr(&out).contains(said), "{said:?} in {}", stderr(&out));
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn drums_read_takes_its_grid_from_the_project_and_says_so() {
    let dir = scratch("defaults-grid");
    let root = project(&dir, "porch");
    set_defaults(&root, serde_json::json!({ "grid": "1/8" }));
    let text = "grid 1/8\nvel g 30 x 80 X 110\n\n[bars 1]\n\
        snare | . . x . . . x . |\n\
        kick  | x . . . x . . . |\n";
    let grid = grid_doc(&root, "beat.drums", text);
    let part = root.join("beat.mid");
    ok(
        &duet(&root, &["drums", "render", s(&grid), s(&part)]),
        "drums render",
    );
    let back = root.join("back.duet");
    let out = duet(&root, &["drums", "read", s(&part), s(&back)]);
    ok(&out, "drums read with no --grid, in a project");
    assert!(
        stderr(&out).contains("grid: 1/8, from the project's defaults in .duet/project.duet"),
        "{}",
        stderr(&out)
    );
    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&back).unwrap()).unwrap();
    assert_eq!(read["body"]["grid"], "1/8", "{read}");

    // A flag wins, and is not stated as the project's.
    let given = root.join("given.duet");
    let out = duet(
        &root,
        &["drums", "read", s(&part), s(&given), "--grid", "1/16"],
    );
    ok(&out, "drums read --grid 1/16");
    assert!(
        !stderr(&out).contains("from the project"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
