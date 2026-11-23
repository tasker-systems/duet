//! `duet song snapshot` and `duet song restore` (songs task 4, the songs spec's §3): a snapshot
//! copies the song's `song.duet` and every part the composition places, byte for byte, and
//! states each copy's digest in its `snapshot.duet`; taking one leaves the song's status as it
//! was; `duet song show` reports a file in a snapshot that differs from its stated digest,
//! naming it; and a restore takes a snapshot of the working song before it copies anything back.

use std::path::{Path, PathBuf};

use duet_e2e::{duet_env, notes_doc, ok, scratch};
use duet_schemas::journal::{DocDigest, Event, Line};
use duet_schemas::snapshot::FileDigest;
use duet_schemas::song::Status;
use duet_schemas::{Document, Project, Snapshot, Song, Tool, Version};
use serde_json::{Value, json};

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

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A project at `dir/porch`, holding the song `porch` with three parts: `melody` and `bass`
/// placed, and `spare` kept for later and placed nowhere.
fn porch(dir: &Path) -> PathBuf {
    let root = dir.join("porch");
    std::fs::create_dir_all(root.join(".duet")).unwrap();
    let project = Project::new("Porch", "2026-11-18".parse().unwrap(), Version::new(0, 3));
    std::fs::write(
        root.join(".duet/project.duet"),
        Document::new(Tool::new("duet", "0.3.0"), project)
            .write()
            .unwrap(),
    )
    .unwrap();
    ok(&duet(&root, &["song", "new", "porch"]), "song new");
    for (name, line) in [
        ("melody", "1.1.0 G4 1 90"),
        ("bass", "1.1.0 G2 3 80"),
        ("spare", "1.1.0 D3 1 70"),
    ] {
        let made = notes_doc(&root, name, &format!("[track \"T\" ch1]\n{line}\n"));
        std::fs::rename(made, part_file(&root, name)).unwrap();
    }
    edit_song(&root, |body| {
        body["composition"]["placements"] = json!([
            { "part": "melody", "at": "1.1.0" },
            { "part": "bass", "at": "9.1.0", "repeat": 2 },
            { "part": "melody", "at": "17.1.0" },
        ]);
    });
    root
}

fn song_file(root: &Path) -> PathBuf {
    root.join("songs/porch/song.duet")
}

fn part_file(root: &Path, part: &str) -> PathBuf {
    root.join("songs/porch/parts").join(format!("{part}.duet"))
}

fn read_song(root: &Path) -> Song {
    Document::<Song>::read(&std::fs::read_to_string(song_file(root)).unwrap())
        .unwrap()
        .body
}

/// Edits the song's document as a hand would: its body as JSON.
fn edit_song(root: &Path, edit: impl FnOnce(&mut Value)) {
    let path = song_file(root);
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut doc["body"]);
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

