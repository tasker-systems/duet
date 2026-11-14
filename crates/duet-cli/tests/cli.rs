//! The `duet` binary end to end: every conversion writes the same music it wrote before
//! songs could carry an end, now stating where it ends, and a drum grid that ends in
//! empty bars comes back whole through MIDI with no bar count.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duet-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn duet(args: &[&Path]) {
    let out = Command::new(env!("CARGO_BIN_EXE_duet"))
        .args(args)
        .output()
        .expect("run duet");
    assert!(
        out.status.success(),
        "duet {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Every line of `new` but its header's `end` line, and that line's position.
fn without_end_line(new: &str) -> (String, Option<String>) {
    let mut end = None;
    let mut kept = String::new();
    let header = new.split("\n\n").next().unwrap_or("").len();
    let mut at = 0;
    for line in new.split_inclusive('\n') {
        if at < header && line.starts_with("end ") {
            assert!(end.is_none(), "one end line");
            end = Some(line.trim_end()["end ".len()..].to_owned());
        } else {
            kept.push_str(line);
        }
        at += line.len();
    }
    (kept, end)
}

/// The events of every track, end of track left out, and where each track ends.
fn midi_tracks(bytes: &[u8]) -> (Vec<Vec<String>>, Vec<u64>) {
    let smf = midly::Smf::parse(bytes).expect("MIDI");
    let mut tracks = Vec::new();
    let mut ends = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        let mut events = Vec::new();
        for e in track {
            tick += u64::from(e.delta.as_int());
            match e.kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack) => ends.push(tick),
                kind => events.push(format!("{tick} {kind:?}")),
            }
        }
        tracks.push(events);
    }
    (tracks, ends)
}

/// The golden files in `tests/golden` were written by duet at 145077f, before a song
/// could name an end, from `samples/sketch.txt` and `samples/groove.drums` (which now
/// state their ends, `end 5.1.0` and `end 3.1.0`, the defaults). Every file written now
/// holds exactly the same music, every note, control and conductor event at the same
/// tick, and differs only by stating its end: an `end` line in text, an `end` field in
/// a song's JSON and TOML, and every MIDI track ending there.
#[test]
fn every_output_holds_the_same_music_and_states_its_end() {
    let dir = scratch("golden");
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let sketch = root().join("samples/sketch.txt");
    let groove = root().join("samples/groove.drums");
    let out = |name: &str| dir.join(name);
    let p = |s: &'static str| Path::new(s);

    for name in ["sketch.mid", "sketch.txt", "sketch.json", "sketch.toml"] {
        duet(&[p("convert"), &sketch, &out(name)]);
    }
    duet(&[
        p("convert"),
        &out("sketch.mid"),
        &out("sketch-from-midi.txt"),
    ]);
    for name in ["groove.mid", "groove.txt", "groove.json"] {
        duet(&[p("drums"), p("render"), &groove, &out(name)]);
    }
    for name in ["groove.drums", "groove.drums.json"] {
        duet(&[
            p("drums"),
            p("read"),
            &out("groove.mid"),
            &out(name),
            p("--grid"),
            p("1/16"),
        ]);
    }

    // (file, the end it states, that end in ticks at 480 per quarter in 4/4)
    let ends = [("sketch", "5.1.0", 4 * 1920), ("groove", "3.1.0", 2 * 1920)];
    let mut compared = 0;
    for entry in std::fs::read_dir(&golden).expect("golden files") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("a name")
            .to_str()
            .expect("utf-8")
            .to_owned();
        let (_, end, end_tick) = ends
            .iter()
            .find(|(stem, ..)| name.starts_with(stem))
            .copied()
            .expect("a known sample");
        let want = std::fs::read(&path).expect("golden");
        let got = std::fs::read(dir.join(&name)).expect("written");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "mid" => {
                let (want_tracks, _) = midi_tracks(&want);
                let (got_tracks, got_ends) = midi_tracks(&got);
                assert_eq!(got_tracks, want_tracks, "{name}: the same events");
                assert!(
                    got_ends.iter().all(|t| *t == end_tick),
                    "{name}: {got_ends:?}"
                );
            }
            "json" if name.ends_with(".drums.json") => {
                // A grid's bars say where it ends; one ending on its last bar line has no
                // end field, so the exact form is unchanged.
                assert!(got == want, "{name} differs");
            }
            "json" => {
                let want: serde_json::Value = serde_json::from_slice(&want).expect("json");
                let mut got: serde_json::Value = serde_json::from_slice(&got).expect("json");
                let stated = got.as_object_mut().expect("an object").remove("end");
                assert_eq!(stated, Some(serde_json::Value::from(end)), "{name}");
                assert_eq!(got, want, "{name}: the same song");
            }
            "toml" => {
                let want: toml::Table = toml::from_slice(&want).expect("toml");
                let mut got: toml::Table = toml::from_slice(&got).expect("toml");
                let stated = got.remove("end");
                assert_eq!(stated, Some(toml::Value::from(end)), "{name}");
                assert_eq!(got, want, "{name}: the same song");
            }
            _ => {
                let got = String::from_utf8(got).expect("text");
                let (rest, stated) = without_end_line(&got);
                assert_eq!(stated.as_deref(), Some(end), "{name} states its end");
                assert_eq!(rest, String::from_utf8(want).expect("text"), "{name}");
            }
        }
        compared += 1;
    }
    assert_eq!(compared, 10);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Witness that the drum grid's trailing-bars caveat is gone: a grid ending in an empty
/// bar, rendered to MIDI and read back with no `--bars`, is the same text.
#[test]
fn a_grid_ending_in_empty_bars_survives_midi_with_no_bar_count() {
    let dir = scratch("rests");
    let text = "# notefmt drums v1: one row per lane, one column per step, | at bar lines\n\
        tpq 480\ntempo 120\ntime 4/4\nend 4.1.0\ngrid 1/8\nmap gm\n\n[bars 1-3]\n\
        #       1 . 2 . 3 . 4 .   1 . 2 . 3 . 4 .   1 . 2 . 3 . 4 .\n\
        snare | . . x . . . x . | . . . . . . . . | . . . . . . . . |\n\
        kick  | x . . . x . . . | . . . . . . . . | . . . . . . . . |\n";
    let grid = dir.join("rests.drums");
    std::fs::write(&grid, text).expect("write");
    let (mid, back) = (dir.join("rests.mid"), dir.join("back.drums"));
    let p = |s: &'static str| Path::new(s);
    duet(&[p("drums"), p("render"), &grid, &mid]);
    duet(&[p("drums"), p("read"), &mid, &back, p("--grid"), p("1/8")]);
    let read = std::fs::read_to_string(&back).expect("read back");
    assert_eq!(read, text);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A song that names no end is written stating its default, in every format.
#[test]
fn a_song_with_no_end_is_written_stating_its_default() {
    let dir = scratch("default");
    let take = dir.join("take.txt");
    std::fs::write(&take, "[track \"P\" ch1]\n2.3.0 C4 1 80\n").expect("write");
    let p = |s: &'static str| Path::new(s);
    for name in ["take.json", "take.toml", "take-out.txt"] {
        duet(&[p("convert"), &take, &dir.join(name)]);
    }
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).expect("read");
    assert!(
        read("take.json").contains("\"end\": \"3.1.0\""),
        "{}",
        read("take.json")
    );
    assert!(
        read("take.toml").contains("end = \"3.1.0\""),
        "{}",
        read("take.toml")
    );
    assert!(
        read("take-out.txt").contains("\nend 3.1.0\n"),
        "{}",
        read("take-out.txt")
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
