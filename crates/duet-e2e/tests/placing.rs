//! `duet song link` and `duet song place`, and `duet song show` listing a song's takes (the
//! placing-takes design, build task 3); and a capture of a linked take placing it on each song
//! and stating where it disagrees (build task 4).
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
    with_song_at_and_take(name, "92")
}

/// A project holding the song `wayfaring` (at `tempo`, 4/4) and the take `beat`, clicks at 92
/// bpm.
fn with_song_at_and_take(name: &str, tempo: &str) -> PathBuf {
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
                tempo,
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

/// Captures `beat` again, into a stem of its own: a capture never overwrites its outputs.
fn capture_again(root: &Path, stem: &str, more: &[&str]) -> Output {
    let out_stem = format!("takes/beat/{stem}");
    let mut args = vec!["capture", "rhythm", "beat", "-o", out_stem.as_str()];
    args.extend_from_slice(more);
    let out = duet(root, &args);
    ok(&out, "capture again");
    out
}

/// The analysis a capture of `beat` wrote at `takes/beat/STEM.capture.duet`.
fn analysis_at(root: &Path, stem: &str) -> Analysis {
    let text =
        std::fs::read_to_string(root.join(format!("takes/beat/{stem}.capture.duet"))).unwrap();
    Document::<Analysis>::read(&text).unwrap().body
}

/// The song at `songs/<name>/song.duet`, as its document.
fn song_doc(root: &Path, name: &str) -> Document<Song> {
    let text = std::fs::read_to_string(root.join(format!("songs/{name}/song.duet"))).unwrap();
    Document::<Song>::read(&text).unwrap()
}

/// The part a capture of `beat` wrote at `takes/beat/STEM.duet`, as its statements.
fn part_statements(root: &Path, stem: &str) -> Vec<String> {
    let text = std::fs::read_to_string(root.join(format!("takes/beat/{stem}.duet"))).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["duet"]["statements"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .filter_map(|l| l.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_capture_places_an_unplaced_link_estimated_and_says_so() {
    let root = with_song_and_take("placing-capture-unplaced");
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    assert!(song(&root).takes[0].placed.is_none());

    let out = duet(&root, &["capture", "rhythm", "beat"]);
    ok(&out, "capture");
    let (bpm, at) = measured(&root);
    let p = placed(&root);
    assert_eq!(p.by, By::Estimated);
    assert_eq!(p.bpm, Some(bpm), "the measured tempo, not the rounded one");
    assert_eq!(p.at_secs, at);
    assert_eq!(p.bar.to_string(), "1.1.0");
    let said = "placed the take beat on wayfaring's bars, estimated: bar 1.1.0";
    assert!(stderr(&out).contains(said), "says so: {}", stderr(&out));
    assert!(
        part_statements(&root, "beat.rhythm")
            .iter()
            .any(|l| l.contains(said)),
        "the part states it: {:?}",
        part_statements(&root, "beat.rhythm")
    );
}

#[test]
fn a_capture_of_a_placed_take_states_a_tempo_that_disagrees_and_changes_neither() {
    let root = with_song_at_and_take("placing-capture-disagrees", "100");
    capture(&root);
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    let before = song_doc(&root, "wayfaring").body.composition;

    let out = capture_again(&root, "beat.again", &[]);
    let after = song_doc(&root, "wayfaring").body.composition;
    assert_eq!(after, before, "the composition is never written");
    let said = stderr(&out);
    assert!(said.contains("tempo estimated 92"), "{said}");
    assert!(said.contains("is 100 at bar 1"), "{said}");
    assert!(
        part_statements(&root, "beat.again")
            .iter()
            .any(|l| l.contains("is 100 at bar 1")),
        "the part states it"
    );
    let a = analysis_at(&root, "beat.again");
    let tempo = *a.timing.tempo_bpm.value();
    assert!(
        (tempo - 92.0).abs() < 1.0,
        "the analysis keeps its own estimate, not the song's: {tempo}"
    );
    assert_eq!(placed(&root).by, By::Estimated);
}

#[test]
fn a_later_capture_leaves_a_set_placement_and_states_what_disagrees_with_it() {
    let root = with_song_and_take("placing-capture-set");
    capture(&root);
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    ok(
        &duet(
            &root,
            &["song", "place", "wayfaring", "beat", "--bpm", "90"],
        ),
        "place --bpm",
    );
    let before = placed(&root);
    let bytes = song_bytes(&root);

    let out = capture_again(&root, "beat.again", &[]);
    assert_eq!(placed(&root), before, "a set placement is kept");
    assert_eq!(song_bytes(&root), bytes, "the song is not written");
    let said = stderr(&out);
    assert!(
        said.contains("the placement is set at 90 bpm"),
        "states the set tempo: {said}"
    );
    assert!(!said.contains("placed the take beat"), "{said}");
}

/// A project holding the songs `wayfaring` and `second`, both at 92 bpm and both linking the
/// take `beat`, unplaced.
fn two_songs_linking(name: &str) -> PathBuf {
    let root = with_song_and_take(name);
    ok(
        &duet(
            &root,
            &["song", "new", "second", "--tempo", "92", "--meter", "4/4"],
        ),
        "song new second",
    );
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    ok(&duet(&root, &["song", "link", "second", "beat"]), "link");
    root
}

#[test]
fn a_take_linked_to_two_songs_is_placed_and_stated_for_each() {
    let root = two_songs_linking("placing-capture-two");
    let out = duet(&root, &["capture", "rhythm", "beat"]);
    ok(&out, "capture");
    let (bpm, _) = measured(&root);
    let said = stderr(&out);
    for name in ["wayfaring", "second"] {
        let link = &song_doc(&root, name).body.takes[0];
        let p = link.placed.expect("placed");
        assert_eq!((p.by, p.bpm), (By::Estimated, Some(bpm)), "{name}");
        assert!(
            said.contains(&format!("placed the take beat on {name}'s bars, estimated")),
            "{name}: {said}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_song_that_cannot_be_written_is_named_and_the_other_is_still_placed() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = two_songs_linking("placing-capture-refused");
    let locked = root.join("songs/second");
    let bytes = std::fs::read(locked.join("song.duet")).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = duet(&root, &["capture", "rhythm", "beat"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    ok(&out, "the capture itself succeeds");

    let said = stderr(&out);
    assert!(
        said.contains("not placed on second's bars"),
        "names the song: {said}"
    );
    assert_eq!(
        std::fs::read(locked.join("song.duet")).unwrap(),
        bytes,
        "the refused song is left as it was"
    );
    assert_eq!(placed(&root).by, By::Estimated, "the other is placed");
    assert!(
        said.contains("placed the take beat on wayfaring's bars"),
        "{said}"
    );
}

#[test]
fn a_song_that_does_not_read_is_stated_and_the_linking_song_is_still_placed() {
    let root = with_song_and_take("placing-capture-unreadable");
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");
    let broken = root.join("songs/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("song.duet"), "not a song").unwrap();

    let out = duet(&root, &["capture", "rhythm", "beat"]);
    ok(&out, "the capture itself succeeds");
    let said = stderr(&out);
    assert_eq!(
        placed(&root).by,
        By::Estimated,
        "the linking song is placed"
    );
    assert!(
        said.contains("placed the take beat on wayfaring's bars"),
        "{said}"
    );
    assert!(
        said.contains("songs/broken could not be read, so the take may be linked there and was not placed on it"),
        "names the unreadable song: {said}"
    );
}

#[test]
fn a_given_tempo_places_the_link_and_is_stated_as_given() {
    let root = with_song_at_and_take("placing-capture-given", "100");
    ok(&duet(&root, &["song", "link", "wayfaring", "beat"]), "link");

    let out = duet(&root, &["capture", "rhythm", "beat", "--tempo", "92"]);
    ok(&out, "capture --tempo");
    let (bpm, _) = measured(&root);
    let p = placed(&root);
    assert_eq!(p.by, By::Estimated, "placed from the capture");
    assert_eq!(p.bpm, Some(bpm));
    assert!((bpm - 92.0).abs() < 1e-9, "the given tempo: {bpm}");
    let said = stderr(&out);
    assert!(
        said.contains("tempo given 92 bpm; wayfaring is 100 at bar 1"),
        "{said}"
    );
    assert!(!said.contains("tempo estimated"), "{said}");
}

// `duet record <take> --song <song>` (build task 7): the take is placed by the click it was
// played to.

/// The opt-in for the tests that play the click from the speakers and record it: they need an
/// input and an output device and a quiet room, which CI has none of.
const SPEAKERS: &str = "DUET_E2E_SPEAKERS";

/// How far a tick heard in the take may fall from where the placement puts it.
const TOLERANCE_MS: f64 = 2.0;

/// Whether the speakers-on tests run here; when not, says why they are skipped.
fn speakers_on() -> bool {
    let on = std::env::var_os(SPEAKERS).is_some();
    if !on {
        eprintln!(
            "skipped: set {SPEAKERS}=1 to play the click from the speakers and record it, in a \
             quiet room with an input and an output device"
        );
    }
    on
}

/// A project holding the song `s`, at 92 bpm in 4/4.
fn with_song_s(name: &str) -> PathBuf {
    let dir = scratch(name);
    let root = project(&dir, "porch");
    ok(
        &duet(
            &root,
            &["song", "new", "s", "--tempo", "92", "--meter", "4/4"],
        ),
        "song new",
    );
    root
}

fn song_s(root: &Path) -> Song {
    let text = std::fs::read_to_string(root.join("songs/s/song.duet")).unwrap();
    Document::<Song>::read(&text).unwrap().body
}

#[test]
fn record_song_is_refused_before_any_device_outside_a_project_for_an_unknown_song_and_for_a_file_output()
 {
    let dir = scratch("placing-record-refused");
    let outside = duet(&dir, &["record", "t", "--song", "s", "--seconds", "1"]);
    assert!(!outside.status.success());
    let said = stderr(&outside);
    assert!(
        said.contains("--song") && said.contains("no project here"),
        "outside a project: {said}"
    );

    let root = with_song_s("placing-record-refused-song");
    let unknown = duet(&root, &["record", "t", "--song", "nope", "--seconds", "1"]);
    assert!(!unknown.status.success());
    let said = stderr(&unknown);
    assert!(
        said.contains("no song nope") && said.contains("its songs are:\n  s"),
        "names the project's songs: {said}"
    );
    assert!(!root.join("takes/t").exists(), "nothing was recorded");

    let file = duet(
        &root,
        &["record", "x.flac", "--song", "s", "--seconds", "1"],
    );
    assert!(!file.status.success());
    let said = stderr(&file);
    assert!(
        said.contains("--song takes a take's name") && said.contains("duet record x --song s"),
        "a file output: {said}"
    );
    assert!(!root.join("x.flac").exists(), "nothing was recorded");
    assert!(song_s(&root).takes.is_empty(), "no refusal links anything");
}

#[test]
fn record_calibrate_is_refused_before_any_device_with_a_take_with_song_and_with_seconds() {
    let dir = scratch("placing-calibrate-refused");
    for (args, with) in [
        (&["record", "--calibrate", "t"][..], "'[OUTPUT]'"),
        (
            &["record", "--calibrate", "--song", "s"][..],
            "'--song <SONG>'",
        ),
        (
            &["record", "--calibrate", "--seconds", "3"][..],
            "'--seconds <SECONDS>'",
        ),
    ] {
        let out = duet(&dir, args);
        assert!(!out.status.success(), "{args:?}");
        let said = stderr(&out);
        assert!(
            said.contains(&format!(
                "the argument '--calibrate' cannot be used with {with}"
            )),
            "{args:?}: {said}"
        );
    }
    assert!(
        !dir.join("no-config/calibration.toml").exists(),
        "nothing was calibrated"
    );
}

/// The channel 0 samples of a FLAC take, and its rate.
fn decoded(path: &Path) -> (Vec<f32>, u32) {
    use duet_capture::{FileSource, Source as _};
    let mut source = FileSource::open(path).expect("open the take");
    let (mut samples, mut rate) = (Vec::new(), 0);
    while let Some(chunk) = source.next_chunk().expect("read the take") {
        rate = chunk.rate;
        let channels = usize::from(chunk.channels.max(1));
        samples.extend(chunk.samples.iter().step_by(channels));
    }
    (samples, rate)
}

/// Seconds into the take where the tick expected at `at_secs` is heard: the first sample from
/// 50 ms before it whose magnitude exceeds 8 times `floor` (the median absolute level of the
/// take's first 100 ms), the rule Task 7 measured the stamps' error by. Its own code, not
/// calibration's, so the witness does not check duet by duet's own finder.
fn onset(samples: &[f32], rate: u32, at_secs: f64, floor: f32) -> Option<f64> {
    let r = f64::from(rate);
    let from = ((at_secs - 0.05).max(0.0) * r).round() as usize;
    samples
        .iter()
        .skip(from)
        .position(|s| s.abs() > 8.0 * floor)
        .map(|i| (from + i) as f64 / r)
}

fn median_level(samples: &[f32]) -> f32 {
    let mut levels: Vec<f32> = samples.iter().map(|s| s.abs()).collect();
    levels.sort_by(f32::total_cmp);
    levels.get(levels.len() / 2).copied().unwrap_or(0.0)
}

#[test]
fn record_song_places_the_take_by_click_and_its_ticks_land_where_it_says() {
    use duet_core::Position;
    if !speakers_on() {
        return;
    }
    let root = with_song_s("placing-record-click");
    // Ruling 18: the pair is calibrated first, into the run's own config directory.
    let calibrated = duet(&root, &["record", "--calibrate"]);
    ok(
        &calibrated,
        "record --calibrate (it needs an input and an output device)",
    );
    let told = format!("{}{}", stdout(&calibrated), stderr(&calibrated));
    eprintln!("record --calibrate: {}", stdout(&calibrated).trim());
    assert!(told.contains(" → "), "names the pair: {told}");
    assert!(
        told.contains("12 of 12 ticks heard") || told.contains("11 of 12 ticks heard"),
        "{told}"
    );
    let file = root.join("no-config/calibration.toml");
    assert!(told.contains(s(&file)), "names the file: {told}");
    assert!(file.is_file());

    // Ten seconds: bar 1 falls about 2.9 s in (0.2 s of lead and a bar of count-in), and bar 3
    // 5.2 s after it.
    let out = duet(&root, &["record", "t", "--song", "s", "--seconds", "10"]);
    ok(
        &out,
        "record --song (it needs an input and an output device)",
    );
    let said = stderr(&out);
    assert!(said.contains("headphones"), "{said}");
    assert!(
        said.contains("calibrated: "),
        "says the calibration it applies: {said}"
    );

    let song = song_s(&root);
    assert_eq!(song.takes.len(), 1, "{said}");
    assert_eq!(song.takes[0].take.as_str(), "t");
    let p = song.takes[0].placed.expect("the link is placed");
    assert_eq!(p.by, By::Click);
    assert_eq!(p.bar, Position::bar(1));
    assert_eq!(p.bpm, None);
    let ppm = p.clock_ppm.expect("the drift is measured over ten seconds");

    let (samples, rate) = decoded(&root.join("takes/t/t.flac"));
    let frames = p.at_secs * f64::from(rate);
    assert!(
        (frames - frames.round()).abs() < 1e-6,
        "at_secs is a whole number of frames: {frames}"
    );
    let floor = median_level(&samples[..(rate / 10) as usize]);
    assert!(
        floor > 0.0,
        "the take's first 100 ms are digital silence: the input heard nothing"
    );

    let bar_one = onset(&samples, rate, p.at_secs, floor).expect("bar 1's tick is heard");
    let bar_one_ms = (bar_one - p.at_secs) * 1000.0;
    let between = song
        .composition
        .secs_between(Position::bar(1), Position::bar(3))
        .unwrap();
    let bar_three_at = p.at_secs + between * (1.0 + ppm / 1e6);
    let bar_three = onset(&samples, rate, bar_three_at, floor).expect("bar 3's tick is heard");
    let bar_three_ms = (bar_three - bar_three_at) * 1000.0;
    eprintln!(
        "record --song: at_secs {:.6} ({} frames at {rate} Hz), clock_ppm {ppm:.2}; bar 1's tick \
         heard {bar_one_ms:+.3} ms from it, bar 3's {bar_three_ms:+.3} ms",
        p.at_secs,
        frames.round()
    );
    assert!(
        bar_one_ms.abs() <= TOLERANCE_MS,
        "bar 1's tick is {bar_one_ms:+.3} ms from at_secs"
    );
    assert!(
        bar_three_ms.abs() <= TOLERANCE_MS,
        "bar 3's tick is {bar_three_ms:+.3} ms from where at_secs and clock_ppm put it"
    );
}

#[test]
fn record_song_with_no_reconcile_writes_no_clock_ppm() {
    if !speakers_on() {
        return;
    }
    let root = with_song_s("placing-record-no-reconcile");
    let out = duet(
        &root,
        &[
            "record",
            "t",
            "--song",
            "s",
            "--seconds",
            "5",
            "--no-reconcile",
        ],
    );
    ok(&out, "record --song --no-reconcile");
    let said = stderr(&out);
    assert!(
        said.contains("not calibrated for ") && said.contains("duet record --calibrate"),
        "a fresh config directory holds no calibration: {said}"
    );
    let p = song_s(&root).takes[0].placed.expect("the link is placed");
    assert_eq!(p.by, By::Click);
    assert_eq!(p.clock_ppm, None, "--no-reconcile leaves the drift out");
    assert!(said.contains("ppm"), "the drift is still stated: {said}");
}
