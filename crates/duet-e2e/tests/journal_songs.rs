//! The journal follows the songs build (bus design decisions 17 and 23, the domain model's
//! rulings 13 and 14): `duet song status` journals a `song.status-set` naming the song by its
//! id; a `doc.written` in a song's folder carries the song's id, and a part's its own; the
//! index's closing summary lists songs and parts by id with the name beside it; and
//! `duet journal check` reports a song whose `song.duet` disagrees with the journal, naming both
//! sides, and changes neither.

use std::path::{Path, PathBuf};

use duet_e2e::{duet_env, golden, ok, s, scratch};
use duet_schemas::journal::{Event, Indexed, Line};
use duet_schemas::song::Status;
use duet_schemas::{Document, PartBody, Project, Song, Tool, Version};
use serde_json::Value;

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

fn song_file(root: &Path, song: &str) -> PathBuf {
    root.join("songs").join(song).join("song.duet")
}

fn read_song(root: &Path, song: &str) -> Song {
    let text = std::fs::read_to_string(song_file(root, song)).unwrap();
    Document::<Song>::read(&text).unwrap().body
}

/// Sets a song's status in its document as a hand would, outside duet.
fn set_by_hand(root: &Path, song: &str, status: &str) {
    let path = song_file(root, song);
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["body"]["status"] = status.into();
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

#[test]
fn song_status_in_a_project_journals_a_status_set_naming_the_song_by_its_id() {
    let dir = scratch("journal-songs-status");
    let root = project(&dir, "porch");
    ok(&duet(&root, &["song", "new", "wayfaring"]), "song new");
    ok(
        &duet(&root, &["song", "status", "wayfaring", "sketch"]),
        "song status",
    );
    let id = read_song(&root, "wayfaring").id;

    let lines = entries(&root);
    let events: Vec<&Event> = lines.iter().map(|l| l.read().unwrap()).collect();
    assert_eq!(events.len(), 3, "{events:?}");
    for written in &events[..2] {
        let Event::DocWritten(written) = written else {
            panic!("a doc.written first: {written:?}");
        };
        assert_eq!(written.path.as_str(), "songs/wayfaring/song.duet");
        assert_eq!(
            written.song.as_ref(),
            Some(&id),
            "the song's id rides with it"
        );
    }
    let Event::SongStatusSet(set) = events[2] else {
        panic!("then the move: {:?}", events[2]);
    };
    assert_eq!(
        (&set.song, set.name.as_str(), set.from, set.to),
        (&id, "wayfaring", Status::Idea, Status::Sketch)
    );
    assert_eq!(lines[2].by.who, "tester");
    assert_eq!(
        lines[1].cause, lines[2].cause,
        "the document and the move are one command's"
    );

    // A refused move journals nothing.
    let refused = duet(&root, &["song", "status", "wayfaring", "arranged"]);
    assert!(!refused.status.success());
    assert_eq!(entries(&root).len(), 3, "nothing journalled");

    let check = duet(&root, &["journal", "check"]);
    ok(&check, "journal check");
    assert!(
        stdout(&check).ends_with("nothing wrong\n"),
        "{}",
        stdout(&check)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_status_changed_by_hand_in_song_duet_is_reported_by_journal_check_naming_both_and_left_as_it_is()
 {
    let dir = scratch("journal-songs-differs");
    let root = project(&dir, "porch");
    ok(&duet(&root, &["song", "new", "wayfaring"]), "song new");
    ok(&duet(&root, &["song", "new", "blood-sport"]), "song new");
    ok(
        &duet(&root, &["song", "status", "wayfaring", "sketch"]),
        "song status",
    );
    ok(&duet(&root, &["journal", "check"]), "clean before the edit");

    set_by_hand(&root, "wayfaring", "arranged");
    set_by_hand(&root, "blood-sport", "developing");
    let before = (
        std::fs::read(song_file(&root, "wayfaring")).unwrap(),
        std::fs::read(song_file(&root, "blood-sport")).unwrap(),
        std::fs::read(duet_bus::index_path(&root)).unwrap(),
    );
    let check = duet(&root, &["journal", "check"]);
    assert!(!check.status.success(), "a disagreement is wrong");
    let shown = stdout(&check);
    let (wayfaring, blood) = (
        read_song(&root, "wayfaring").id,
        read_song(&root, "blood-sport").id,
    );
    assert!(
        shown.contains(&format!(
            "song wayfaring ({wayfaring}): its song.duet says arranged, and the journal's last \
             move, "
        )) && shown.contains(", leaves it at sketch; neither is changed"),
        "{shown}"
    );
    assert!(
        shown.contains(&format!(
            "song blood-sport ({blood}): its song.duet says developing, and the journal has no \
             move for it, so leaves it at idea; neither is changed"
        )),
        "{shown}"
    );
    assert!(shown.ends_with("2 things wrong\n"), "{shown}");
    assert_eq!(
        (
            std::fs::read(song_file(&root, "wayfaring")).unwrap(),
            std::fs::read(song_file(&root, "blood-sport")).unwrap(),
            std::fs::read(duet_bus::index_path(&root)).unwrap(),
        ),
        before,
        "the check repairs nothing"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_index_lists_a_song_and_its_part_by_id_with_the_name_beside_it() {
    let dir = scratch("journal-songs-index");
    let root = project(&dir, "porch");
    std::fs::write(
        root.join(".duet/duet.toml"),
        "[journal]\nsegment-size = 1\n",
    )
    .unwrap();
    ok(&duet(&root, &["song", "new", "wayfaring"]), "song new");
    let lead = root.join("songs/wayfaring/parts/lead.duet");
    ok(
        &duet(&root, &["convert", s(&golden("sketch.mid")), s(&lead)]),
        "convert into the song's parts/",
    );
    ok(&duet(&root, &["song", "new", "other"]), "song new, to roll");
    let song = read_song(&root, "wayfaring").id;
    let part = Document::<PartBody>::read(&std::fs::read_to_string(&lead).unwrap())
        .unwrap()
        .body
        .id;

    let journal = duet_bus::read(&root).unwrap();
    let closes: Vec<_> = journal
        .index
        .lines
        .iter()
        .filter_map(|l| l.line.closes.as_ref())
        .collect();
    assert!(
        closes.iter().any(
            |c| c.songs == [Indexed::new(Some(song.clone()), "wayfaring")]
                && c.parts == [Indexed::new(Some(part.clone()), "wayfaring/lead")]
        ),
        "{closes:?}"
    );
    let index = std::fs::read_to_string(duet_bus::index_path(&root)).unwrap();
    assert!(
        index.contains(&format!(
            r#""songs":[{{"id":"{song}","name":"wayfaring"}}],"parts":[{{"id":"{part}","name":"wayfaring/lead"}}]"#
        )),
        "{index}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
