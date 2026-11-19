//! Every action records the documents it writes (bus task 2). Inside a project (a directory whose
//! `.duet/` holds `project.duet`, or the one `DUET_PROJECT` names) each is appended to the
//! journal as a `doc.written`: its path from the root, its kind and version, its body's digest,
//! and whether it replaced a file. Outside a project no journal is written.

use std::path::{Path, PathBuf};

use duet_capture::{Analysis, PerformanceHead};
use duet_e2e::{duet_env, notes_doc, ok, root, s, scratch};
use duet_schemas::journal::{DocDigest, DocWritten, Event, Line};
use duet_schemas::{Document, Kind, PartBody, Project, Song, Stream, Tool, Version};

/// Runs `duet` in `cwd` as `tester`, with no config but the project's.
fn duet(cwd: &Path, args: &[&str]) -> std::process::Output {
    duet_env(
        cwd,
        &cwd.join("no-config"),
        args,
        b"",
        &[("DUET_ACTOR", "tester")],
    )
}

/// A project at `dir/name`: its `.duet/project.duet`, and nothing else.
fn project(dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join(".duet")).unwrap();
    let project = Project::new("Porch", "2026-11-18".parse().unwrap(), Version::new(0, 3));
    std::fs::write(
        root.join(".duet/project.duet"),
        Document::new(Tool::new("duet", "0.3.0"), project)
            .write()
            .unwrap(),
    )
    .unwrap();
    root
}

