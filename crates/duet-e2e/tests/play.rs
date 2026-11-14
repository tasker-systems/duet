//! `duet play` end to end, as far as a machine with no audio device or MIDI port can
//! take it: every way it refuses to start, which is before it opens either. Sound and
//! port output themselves are witnessed only by a local run.

use std::path::Path;

use duet_e2e::{duet, duet_with, root, s, scratch};

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn sample() -> String {
    s(&root().join("samples/sketch.notes")).to_owned()
}

/// With no SoundFont anywhere and no --port, it fails, saying where it looked and how to
/// get sound: it is never silent.
#[test]
fn with_nothing_to_play_through_it_says_how_to_get_sound() {
    let dir = scratch("play-none");
    let out = duet(&dir, &["play", &sample()]);
    assert!(!out.status.success());
    let err = stderr(&out);
    let installed = dir.join("no-config/data/default.sf2");
    for want in [
        "no SoundFont to play with",
        &installed.display().to_string(),
        "--soundfont",
        "[play] soundfont",
        "--port",
    ] {
        assert!(err.contains(want), "{want:?} in\n{err}");
    }
    assert!(out.stdout.is_empty());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A SoundFont that is named but missing is refused by name, wherever it was named.
#[test]
fn a_named_soundfont_that_is_missing_is_refused() {
    let dir = scratch("play-missing");
    let gone = dir.join("gone.sf2");
    let out = duet(&dir, &["play", &sample(), "--soundfont", s(&gone)]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains(&format!("{} (--soundfont) is not a file", gone.display())),
        "{err}"
    );

    // From duet.toml, a relative path is the file's own directory's.
    let user = dir.join("user");
    std::fs::create_dir_all(&user).expect("user dir");
    std::fs::write(
        user.join("duet.toml"),
        "[play]\nsoundfont = \"fonts/gm.sf2\"\n",
    )
    .expect("config");
    let out = duet_with(&dir, &user, &["play", &sample()], b"");
    assert!(!out.status.success());
    let err = stderr(&out);
    let want = user.join("fonts/gm.sf2");
    assert!(
        err.contains(&format!(
            "{} ([play] soundfont in {}) is not a file",
            want.display(),
            user.join("duet.toml").display()
        )),
        "{err}"
    );
    // --soundfont comes before duet.toml's.
    let out = duet_with(
        &dir,
        &user,
        &["play", &sample(), "--soundfont", s(&gone)],
        b"",
    );
    assert!(
        stderr(&out).contains("(--soundfont) is not a file"),
        "{}",
        stderr(&out)
    );
    let show = duet_with(&dir, &user, &["config", "show"], b"");
    let show = String::from_utf8_lossy(&show.stdout);
    assert!(
        show.contains(&format!(
            "[play]\nsoundfont = \"{}\"  # {}\n",
            want.display(),
            user.join("duet.toml").display()
        )),
        "{show}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A port name no port has is refused, listing the ports there are. Where MIDI itself
/// cannot be opened, as in a container with no sequencer, that is the refusal instead.
#[test]
fn an_unknown_port_is_refused() {
    let dir = scratch("play-port");
    let out = duet(&dir, &["play", &sample(), "--port", "no such port 7f3a"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("no MIDI port is named \"no such port 7f3a\"; the ports are:")
            || err.contains("opening MIDI output"),
        "{err}"
    );
    assert!(!err.contains("panicked"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// --soundfont and --port name two different ways to play, so both together is refused.
#[test]
fn soundfont_and_port_together_are_refused() {
    let dir = scratch("play-both");
    let out = duet(
        &dir,
        &["play", &sample(), "--soundfont", "a.sf2", "--port", "x"],
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("cannot be used with"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A song that cannot be written as MIDI cannot be played, and says why before anything
/// else is looked for.
#[test]
fn a_song_that_cannot_be_midi_is_refused_first() {
    let dir = scratch("play-bad");
    let bad = dir.join("bad.notes");
    std::fs::write(&bad, "end 1.2.0\n\n[track \"P\" ch1]\n1.1.0 C4 4 80\n").expect("write");
    let out = duet(&dir, &[Path::new("play"), &bad]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("past the song's end"), "{err}");
    assert!(!err.contains("SoundFont"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