/// The song's snapshot folders, by name, in order.
fn snapshots(root: &Path) -> Vec<String> {
    let dir = root.join("songs/porch/snapshots");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The one snapshot folder whose name ends `-<name>`.
fn snapshot_dir(root: &Path, name: &str) -> PathBuf {
    let found: Vec<String> = snapshots(root)
        .into_iter()
        .filter(|f| f.ends_with(&format!("-{name}")))
        .collect();
    assert_eq!(found.len(), 1, "one snapshot named {name}: {found:?}");
    root.join("songs/porch/snapshots").join(&found[0])
}

fn read_snapshot(dir: &Path) -> Snapshot {
    Document::<Snapshot>::read(&std::fs::read_to_string(dir.join("snapshot.duet")).unwrap())
        .unwrap()
        .body
}

/// Every entry of the project's journal, in replay order.
fn entries(root: &Path) -> Vec<Line> {
    duet_bus::read(root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .entries()
        .map(|(_, line)| line.clone())
        .collect()
}

/// The bytes of every file under `dir`, by its path from `dir`.
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_snapshot_copies_the_song_and_every_placed_part_byte_for_byte_and_states_their_digests() {
    let dir = scratch("snapshot-copies");
    let root = porch(&dir);
    let out = duet(
        &root,
        &[
            "song",
            "snapshot",
            "porch",
            "first-verse",
            "--said",
            "the verse \"works\"",
        ],
    );
    ok(&out, "song snapshot");
    let snap = snapshot_dir(&root, "first-verse");

    // Every copy is byte for byte its source, and only what the composition places is copied.
    let held = tree(&snap);
    let paths: Vec<&str> = held.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "parts/bass.duet",
            "parts/melody.duet",
            "snapshot.duet",
            "song.duet"
        ],
        "the spare part is placed nowhere, so it is not copied"
    );
    for (path, bytes) in &held {
        if path != "snapshot.duet" {
            let source = std::fs::read(root.join("songs/porch").join(path)).unwrap();
            assert_eq!(bytes, &source, "{path} is not byte for byte its source");
        }
    }

    // snapshot.duet states the name, the day, the owner's words and each copy's digest.
    let body = read_snapshot(&snap);
    assert_eq!(body.name.as_str(), "first-verse");
    assert_eq!(snap.file_name().unwrap().to_string_lossy(), body.folder());
    assert_eq!(body.said.as_deref(), Some("the verse \"works\""));
    let stated: Vec<(String, FileDigest)> = body
        .files
        .iter()
        .map(|f| (f.path.to_string(), f.digest))
        .collect();
    assert_eq!(
        stated,
        [
            (
                "song.duet",
                FileDigest::of(&std::fs::read(snap.join("song.duet")).unwrap())
            ),
            (
                "parts/melody.duet",
                FileDigest::of(&std::fs::read(snap.join("parts/melody.duet")).unwrap())
            ),
            (
                "parts/bass.duet",
                FileDigest::of(&std::fs::read(snap.join("parts/bass.duet")).unwrap())
            ),
        ]
        .map(|(p, d)| (p.to_owned(), d)),
        "song.duet first, then the placed parts in the order first placed"
    );
    assert!(body.takes.is_empty(), "song 1.0 links no takes yet");

    // The journal holds a doc.written for each file, then a snapshot.taken naming the song by
    // its id, with the digest of snapshot.duet's body.
    let song = read_song(&root);
    let events: Vec<Event> = entries(&root)
        .iter()
        .filter_map(Line::read)
        .cloned()
        .collect();
    let written: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::DocWritten(w) if w.path.as_str().contains("/snapshots/") => {
                Some(w.path.to_string())
            }
            _ => None,
        })
        .collect();
    let folder = format!("songs/porch/snapshots/{}", body.folder());
    assert_eq!(
        written,
        [
            "song.duet",
            "parts/melody.duet",
            "parts/bass.duet",
            "snapshot.duet"
        ]
        .map(|p| format!("{folder}/{p}"))
    );
    let Some(Event::SnapshotTaken(taken)) = events.last() else {
        panic!("the last entry is no snapshot.taken: {events:?}");
    };
    assert_eq!(taken.song, song.id);
    assert_eq!(taken.name.as_str(), "porch");
    assert_eq!(taken.snapshot, body.folder());
    assert_eq!(
        taken.digest,
        DocDigest::of(serde_json::to_string(&body).unwrap().as_bytes())
    );

    // A snapshot is never written again: taking one of the same name the same day is refused,
    // and leaves the first as it was.
    let again = duet(&root, &["song", "snapshot", "porch", "first-verse"]);
    assert!(!again.status.success(), "a snapshot was written over");
    assert!(stderr(&again).contains("exists"), "{}", stderr(&again));
    assert_eq!(tree(&snap), held);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_snapshot_names_the_takes_the_song_links_by_name_and_digest() {
    let dir = scratch("snapshot-takes");
    let root = porch(&dir);
    let one = format!("sha256:{}", "1".repeat(64));
    let two = format!("sha256:{}", "2".repeat(64));
    edit_song(&root, |body| {
        body["takes"] = json!([
            { "take": "verse", "digest": two },
            { "take": "chorus", "digest": one },
        ]);
    });
    ok(
        &duet(&root, &["song", "snapshot", "porch", "linked"]),
        "snapshot",
    );

    let snapshot = read_snapshot(&snapshot_dir(&root, "linked"));
    let taken: Vec<(String, String)> = snapshot
        .takes
        .iter()
        .map(|t| (t.take.clone(), t.digest.to_string()))
        .collect();
    assert_eq!(
        taken,
        vec![("chorus".to_owned(), one), ("verse".to_owned(), two)],
        "the song's links as (take, digest), sorted by take"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn taking_a_snapshot_leaves_the_songs_status_and_document_unchanged() {
    let dir = scratch("snapshot-status");
    let root = porch(&dir);
    for (state, name) in [("sketch", "a-sketch"), ("developing", "developing")] {
        ok(&duet(&root, &["song", "status", "porch", state]), state);
        let before = std::fs::read(song_file(&root)).unwrap();
        let moves = entries(&root)
            .iter()
            .filter(|l| l.event == "song.status-set")
            .count();
        ok(&duet(&root, &["song", "snapshot", "porch", name]), name);
        assert_eq!(read_song(&root).status, state.parse::<Status>().unwrap());
        assert_eq!(std::fs::read(song_file(&root)).unwrap(), before);
        assert_eq!(
            entries(&root)
                .iter()
                .filter(|l| l.event == "song.status-set")
                .count(),
            moves,
            "a snapshot journalled a status move"
        );
    }
    // The journal and the document still agree.
    let out = duet(&root, &["journal", "check"]);
    ok(&out, "journal check");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn song_show_reports_a_changed_file_in_a_snapshot_naming_it() {
    let dir = scratch("snapshot-changed");
    let root = porch(&dir);
    ok(
        &duet(&root, &["song", "snapshot", "porch", "first"]),
        "snapshot",
    );
    let snap = snapshot_dir(&root, "first");
    let folder = snap.file_name().unwrap().to_string_lossy().into_owned();

    let out = duet(&root, &["song", "show", "porch"]);
    ok(&out, "song show");
    assert!(
        stdout(&out).contains(&format!("{folder}: first, 3 files, as taken")),
        "{}",
        stdout(&out)
    );

    // One space added to a copied part.
    let bass = snap.join("parts/bass.duet");
    let mut bytes = std::fs::read(&bass).unwrap();
    bytes.push(b' ');
    std::fs::write(&bass, bytes).unwrap();
    let out = duet(&root, &["song", "show", "porch"]);
    ok(&out, "song show of a changed snapshot");
    let shown = stdout(&out);
    assert!(
        shown.contains(&format!(
            "{folder}: parts/bass.duet differs from its stated digest"
        )),
        "{shown}"
    );
    assert!(!shown.contains("as taken"), "{shown}");

    // A restore from it is refused, naming the file, and changes nothing.
    let before = tree(&root.join("songs/porch"));
    let out = duet(&root, &["song", "restore", "porch", &folder]);
    assert!(!out.status.success(), "restored from a changed snapshot");
    assert!(
        stderr(&out).contains("parts/bass.duet differs from its stated digest"),
        "{}",
        stderr(&out)
    );
    assert_eq!(tree(&root.join("songs/porch")), before);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_restore_takes_a_snapshot_of_the_working_song_first_then_copies_the_snapshot_back() {
    let dir = scratch("snapshot-restore");
    let root = porch(&dir);
    ok(
        &duet(&root, &["song", "snapshot", "porch", "first"]),
        "snapshot",
    );
    let first = snapshot_dir(&root, "first");
    let folder = first.file_name().unwrap().to_string_lossy().into_owned();

    // The working song moves on: a part rewritten, a section added, and the status moved.
    let notes = notes_doc(&root, "bass-new", "[track \"T\" ch1]\n1.1.0 A2 3 80\n");
    std::fs::rename(notes, part_file(&root, "bass")).unwrap();
    edit_song(&root, |body| {
        body["composition"]["sections"] = json!([{ "name": "verse", "at": "1.1.0" }]);
    });
    ok(
        &duet(&root, &["song", "status", "porch", "sketch"]),
        "status",
    );
    let working_song = std::fs::read(song_file(&root)).unwrap();
    let working_bass = std::fs::read(part_file(&root, "bass")).unwrap();
    let working_melody = std::fs::read(part_file(&root, "melody")).unwrap();

    let out = duet(&root, &["song", "restore", "porch", &folder]);
    ok(&out, "song restore");

    // Before anything was copied back, the working song was snapshotted, byte for byte.
    let kept = snapshot_dir(&root, &format!("before-restoring-{folder}"));
    assert_eq!(std::fs::read(kept.join("song.duet")).unwrap(), working_song);
    assert_eq!(
        std::fs::read(kept.join("parts/bass.duet")).unwrap(),
        working_bass
    );
    assert_eq!(
        std::fs::read(kept.join("parts/melody.duet")).unwrap(),
        working_melody
    );
    assert!(
        stdout(&out).contains(&kept.file_name().unwrap().to_string_lossy().into_owned()),
        "{}",
        stdout(&out)
    );

    // Then the snapshot's parts were copied back byte for byte, and its song with the working
    // status kept: a restore moves no status.
    assert_eq!(
        std::fs::read(part_file(&root, "bass")).unwrap(),
        std::fs::read(first.join("parts/bass.duet")).unwrap()
    );
    let restored = read_song(&root);
    let snapped =
        Document::<Song>::read(&std::fs::read_to_string(first.join("song.duet")).unwrap())
            .unwrap()
            .body;
    assert_eq!(restored.composition, snapped.composition);
    assert_eq!(
        restored.id, snapped.id,
        "a snapshot keeps the ids it copied"
    );
    assert_eq!(restored.status, Status::Sketch);
    assert!(
        stdout(&out).contains("kept its status, sketch"),
        "{}",
        stdout(&out)
    );

    // The journal states it: the snapshot taken first, and each file restored over its working
    // file. Journal and document still agree.
    let events: Vec<Event> = entries(&root)
        .iter()
        .filter_map(Line::read)
        .cloned()
        .collect();
    let replaced: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::DocWritten(w) if w.replaced => Some(w.path.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        replaced.ends_with(&[
            "songs/porch/song.duet".to_owned(),
            "songs/porch/parts/melody.duet".to_owned(),
            "songs/porch/parts/bass.duet".to_owned(),
        ]),
        "{replaced:?}"
    );
    let Some(Event::SnapshotTaken(taken)) = events.last() else {
        panic!("a restore journalled no snapshot.taken last: {events:?}");
    };
    assert_eq!(taken.snapshot, kept.file_name().unwrap().to_string_lossy());
    ok(&duet(&root, &["journal", "check"]), "journal check");

    // Both snapshots are as taken.
    let shown = stdout(&duet(&root, &["song", "show", "porch"]));
    assert_eq!(shown.matches("as taken").count(), 2, "{shown}");

    // A second restore of the same snapshot keeps the working song under a name of its own.
    ok(
        &duet(&root, &["song", "restore", "porch", &folder]),
        "restore again",
    );
    assert_eq!(snapshots(&root).len(), 3, "{:?}", snapshots(&root));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A restore never writes through a symbolic link: not one left where it stages a file
/// (`<file>.duet.next`), and not a song folder or `parts/` that is one. A file outside the
/// project that a link points at is left as it was.
#[cfg(unix)]
#[test]
fn a_restore_writes_through_no_symbolic_link() {
    use std::os::unix::fs::symlink;
    let dir = scratch("snapshot-links-restore");
    let root = porch(&dir);
    ok(
        &duet(&root, &["song", "snapshot", "porch", "first"]),
        "snapshot",
    );
    let folder = snapshot_dir(&root, "first")
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let outside = dir.join("outside.txt");
    std::fs::write(&outside, b"not the project's").unwrap();

    // A link where the restore stages a part: it is replaced, never written through.
    symlink(&outside, root.join("songs/porch/parts/bass.duet.next")).unwrap();
    symlink(&outside, root.join("songs/porch/song.duet.next")).unwrap();
    ok(
        &duet(&root, &["song", "restore", "porch", &folder]),
        "restore beside a staged link",
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"not the project's");
    assert!(!root.join("songs/porch/parts/bass.duet.next").exists());

    // parts/ that is a link to a folder outside the project: refused, naming it.
    let elsewhere = dir.join("elsewhere");
    std::fs::rename(root.join("songs/porch/parts"), &elsewhere).unwrap();
    symlink(&elsewhere, root.join("songs/porch/parts")).unwrap();
    let before = tree(&elsewhere);
    let out = duet(&root, &["song", "restore", "porch", &folder]);
    assert!(!out.status.success(), "restored through a linked parts/");
    assert!(
        stderr(&out).contains("parts is a symbolic link"),
        "{}",
        stderr(&out)
    );
    assert_eq!(tree(&elsewhere), before, "the linked folder was written");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A snapshot that holds a symbolic link is not as taken: `duet song show` names the link and
/// does not follow it, so a link to the folder that holds it ends, and a restore is refused.
#[cfg(unix)]
#[test]
fn a_link_in_a_snapshot_is_reported_and_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = scratch("snapshot-links-show");
    let root = porch(&dir);
    ok(
        &duet(&root, &["song", "snapshot", "porch", "first"]),
        "snapshot",
    );
    let snap = snapshot_dir(&root, "first");
    let folder = snap.file_name().unwrap().to_string_lossy().into_owned();

    // A link back to the snapshot's own folder, and a stated part made a link.
    symlink(&snap, snap.join("parts/loop")).unwrap();
    let bass = snap.join("parts/bass.duet");
    let copy = dir.join("bass-copy.duet");
    std::fs::rename(&bass, &copy).unwrap();
    symlink(&copy, &bass).unwrap();

    let out = duet(&root, &["song", "show", "porch"]);
    ok(&out, "song show of a snapshot holding links");
    let shown = stdout(&out);
    assert!(
        shown.contains(&format!("{folder}: parts/loop is a symbolic link"))
            && shown.contains(&format!("{folder}: parts/bass.duet is a symbolic link")),
        "{shown}"
    );
    let out = duet(&root, &["song", "restore", "porch", &folder]);
    assert!(
        !out.status.success(),
        "restored from a snapshot holding links"
    );
    assert!(
        stderr(&out).contains("is a symbolic link"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A song folder that is a symbolic link is refused for a snapshot and a restore alike, naming
/// it, and the folder it points at is left as it was.
#[cfg(unix)]
#[test]
fn a_linked_song_folder_is_neither_snapshotted_nor_restored_through() {
    use std::os::unix::fs::symlink;
    let dir = scratch("snapshot-links-song");
    let root = porch(&dir);
    let elsewhere = dir.join("elsewhere");
    std::fs::rename(root.join("songs/porch"), &elsewhere).unwrap();
    symlink(&elsewhere, root.join("songs/porch")).unwrap();
    let before = tree(&elsewhere);
    let out = duet(&root, &["song", "snapshot", "porch", "first"]);
    assert!(
        !out.status.success(),
        "snapshotted through a linked song folder"
    );
    assert!(
        stderr(&out).contains("songs/porch is a symbolic link"),
        "{}",
        stderr(&out)
    );
    assert_eq!(tree(&elsewhere), before);
    std::fs::remove_dir_all(&dir).expect("clean up");
}
