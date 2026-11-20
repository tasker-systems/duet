//! `duet play` of a song (songs task 5, the songs spec's §1): a song is found by its folder, its
//! song.duet, or in a project its name, and each disagreement between a placed part's own
//! conductor and the composition is stated before anything plays. Where its notes land, at the
//! composition's tempo, is witnessed in `duet_actions::arrange`; no test here can hear it, and a
//! machine with no SoundFont or MIDI port stops after the song is read and arranged, which is as
//! far as these go.

use std::path::{Path, PathBuf};

use duet_e2e::{duet_env, notes_doc, ok, scratch};
use duet_schemas::{Document, Project, Tool, Version};
use serde_json::{Value, json};

/// Runs `duet` in `cwd`, with no config and no SoundFont.
fn duet(cwd: &Path, args: &[&str]) -> std::process::Output {
    duet_env(cwd, &cwd.join("no-config"), args, b"", &[])
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// What the kit's placement says, played at the song's 90 bpm.
const KIT_TEMPO: &str =
    "kit is at 120 bpm, and the song is at 90 bpm at 9.1.0: it plays at the song's tempo";

/// A project at `dir/porch` holding the song `porch` at 90 bpm: `melody` from bar 1, and `kit`,
/// a groove at its own 120 bpm, at bar 9 with `repeat: 2`.
fn porch(dir: &Path) -> PathBuf {
    let root = dir.join("porch");
    std::fs::create_dir_all(root.join(".duet")).unwrap();
    let project = Project::new("Porch", "2026-11-20".parse().unwrap(), Version::new(0, 4));
    std::fs::write(
        root.join(".duet/project.duet"),
        Document::new(Tool::new("duet", "0.4.0"), project)
            .write()
            .unwrap(),
    )
    .unwrap();
    ok(
        &duet(&root, &["song", "new", "porch", "--tempo", "90"]),
        "song new",
    );
    for (name, text) in [
        (
            "melody",
            "tempo 90\n\n[track \"Voice\" ch1]\n1.1.0 G4 4 90\n",
        ),
        (
            "kit",
            "tempo 120\nend 2.1.0\n\n[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n1.3.0 C2 0.25 100\n",
        ),
    ] {
        let made = notes_doc(&root, name, text);
        std::fs::rename(made, root.join(format!("songs/porch/parts/{name}.duet"))).unwrap();
    }
    edit_song(&root, |body| {
        body["composition"]["placements"] = json!([
            { "part": "melody", "at": "1.1.0" },
            { "part": "kit", "at": "9.1.0", "repeat": 2 },
        ]);
    });
    root
}

fn edit_song(root: &Path, edit: impl FnOnce(&mut Value)) {
    let path = root.join("songs/porch/song.duet");
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut doc["body"]);
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

/// A song is played by its name in a project, its folder, or its song.duet: each reads the song,
/// states the kit's tempo, and gets as far as looking for a SoundFont.
#[test]
fn a_song_is_played_by_its_name_its_folder_or_its_document() {
    let dir = scratch("play-song");
    let root = porch(&dir);
    for input in ["porch", "songs/porch", "songs/porch/song.duet"] {
        let out = duet(&root, &["play", input]);
        assert!(!out.status.success(), "{input}");
        let err = stderr(&out);
        assert!(err.contains(KIT_TEMPO), "{input}: {err}");
        assert!(err.contains("no SoundFont to play with"), "{input}: {err}");
        // The melody agrees with the song, so nothing is said of it.
        assert!(!err.contains("melody"), "{input}: {err}");
    }
    // From outside the project, its folder still names it.
    let out = duet(&dir, &["play", "porch/songs/porch"]);
    let err = stderr(&out);
    assert!(err.contains(KIT_TEMPO), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// To a port, the song is read and its disagreements said before the port is looked for.
#[test]
fn a_song_to_a_port_states_its_disagreements_first() {
    let dir = scratch("play-song-port");
    let root = porch(&dir);
    let out = duet(&root, &["play", "porch", "--port", "no such port 7f3a"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    let said = err.find(KIT_TEMPO).expect(&err);
    let port = err
        .find("no MIDI port is named")
        .or_else(|| err.find("opening MIDI output"))
        .expect(&err);
    assert!(said < port, "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A song the project does not have, a folder with no song.duet, and a placement of a part the
/// song does not have, are each refused by name; --from does not apply to a song.
#[test]
fn what_is_not_a_playable_song_is_refused_by_name() {
    let dir = scratch("play-song-refused");
    let root = porch(&dir);
    let err = stderr(&duet(&root, &["play", "verse"]));
    assert!(
        err.contains("has no song verse") && err.contains("Its songs are porch"),
        "{err}"
    );
    let err = stderr(&duet(&root, &["play", "songs"]));
    assert!(err.contains("is a folder with no song.duet"), "{err}");
    let err = stderr(&duet(&root, &["play", "porch", "--from", "midi"]));
    assert!(err.contains("--from names a part's format"), "{err}");

    edit_song(&root, |body| {
        body["composition"]["placements"] = json!([{ "part": "bass", "at": "1.1.0" }]);
    });
    let err = stderr(&duet(&root, &["play", "porch"]));
    assert!(
        err.contains("places the part \"bass\", and the song has no part of that name"),
        "{err}"
    );
    assert!(!err.contains("SoundFont"), "{err}");

    edit_song(&root, |body| {
        body["composition"]["placements"] = json!([]);
    });
    let err = stderr(&duet(&root, &["play", "porch"]));
    assert!(err.contains("the song places no part"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A drum grid in parts/ is placed and played as `duet drums render` makes a part of it.
#[test]
fn a_placed_drum_grid_plays() {
    let dir = scratch("play-song-grid");
    let root = porch(&dir);
    let grid = root.join("songs/porch/parts/kit.duet");
    std::fs::remove_file(&grid).unwrap();
    let beat = duet_e2e::root().join("samples/groove.drums");
    ok(
        &duet(
            &root,
            &["apply", duet_e2e::s(&beat), "-o", duet_e2e::s(&grid)],
        ),
        "a drum-grid document",
    );
    let out = duet(&root, &["play", "porch"]);
    let err = stderr(&out);
    assert!(err.contains("no SoundFont to play with"), "{err}");
    assert!(!err.contains("drum-grid document, and"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
