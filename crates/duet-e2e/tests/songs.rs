//! `duet song new | show | status`, on a song's folder, `songs/<song>/`: `new` mints the song's
//! id and starts it at idea, which a renamed folder keeps; `status` makes the four moves the
//! state machine draws and refuses every other by name, leaving the document as it was; `show`
//! refuses a placement of a part the song does not have, two parts with one name or one id, and
//! a meter change off a bar line, each by name.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, notes_doc, ok, s, scratch};
use duet_schemas::journal::Id;
use duet_schemas::project::Defaults;
use duet_schemas::song::Status;
use duet_schemas::{Document, PartBody, Project, Song, Tool, Version};
use serde_json::{Value, json};

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn song_file(dir: &Path, song: &str) -> PathBuf {
    dir.join("songs").join(song).join("song.duet")
}

fn read_song(dir: &Path, song: &str) -> Song {
    let path = song_file(dir, song);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Document::<Song>::read(&text)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .body
}

/// Edits a song's document as a hand would: its body as JSON.
fn edit_song(dir: &Path, song: &str, edit: impl FnOnce(&mut Value)) {
    let path = song_file(dir, song);
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut doc["body"]);
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

fn new_song(dir: &Path, song: &str, more: &[&str]) {
    let mut args = vec!["song", "new", song];
    args.extend_from_slice(more);
    ok(&duet(dir, &args), "song new");
}

