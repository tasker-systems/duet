//! `duet song link` and `duet song place`, and `duet song show` listing a song's takes (the
//! placing-takes design, build task 3).
//!
//! Every take here is audio the test writes itself: nothing comes from `fixtures/`.

#[path = "capture/audio.rs"]
#[expect(
    dead_code,
    reason = "the capture tests' shared generators: these tests use the clicks alone"
)]
mod audio;

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_capture::Analysis;
use duet_e2e::{duet, ok, s, scratch};
use duet_schemas::song::{By, Placed};
use duet_schemas::{Document, Song};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A fresh project at `dir/name`.
fn project(dir: &Path, name: &str) -> PathBuf {
    ok(&duet(dir, &["init", name]), "init");
    dir.join(name)
}

/// Clicks at `bpm` for 16 beats: a take a rhythm capture reads.
fn clicks(bpm: f64) -> Vec<f32> {
    let period = 60.0 / bpm;
    let times: Vec<(f64, f32)> = (0..16)
        .map(|n| (0.5 + f64::from(n) * period, 0.6))
        .collect();
    audio::clicks(0.5 + 16.0 * period + 1.0, &times)
}

/// A take written to `dir/name`.
fn wav(dir: &Path, name: &str, samples: &[f32]) -> PathBuf {
    let path = dir.join(name);
    audio::write_wav(&path, samples);
    path
}

/// A project holding the song `wayfaring` (92 bpm, 4/4) and the take `beat`, clicks at 92 bpm.
fn with_song_and_take(name: &str) -> PathBuf {
    let dir = scratch(name);
    let root = project(&dir, "porch");
    ok(
        &duet(
            &root,
            &[
                "song",
                "new",
                "wayfaring",
                "--tempo",
                "92",
                "--meter",
                "4/4",
            ],
        ),
        "song new",
    );
    let file = wav(&dir, "beat.wav", &clicks(92.0));
    ok(&duet(&root, &["take", "add", s(&file)]), "take add");
    root
}

fn song_path(root: &Path) -> PathBuf {
    root.join("songs/wayfaring/song.duet")
}

fn song_bytes(root: &Path) -> Vec<u8> {
    std::fs::read(song_path(root)).unwrap()
}

fn song(root: &Path) -> Song {
    let text = String::from_utf8(song_bytes(root)).unwrap();
    Document::<Song>::read(&text).unwrap().body
}

fn placed(root: &Path) -> Placed {
    song(root).takes[0].placed.expect("the link is placed")
}

fn capture(root: &Path) {
    ok(&duet(root, &["capture", "rhythm", "beat"]), "capture");
}

/// The tempo the capture of `beat` measured: `60 × rate / timing.beat_period_frames`.
fn measured(root: &Path) -> (f64, f64) {
    let text = std::fs::read_to_string(root.join("takes/beat/beat.rhythm.capture.duet")).unwrap();
    let a = Document::<Analysis>::read(&text).unwrap().body;
    let rate = f64::from(a.rate);
    (
        60.0 * rate / a.timing.beat_period_frames,
        *a.timing.downbeat_frame.value() as f64 / rate,
    )
}

#[test]
fn link_before_any_capture_links_unplaced_and_says_the_next_capture_places_it() {
    let root = with_song_and_take("placing-link-unplaced");
    let out = duet(&root, &["song", "link", "wayfaring", "beat"]);
    ok(&out, "song link");

    let links = song(&root).takes;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].take.as_str(), "beat");
    assert!(links[0].r#for.is_empty());
    assert!(links[0].placed.is_none(), "no capture yet, so no placement");
    assert!(
        stderr(&out).contains("duet capture rhythm beat"),
        "names the next capture: {}",
        stderr(&out)
    );
}

#[test]
fn link_after_a_capture_places_it_estimated_from_that_capture() {
    let root = with_song_and_take("placing-link-estimated");
    capture(&root);
    let out = duet(&root, &["song", "link", "wayfaring", "beat"]);
    ok(&out, "song link");

    let (bpm, at) = measured(&root);
    let p = placed(&root);
    assert_eq!(p.by, By::Estimated);
    assert_eq!(p.bpm, Some(bpm), "the measured tempo, not the rounded one");
    assert_eq!(p.at_secs, at);
    assert_eq!(p.bar.to_string(), "1.1.0");
    assert!(
        stderr(&out).contains("beat.rhythm.capture.duet"),
        "names the analysis it used: {}",
        stderr(&out)
    );
}