/// Every entry of the project's journal, in replay order.
fn entries(root: &Path) -> Vec<Line> {
    duet_bus::read(root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .entries()
        .map(|(_, line)| line.clone())
        .collect()
}

fn written(line: &Line) -> &DocWritten {
    match line.read() {
        Some(Event::DocWritten(doc)) => doc,
        other => panic!("not a doc.written: {other:?}"),
    }
}

/// The digest of a document's body, as its type writes it compactly.
fn body_digest<K: Kind>(path: &Path) -> DocDigest {
    let doc = Document::<K>::read(&std::fs::read_to_string(path).unwrap()).unwrap();
    DocDigest::of(&serde_json::to_vec(&doc.body).unwrap())
}

/// A stream's digest: its head compactly, then every later line.
fn stream_digest(path: &Path) -> DocDigest {
    let text = std::fs::read_to_string(path).unwrap();
    let stream = Stream::<PerformanceHead>::read(&text).unwrap();
    let mut bytes = serde_json::to_vec(&stream.head).unwrap();
    bytes.push(b'\n');
    bytes.extend_from_slice(text.split_once('\n').unwrap().1.as_bytes());
    DocDigest::of(&bytes)
}

/// Every file under `dir`, relative to it, sorted.
fn files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn capture_melody_in_a_project_appends_exactly_the_doc_written_events_for_its_files() {
    let dir = scratch("actions-capture");
    let root = project(&dir, "wayfaring");
    std::fs::create_dir_all(root.join("takes")).unwrap();
    std::fs::copy(
        root_of_corpus().join("wayfaring-stranger.flac"),
        root.join("takes/wayfaring.flac"),
    )
    .unwrap();
    // From a directory under the root: the project is found from the working directory up.
    let takes = root.join("takes");
    ok(
        &duet(&takes, &["capture", "melody", "wayfaring.flac"]),
        "capture melody in a project",
    );

    let lines = entries(&root);
    let got: Vec<(&str, &str, Version, bool)> = lines
        .iter()
        .map(|line| {
            let doc = written(line);
            (
                doc.path.as_str(),
                doc.kind.as_str(),
                doc.version,
                doc.replaced,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                "takes/wayfaring.performance.duet.jsonl",
                "performance",
                PerformanceHead::VERSION,
                false
            ),
            ("takes/wayfaring.duet", "part", PartBody::VERSION, false),
            (
                "takes/wayfaring.capture.duet",
                "capture-analysis",
                Analysis::VERSION,
                false
            ),
        ],
        "exactly the documents capture wrote: the score, takes/wayfaring.abc, is no document"
    );
    assert!(
        takes.join("wayfaring.abc").is_file(),
        "the score was written"
    );
    assert_eq!(
        written(&lines[0]).digest,
        stream_digest(&takes.join("wayfaring.performance.duet.jsonl"))
    );
    assert_eq!(
        written(&lines[1]).digest,
        body_digest::<PartBody>(&takes.join("wayfaring.duet"))
    );
    assert_eq!(
        written(&lines[2]).digest,
        body_digest::<Analysis>(&takes.join("wayfaring.capture.duet"))
    );
    // One act: every entry is by who ran it, through the CLI, caused by one command id.
    let cause = lines[0].cause.clone().expect("a cause");
    for line in &lines {
        assert_eq!(
            (line.by.who.as_str(), line.by.face.as_str()),
            ("tester", "cli")
        );
        assert_eq!(line.cause.as_ref(), Some(&cause));
        assert_eq!(line.event, "doc.written");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

fn root_of_corpus() -> PathBuf {
    root().join("crates/duet-capture/corpus")
}

#[test]
fn an_action_that_writes_over_a_file_records_replaced_true() {
    let dir = scratch("actions-replace");
    let root = project(&dir, "porch");
    let doc = notes_doc(&root, "tune.notes", "[track \"P\" ch1]\n1.1.0 C4 1 80\n");
    // notes_doc ran `duet apply -o`: a new document.
    let view = root.join("tune.view");
    ok(
        &duet(&root, &["view", s(&doc), "-o", s(&view)]),
        "view the document",
    );
    ok(
        &duet(&root, &["apply", s(&view), "--onto", s(&doc)]),
        "apply onto it",
    );
    ok(&duet(&root, &["song", "new", "porch"]), "song new");
    ok(
        &duet(&root, &["song", "status", "porch", "sketch"]),
        "song status",
    );
    let got: Vec<(String, bool)> = entries(&root)
        .iter()
        .map(|line| {
            let doc = written(line);
            (doc.path.as_str().to_owned(), doc.replaced)
        })
        .collect();
    assert_eq!(
        got,
        [
            ("tune.duet".to_owned(), false),
            ("tune.duet".to_owned(), true),
            ("songs/porch/song.duet".to_owned(), false),
            ("songs/porch/song.duet".to_owned(), true),
        ],
        "a view's text is no document, and is not recorded"
    );
    let last = entries(&root).pop().unwrap();
    assert_eq!(
        written(&last).digest,
        body_digest::<Song>(&root.join("songs/porch/song.duet"))
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn outside_a_project_every_action_writes_what_it_always_has_and_no_journal() {
    let dir = scratch("actions-outside");
    // A .duet directory with config in it is no project: it holds no project.duet.
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    let doc = notes_doc(&dir, "tune.notes", "[track \"P\" ch1]\n1.1.0 C4 1 80\n");
    ok(
        &duet(&dir, &["convert", s(&doc), s(&dir.join("tune.mid"))]),
        "convert",
    );
    ok(&duet(&dir, &["song", "new", "porch"]), "song new");
    ok(
        &duet(
            &dir,
            &[
                "part",
                "quantize",
                s(&doc),
                s(&dir.join("q.duet")),
                "--grid",
                "1/8",
            ],
        ),
        "quantize",
    );
    assert_eq!(
        files(&dir),
        [
            "q.duet",
            "songs/porch/song.duet",
            "tune.duet",
            "tune.mid",
            "tune.notes"
        ],
        "no journal, no .duet/run"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn readers_write_nothing_in_a_project() {
    let dir = scratch("actions-readers");
    let root = project(&dir, "porch");
    let doc = notes_doc(&root, "tune.notes", "[track \"P\" ch1]\n1.1.0 C4 1 80\n");
    ok(&duet(&root, &["song", "new", "porch"]), "song new");
    let before = files(&root);
    for args in [
        vec!["part", "show", s(&doc)],
        vec!["view", s(&doc)],
        vec!["song", "show", "porch"],
        vec!["drums", "lanes"],
        vec!["config", "show"],
        vec!["config", "path"],
        vec!["project", "path"],
        vec!["project", "show"],
        vec!["take", "list"],
        vec!["journal", "show"],
        vec!["journal", "check"],
    ] {
        ok(&duet(&root, &args), &format!("{args:?}"));
        assert_eq!(files(&root), before, "{args:?} wrote a file");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_project_duet_project_names_is_the_one_journalled_and_a_wrong_one_is_refused() {
    let dir = scratch("actions-named");
    let root = project(&dir, "porch");
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let named = |args: &[&str], project: &str| {
        duet_env(
            &elsewhere,
            &elsewhere.join("no-config"),
            args,
            b"",
            &[("DUET_PROJECT", project), ("DUET_ACTOR", "tester")],
        )
    };
    ok(
        &named(&["song", "new", "porch", "--project", s(&root)], s(&root)),
        "song new in the named project",
    );
    let paths: Vec<String> = entries(&root)
        .iter()
        .map(|line| written(line).path.as_str().to_owned())
        .collect();
    assert_eq!(paths, ["songs/porch/song.duet"]);
    let out = named(&["drums", "lanes"], s(&elsewhere));
    assert!(!out.status.success(), "a directory that is no project");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("DUET_PROJECT names") && said.contains("no .duet/project.duet"),
        "{said}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_journal_rolls_at_the_configured_segment_size() {
    let dir = scratch("actions-roll");
    let root = project(&dir, "porch");
    std::fs::write(
        root.join(".duet/duet.toml"),
        "[journal]\nsegment-size = 1\n",
    )
    .unwrap();
    let shown = duet(&root, &["config", "show"]);
    ok(&shown, "config show");
    assert!(
        String::from_utf8_lossy(&shown.stdout).contains("[journal]\nsegment-size = 1  #"),
        "{}",
        String::from_utf8_lossy(&shown.stdout)
    );
    ok(&duet(&root, &["song", "new", "one"]), "song new one");
    ok(&duet(&root, &["song", "new", "two"]), "song new two");
    let journal = duet_bus::read(&root).unwrap();
    assert_eq!(journal.segments.len(), 2, "a segment of one byte rolls");
    assert_eq!(entries(&root).len(), 2);

    std::fs::write(
        root.join(".duet/duet.toml"),
        "[journal]\nsegment-size = \"none\"\n",
    )
    .unwrap();
    let out = duet(&root, &["drums", "lanes"]);
    assert!(!out.status.success(), "a size that is no size");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("segment-size is a size"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A project whose `journal.duet.jsonl` is a symbolic link out of it, as a clone could hold: the
/// action's own document is written, but its journal entry is refused by name, and the file the
/// link points at is left exactly as it was.
#[cfg(unix)]
#[test]
fn a_journal_that_is_a_symbolic_link_out_of_the_project_is_refused_and_its_target_untouched() {
    let dir = scratch("actions-link");
    let root = project(&dir, "porch");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("victim.txt");
    std::fs::write(&victim, "keep this\na torn last line").unwrap();
    std::os::unix::fs::symlink(&victim, root.join("journal.duet.jsonl")).unwrap();
    let out = duet(&root, &["song", "new", "porch"]);
    assert!(!out.status.success(), "the journal followed the link");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("journal.duet.jsonl is a symbolic link")
            && said.contains("nothing was written"),
        "{said}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "keep this\na torn last line"
    );
    assert!(
        root.join("songs/porch/song.duet").is_file(),
        "the action ran"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