#[test]
fn song_new_mints_an_id_and_renaming_the_songs_folder_leaves_it_unchanged() {
    let dir = scratch("song-new");
    new_song(
        &dir,
        "wayfaring-stranger",
        &["--title", "Wayfaring Stranger"],
    );
    let song = read_song(&dir, "wayfaring-stranger");
    assert!(
        song.id.as_str().parse::<Id>().is_ok(),
        "a UUIDv7: {}",
        song.id
    );
    assert_eq!(song.status, Status::Idea);
    assert_eq!(song.title, "Wayfaring Stranger");
    assert!(dir.join("songs/wayfaring-stranger/parts").is_dir());

    // Another song is another id; making one twice is refused, and leaves the first.
    new_song(&dir, "blood-sport", &[]);
    let other = read_song(&dir, "blood-sport");
    assert_ne!(other.id, song.id);
    assert_eq!(other.title, "blood-sport", "the title defaults to the name");
    let before = std::fs::read(song_file(&dir, "wayfaring-stranger")).unwrap();
    let again = duet(&dir, &["song", "new", "wayfaring-stranger"]);
    assert!(!again.status.success(), "made twice");
    assert!(
        stderr(&again).contains("made already"),
        "{}",
        stderr(&again)
    );
    assert_eq!(
        std::fs::read(song_file(&dir, "wayfaring-stranger")).unwrap(),
        before
    );

    // Renaming the folder renames the song for commands, and keeps its id.
    std::fs::rename(
        dir.join("songs/wayfaring-stranger"),
        dir.join("songs/stranger"),
    )
    .unwrap();
    let shown = duet(&dir, &["song", "show", "stranger"]);
    ok(&shown, "song show after the rename");
    assert!(
        stdout(&shown).contains(&format!("id {}", song.id)),
        "{}",
        stdout(&shown)
    );
    assert_eq!(read_song(&dir, "stranger").id, song.id);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn song_new_starts_its_composition_from_its_flags_then_the_projects_defaults() {
    let dir = scratch("song-defaults");
    let mut project = Project::new("Porch", "2026-11-18".parse().unwrap(), Version::new(0, 3));
    let mut defaults = Defaults::default();
    defaults.tempo_bpm = Some(92.0);
    defaults.meter = Some("3/4".parse().unwrap());
    defaults.key = Some("Ab".parse().unwrap());
    project.defaults = Some(defaults);
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    std::fs::write(
        dir.join(".duet/project.duet"),
        Document::new(Tool::new("duet", "0.3.0"), project)
            .write()
            .unwrap(),
    )
    .unwrap();
    new_song(&dir, "porch", &[]);
    new_song(&dir, "lift", &["--tempo", "70", "--key", "D dorian"]);
    let porch = read_song(&dir, "porch").composition;
    assert_eq!(porch.tempo[0].bpm, 92.0);
    assert_eq!(porch.meter[0].sig.to_string(), "3/4");
    assert_eq!(porch.key[0].key.to_string(), "Ab major");
    let lift = read_song(&dir, "lift").composition;
    assert_eq!(lift.tempo[0].bpm, 70.0);
    assert_eq!(
        lift.meter[0].sig.to_string(),
        "3/4",
        "the project's, not given"
    );
    assert_eq!(lift.key[0].key.to_string(), "D dorian");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn song_status_makes_each_allowed_move_and_refuses_every_other_by_name_leaving_the_song() {
    let dir = scratch("song-status");
    new_song(&dir, "porch", &[]);
    let path = song_file(&dir, "porch");
    let states = ["idea", "sketch", "developing", "arranged"];
    // Each step makes one allowed move, after trying every other move from where it stands.
    let walk = [
        ("idea", "sketch"),
        ("sketch", "developing"),
        ("developing", "arranged"),
        ("arranged", "developing"),
    ];
    for (from, to) in walk {
        assert_eq!(read_song(&dir, "porch").status.as_str(), from);
        let allowed = from.parse::<Status>().unwrap().moves()[0].as_str();
        for refused in states.iter().filter(|s| **s != allowed) {
            let before = std::fs::read(&path).unwrap();
            let out = duet(&dir, &["song", "status", "porch", refused]);
            assert!(!out.status.success(), "{from} → {refused} was made");
            let said = stderr(&out);
            assert!(
                said.contains(&format!("at {from}"))
                    && said.contains(&format!("moves only to {allowed}")),
                "{from} → {refused}: {said}"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                before,
                "{from} → {refused} changed the song"
            );
        }
        let out = duet(&dir, &["song", "status", "porch", to]);
        ok(&out, &format!("{from} → {to}"));
        assert!(
            stdout(&out).contains(&format!("{from} → {to}")),
            "{}",
            stdout(&out)
        );
        assert_eq!(read_song(&dir, "porch").status.as_str(), to);
    }
    let out = duet(&dir, &["song", "status", "porch", "finished"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("finished"), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A part in the song's `parts/`, made as a hand makes one: a notes view applied.
fn part(dir: &Path, song: &str, name: &str) -> PathBuf {
    let made = notes_doc(dir, name, "[track \"Bass\" ch2]\n1.1.0 G2 3 80\n");
    let path = dir
        .join("songs")
        .join(song)
        .join("parts")
        .join(format!("{name}.duet"));
    std::fs::rename(made, &path).unwrap();
    path
}

#[test]
fn song_show_states_each_part_and_refuses_an_unknown_part_and_two_parts_of_one_name_or_id() {
    let dir = scratch("song-show");
    new_song(&dir, "porch", &[]);
    let bass = part(&dir, "porch", "bass");
    edit_song(&dir, "porch", |body| {
        body["composition"]["placements"] = json!([{ "part": "bass", "at": "9.1.0", "repeat": 2 }]);
    });
    let out = duet(&dir, &["song", "show", "porch"]);
    ok(&out, "song show");
    let shown = stdout(&out);
    let bass_id = Document::<PartBody>::read(&std::fs::read_to_string(&bass).unwrap())
        .unwrap()
        .body
        .id;
    assert!(
        shown.contains(&format!("bass (part), id {bass_id}, by hand, placed once"))
            && shown.contains("placements bass at 9.1.0 ×2")
            && shown.contains("status idea (moves to sketch)"),
        "{shown}"
    );

    // A placement of a part the song does not have.
    edit_song(&dir, "porch", |body| {
        body["composition"]["placements"] = json!([
            { "part": "bass", "at": "9.1.0" },
            { "part": "melody", "at": "1.1.0" },
        ]);
    });
    let out = duet(&dir, &["song", "show", "porch"]);
    assert!(!out.status.success(), "an unknown part was shown");
    assert!(
        stderr(&out).contains("placements[1] places the part \"melody\"")
            && stderr(&out).contains("its parts are bass"),
        "{}",
        stderr(&out)
    );

    // Two parts with one name: a stem is a part's name, whatever case its .duet is in.
    edit_song(&dir, "porch", |body| {
        body["composition"]["placements"] = json!([]);
    });
    std::fs::copy(&bass, dir.join("songs/porch/parts/bass.DUET")).unwrap();
    let out = duet(&dir, &["song", "show", "porch"]);
    assert!(!out.status.success(), "two parts named bass were shown");
    assert!(
        stderr(&out).contains("two parts are named \"bass\""),
        "{}",
        stderr(&out)
    );
    std::fs::remove_file(dir.join("songs/porch/parts/bass.DUET")).unwrap();

    // Two parts with one id: a file copied by hand.
    std::fs::copy(&bass, dir.join("songs/porch/parts/bass-copy.duet")).unwrap();
    let out = duet(&dir, &["song", "show", "porch"]);
    assert!(!out.status.success(), "two parts with one id were shown");
    assert!(
        stderr(&out).contains("\"bass\" and \"bass-copy\" have one id"),
        "{}",
        stderr(&out)
    );
    // `duet convert` makes a copy that is a new part, with its own id.
    std::fs::remove_file(dir.join("songs/porch/parts/bass-copy.duet")).unwrap();
    ok(
        &duet(
            &dir,
            &[
                "convert",
                s(&bass),
                s(&dir.join("songs/porch/parts/bass-copy.duet")),
            ],
        ),
        "convert a copy",
    );
    ok(&duet(&dir, &["song", "show", "porch"]), "a converted copy");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_meter_change_off_a_bar_line_is_refused_by_name_and_the_status_moves_nothing() {
    let dir = scratch("song-meter");
    new_song(&dir, "porch", &["--meter", "3/4"]);
    edit_song(&dir, "porch", |body| {
        body["composition"]["meter"] = json!([
            { "at": "1.1.0", "sig": "3/4" },
            { "at": "9.2.0", "sig": "4/4" },
        ]);
    });
    let before = std::fs::read(song_file(&dir, "porch")).unwrap();
    for args in [
        vec!["song", "show", "porch"],
        vec!["song", "status", "porch", "sketch"],
    ] {
        let out = duet(&dir, &args);
        assert!(!out.status.success(), "{args:?} read the song");
        assert!(
            stderr(&out)
                .contains("composition.meter[1] changes the meter at 9.2.0, off a bar line"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(std::fs::read(song_file(&dir, "porch")).unwrap(), before);
    std::fs::remove_dir_all(&dir).expect("clean up");
}