#[test]
fn link_again_adds_its_for_parts_and_without_for_changes_nothing() {
    let root = with_song_and_take("placing-link-again");
    capture(&root);
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    let first = song_bytes(&root);

    let out = duet(&root, &["song", "link", "wayfaring", "beat"]);
    ok(&out, "link again");
    assert_eq!(
        song_bytes(&root),
        first,
        "a plain link again changes nothing"
    );
    assert!(
        stdout(&out).contains("linked already"),
        "says so: {}",
        stdout(&out)
    );

    let out = duet(
        &root,
        &[
            "song",
            "link",
            "wayfaring",
            "beat",
            "--for",
            "verse",
            "--for",
            "chorus",
        ],
    );
    ok(&out, "link with --for");
    let link = &song(&root).takes[0];
    let parts: Vec<&str> = link.r#for.iter().map(|p| p.as_str()).collect();
    assert_eq!(parts, ["verse", "chorus"]);
    assert_eq!(link.placed.unwrap().by, By::Estimated, "placement kept");
    let second = song_bytes(&root);

    ok(
        &duet(
            &root,
            &["song", "link", "wayfaring", "beat", "--for", "verse"],
        ),
        "link again with a part it has",
    );
    assert_eq!(
        song_bytes(&root),
        second,
        "a part it has again changes nothing"
    );
}

#[test]
fn place_with_bpm_sets_it_keeping_every_value_not_given() {
    let root = with_song_and_take("placing-place-bpm");
    capture(&root);
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    let estimate = placed(&root);

    let out = duet(
        &root,
        &["song", "place", "wayfaring", "beat", "--bpm", "90"],
    );
    ok(&out, "place --bpm");
    let p = placed(&root);
    assert_eq!(p.by, By::Set);
    assert_eq!(p.bpm, Some(90.0));
    assert_eq!(p.bar, estimate.bar, "the bar is the estimate's");
    assert_eq!(
        p.at_secs, estimate.at_secs,
        "the seconds are the estimate's"
    );
    assert!(
        stderr(&out).contains("was estimated") || stdout(&out).contains("was estimated"),
        "states what it replaced: {} {}",
        stdout(&out),
        stderr(&out)
    );
}

#[test]
fn place_click_with_bar_and_at_places_it_by_click_and_refuses_bpm_with_click() {
    let root = with_song_and_take("placing-place-click");
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");

    let out = duet(
        &root,
        &[
            "song",
            "place",
            "wayfaring",
            "beat",
            "--click",
            "--bar",
            "1.1.0",
            "--at",
            "2.5",
        ],
    );
    ok(&out, "place --click");
    let p = placed(&root);
    assert_eq!(p.by, By::Click);
    assert_eq!(p.bar.to_string(), "1.1.0");
    assert_eq!(p.at_secs, 2.5);
    assert_eq!(p.bpm, None, "the composition's map is the tempo");
    let before = song_bytes(&root);

    let out = duet(
        &root,
        &[
            "song",
            "place",
            "wayfaring",
            "beat",
            "--click",
            "--bpm",
            "90",
            "--bar",
            "1.1.0",
            "--at",
            "2.5",
        ],
    );
    assert!(!out.status.success(), "--bpm with --click is refused");
    assert_eq!(song_bytes(&root), before);

    let out = duet(&root, &["song", "place", "wayfaring", "beat", "--click"]);
    assert!(!out.status.success(), "--click needs --bar and --at");
    let said = stderr(&out);
    assert!(said.contains("--bar") && said.contains("--at"), "{said}");
    assert_eq!(song_bytes(&root), before);
}

#[test]
fn place_on_an_unplaced_link_without_every_value_is_refused_naming_each_and_leaves_the_song() {
    let root = with_song_and_take("placing-place-partial");
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    let before = song_bytes(&root);

    let out = duet(
        &root,
        &["song", "place", "wayfaring", "beat", "--bpm", "90"],
    );
    assert!(!out.status.success(), "refused");
    let said = stderr(&out);
    assert!(said.contains("--bar") && said.contains("--at"), "{said}");
    assert!(
        !said.contains("--bpm 90"),
        "does not ask again for what it was given: {said}"
    );
    assert_eq!(song_bytes(&root), before, "the song is left as it was");

    let out = duet(
        &root,
        &[
            "song",
            "place",
            "wayfaring",
            "beat",
            "--bpm",
            "90",
            "--bar",
            "1.1.0",
            "--at",
            "0.5",
        ],
    );
    ok(&out, "every value given");
    let p = placed(&root);
    assert_eq!((p.by, p.bpm, p.at_secs), (By::Set, Some(90.0), 0.5));
}

#[test]
fn place_on_a_take_not_linked_links_it_then_places_it() {
    let root = with_song_and_take("placing-place-unlinked");
    let out = duet(
        &root,
        &[
            "song",
            "place",
            "wayfaring",
            "beat",
            "--bpm",
            "90",
            "--bar",
            "1.1.0",
            "--at",
            "0.5",
        ],
    );
    ok(&out, "place on an unlinked take");
    let links = song(&root).takes;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].take.as_str(), "beat");
    assert!(links[0].r#for.is_empty());
    assert_eq!(links[0].placed.unwrap().by, By::Set);
}

#[test]
fn link_refuses_a_take_not_in_the_project_naming_take_add() {
    let root = with_song_and_take("placing-link-missing");
    let before = song_bytes(&root);
    let out = duet(&root, &["song", "link", "wayfaring", "nothing"]);
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("duet take add"), "names the command: {said}");
    assert!(said.contains("nothing"), "{said}");
    assert_eq!(song_bytes(&root), before);

    let out = duet(
        &root,
        &[
            "song",
            "place",
            "wayfaring",
            "nothing",
            "--bpm",
            "90",
            "--bar",
            "1.1.0",
            "--at",
            "0",
        ],
    );
    assert!(!out.status.success());
    assert!(stderr(&out).contains("duet take add"), "{}", stderr(&out));
    assert_eq!(song_bytes(&root), before);
}

#[test]
fn song_show_lists_each_take_with_its_placement_and_states_a_for_naming_no_part() {
    let root = with_song_and_take("placing-show");
    let shown = stdout(&duet(&root, &["song", "show", "wayfaring"]));
    assert!(shown.contains("takes none"), "{shown}");

    capture(&root);
    ok(
        &duet(
            &root,
            &["song", "link", "wayfaring", "beat", "--for", "verse"],
        ),
        "link",
    );
    let out = duet(&root, &["song", "show", "wayfaring"]);
    ok(&out, "show");
    let shown = stdout(&out);
    let (bpm, _) = measured(&root);
    assert!(shown.contains("takes\n"), "{shown}");
    assert!(
        shown.contains("beat, for verse, estimated: bar 1.1.0 at "),
        "{shown}"
    );
    assert!(shown.contains(&format!("{bpm:.1} bpm")), "{shown}");
    assert!(
        shown.contains("the take \"beat\" is for the part \"verse\", and the song has no part"),
        "states a for naming no part: {shown}"
    );

    ok(
        &duet(
            &root,
            &[
                "song",
                "place",
                "wayfaring",
                "beat",
                "--click",
                "--bar",
                "1.1.0",
                "--at",
                "1",
            ],
        ),
        "place",
    );
    let shown = stdout(&duet(&root, &["song", "show", "wayfaring"]));
    assert!(
        shown.contains("beat, for verse, click: bar 1.1.0 at 1.000 s"),
        "{shown}"
    );

    ok(
        &duet(
            &root,
            &["song", "new", "second", "--tempo", "92", "--meter", "4/4"],
        ),
        "new",
    );
    let file = wav(&root, "fresh.wav", &clicks(100.0));
    ok(&duet(&root, &["take", "add", s(&file)]), "take add fresh");
    ok(
        &duet(&root, &["song", "link", "second", "fresh"]),
        "link second",
    );
    let shown = stdout(&duet(&root, &["song", "show", "second"]));
    assert!(shown.contains("fresh, for none, unplaced"), "{shown}");
}
