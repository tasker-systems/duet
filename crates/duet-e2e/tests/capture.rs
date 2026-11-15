//! `duet capture melody|rhythm`: a take in, the exact performance (`.duet`, a song document), a
//! quantized score (`.abc`, or `.grid.duet`, a drum-grid document) and the analysis
//! (`.capture.duet`) out, with every estimate stated on standard error and in the files, and
//! every refusal made before anything is written.
//!
//! The takes are generated here (see `capture/audio.rs`) and written to scratch directories.

#[path = "capture/audio.rs"]
mod audio;

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_core::Song;
use duet_drums::DrumGrid;
use duet_e2e::{duet, duet_with, ok, root, s, scratch};
use duet_schemas::{AnyDocument, Document};

/// A melody in E-flat major at 120 a minute, from half a second in: the tonic triad and
/// scale, with the tonic and dominant held longest.
fn eb_major() -> Vec<f32> {
    let q = 0.5;
    let bars: [&[(u8, f64)]; 4] = [
        &[(63, 2.0 * q), (67, q), (70, q)],
        &[(75, q), (70, q), (67, q), (65, q)],
        &[(68, q), (72, q), (70, 2.0 * q)],
        &[(65, q), (62, q), (63, 2.0 * q)],
    ];
    let notes: Vec<(u8, f64)> = bars
        .iter()
        .cycle()
        .take(8)
        .flat_map(|b| b.iter().copied())
        .collect();
    audio::notes(0.5, &notes)
}

/// Clicks at `bpm` from `lead` for `beats` beats, with `amplitudes` cycling over them and the
/// `n`th click moved by `nudge(n)` seconds.
fn click_track(
    bpm: f64,
    lead: f64,
    beats: usize,
    amplitudes: &[f32],
    nudge: impl Fn(usize) -> f64,
) -> Vec<f32> {
    let period = 60.0 / bpm;
    let times: Vec<(f64, f32)> = (0..beats)
        .map(|n| {
            (
                lead + n as f64 * period + nudge(n),
                amplitudes[n % amplitudes.len()],
            )
        })
        .collect();
    audio::clicks(lead + beats as f64 * period + 1.0, &times)
}

/// Clicks whose tempo slides in a straight line from `from` to `to` bpm over `secs`.
fn sliding_clicks(from: f64, to: f64, secs: f64) -> Vec<f32> {
    const LEAD: f64 = 0.5;
    // Beats elapsed by t: (from t + slope t² / 2) / 60, with slope in bpm per second.
    let half_slope = (to - from) / secs / 2.0;
    let times: Vec<(f64, f32)> = (0..)
        .map(|k| {
            let beats = f64::from(k);
            let t = (-from + (from * from + 4.0 * half_slope * 60.0 * beats).sqrt())
                / (2.0 * half_slope);
            (t + LEAD, 0.5)
        })
        .take_while(|&(t, _)| t < secs + LEAD)
        .collect();
    audio::clicks(secs + LEAD + 1.0, &times)
}

/// Low bursts on beats 1 and 3 and high ones on 2 and 4, at 120 a minute for four bars.
fn kick_and_hat() -> Vec<f32> {
    let (kicks, hats): (Vec<usize>, Vec<usize>) = (0..16).partition(|n| n % 2 == 0);
    let at = |beats: Vec<usize>, len: f64| -> Vec<(f64, f64)> {
        beats
            .into_iter()
            .map(|n| (0.5 + n as f64 * 0.5, len))
            .collect()
    };
    let secs = 9.5;
    let mut signal = audio::bursts(secs, &at(kicks, 0.2), 40.0, 150.0);
    audio::add(
        &mut signal,
        &audio::bursts(secs, &at(hats, 0.05), 6_000.0, 10_000.0),
    );
    signal
}

/// A take written to `dir/name`.
fn take(dir: &Path, name: &str, samples: &[f32]) -> PathBuf {
    let path = dir.join(name);
    audio::write_wav(&path, samples);
    path
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The song in a song document, as duet-schemas reads it.
fn song_of(path: &Path) -> Song {
    document_of(path).body
}

/// A song document, as duet-schemas reads it.
fn document_of(path: &Path) -> Document<Song> {
    Document::read(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A drum-grid document, as duet-schemas reads it.
fn grid_document_of(path: &Path) -> Document<DrumGrid> {
    Document::read(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The grid in a drum-grid document.
fn grid_of(path: &Path) -> DrumGrid {
    grid_document_of(path).body
}

/// The body of a capture-analysis document, read as one by duet-schemas, as JSON.
fn analysis_of(path: &Path) -> serde_json::Value {
    let text = read(path);
    match AnyDocument::read(&text) {
        Ok(doc) => assert_eq!(doc.kind(), "capture-analysis", "{}", path.display()),
        Err(e) => panic!("{}: {e}", path.display()),
    }
    let json: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    json["body"].clone()
}

/// The comment lines of a file: those starting with `prefix`.
fn comments(text: &str, prefix: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with(prefix))
        .map(str::to_owned)
        .collect()
}

fn capture(dir: &Path, args: &[&str]) -> Output {
    let mut all = vec!["capture"];
    all.extend_from_slice(args);
    duet(dir, &all)
}

/// The files in `dir`, other than the empty config directory `duet` is run with.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n != "no-config")
        .collect();
    names.sort();
    names
}

/// Every note start of a song, in ticks.
fn ticks(song: &Song) -> Vec<u64> {
    let map = song.time_map().expect("time map");
    song.tracks
        .iter()
        .flat_map(|t| t.notes.iter().map(|n| map.tick(n.at).expect("tick")))
        .collect()
}

#[test]
fn melody_writes_notes_and_abc_beside_the_input() {
    let dir = scratch("capture-melody");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav)]);
    ok(&out, "capture melody");
    let song = song_of(&dir.join("tune.duet"));
    assert!(song.tracks[0].notes.len() >= 20, "{song:?}");
    // Onset strengths give velocities off the dynamics' levels, so the ABC snaps them.
    let err = stderr(&out);
    assert!(
        err.lines().any(|l| l.starts_with("velocities: ")
            && l.contains("snapped to the nearest dynamic's level in the .abc")),
        "{err}"
    );
    let check = duet(&dir, &["abc", "check", s(&dir.join("tune.abc"))]);
    ok(&check, "abc check");
    analysis_of(&dir.join("tune.capture.duet"));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Capture's song document is a document like any other: `duet song show` reads it by its
/// envelope, with no `--from`, and prints the performance capture wrote.
#[test]
fn song_show_reads_captures_song_document_directly() {
    let dir = scratch("capture-show");
    let wav = take(&dir, "tune.wav", &eb_major());
    ok(&capture(&dir, &["melody", s(&wav)]), "capture melody");
    let path = dir.join("tune.duet");
    let doc = document_of(&path);
    assert_eq!(doc.envelope().kind, "song");
    assert_eq!(doc.tool.name, "duet");
    let out = duet(&dir, &["song", "show", s(&path)]);
    ok(&out, "song show tune.duet");
    let shown = Song::from_text(&String::from_utf8_lossy(&out.stdout)).expect("notefmt");
    assert_eq!(shown, doc.body);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Every file a melody capture writes is refused, rather than overwritten, when it exists.
#[test]
fn melody_refuses_to_overwrite_each_of_its_outputs() {
    let dir = scratch("capture-refuse-each");
    let wav = take(&dir, "tune.wav", &eb_major());
    for name in [
        "tune.duet",
        "tune.abc",
        "tune.capture.duet",
        "tune.performance.duet.jsonl",
    ] {
        let path = dir.join(name);
        std::fs::write(&path, "keep me\n").expect("write");
        let out = capture(&dir, &["melody", s(&wav)]);
        assert!(!out.status.success(), "{name} was overwritten");
        let err = stderr(&out);
        assert!(
            err.contains(&format!("{name} exists")) && err.contains("never overwrites"),
            "{name}: {err}"
        );
        assert_eq!(read(&path), "keep me\n");
        assert_eq!(files(&dir), [name, "tune.wav"], "nothing else is written");
        std::fs::remove_file(&path).expect("remove");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn melody_writes_its_performance_beside_its_other_outputs_and_never_overwrites_it() {
    let dir = scratch("capture-performance");
    let wav = take(&dir, "tune.wav", &eb_major());
    ok(&capture(&dir, &["melody", s(&wav)]), "capture melody");
    assert_eq!(
        files(&dir),
        [
            "tune.abc",
            "tune.capture.duet",
            "tune.duet",
            "tune.performance.duet.jsonl",
            "tune.wav"
        ]
    );
    let path = dir.join("tune.performance.duet.jsonl");
    let text = read(&path);
    let mut lines = text.lines();
    let first: serde_json::Value = serde_json::from_str(lines.next().expect("a head")).unwrap();
    assert_eq!(first["duet"]["kind"], "performance", "{first}");
    assert_eq!(first["duet"]["version"], "1.0", "{first}");
    assert_eq!(first["duet"]["tool"]["name"], "duet", "{first}");
    assert_eq!(first["head"]["unit"], "absolute-cents", "{first}");
    let hop = first["head"]["hop"].as_u64().expect("hop");
    let mut next = first["head"]["first_frame"].as_u64().expect("first_frame");
    let mut pitches = Vec::new();
    for line in lines {
        let block: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(block["start"].as_u64(), Some(next), "{line}");
        let column = |name: &str| block[name].as_array().expect(name).clone();
        let pitch = column("pitch");
        assert_eq!(pitch.len(), column("voicing").len());
        assert_eq!(pitch.len(), column("level").len());
        next += pitch.len() as u64 * hop;
        pitches.extend(pitch.iter().map(serde_json::Value::as_f64));
    }
    // The take is 16.5 s at 22 050 Hz, a frame every hop.
    let seconds = pitches.len() as f64 * hop as f64 / 22_050.0;
    assert!((16.0..18.0).contains(&seconds), "{seconds}");
    assert!(pitches.iter().any(Option::is_none), "the lead-in is voiced");
    // Its first note is E-flat 4, 6300 cents, held a second from 0.5 s.
    let held: Vec<f64> = pitches[86..120]
        .iter()
        .map(|p| p.expect("voiced"))
        .collect();
    assert!(held.iter().all(|p| (p - 6300.0).abs() < 20.0), "{held:?}");

    // Run again with the other outputs gone: the performance alone is refused, and kept.
    for other in ["tune.abc", "tune.capture.duet", "tune.duet"] {
        std::fs::remove_file(dir.join(other)).expect("remove");
    }
    let out = capture(&dir, &["melody", s(&wav)]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("tune.performance.duet.jsonl exists") && err.contains("never overwrites"),
        "{err}"
    );
    assert_eq!(read(&path), text);
    assert_eq!(files(&dir), ["tune.performance.duet.jsonl", "tune.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn rhythm_writes_a_song_a_grid_and_the_analysis() {
    let dir = scratch("capture-rhythm");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = capture(&dir, &["rhythm", s(&wav), "-o", s(&dir.join("out"))]);
    ok(&out, "capture rhythm");
    song_of(&dir.join("out.duet"));
    grid_of(&dir.join("out.grid.duet"));
    analysis_of(&dir.join("out.capture.duet"));
    assert_eq!(
        files(&dir),
        ["beat.wav", "out.capture.duet", "out.duet", "out.grid.duet"]
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn estimates_are_stated_on_stderr_and_in_both_files() {
    let dir = scratch("capture-stated");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav)]);
    ok(&out, "capture melody");
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("tempo: estimated "))
        .unwrap_or_else(|| panic!("no tempo statement: {err}"));
    // In the song document's envelope, and as a comment in the ABC.
    let stated = document_of(&dir.join("tune.duet")).statements;
    let abc = comments(&read(&dir.join("tune.abc")), "%");
    assert!(stated.iter().any(|l| l == line), "{stated:?}");
    assert!(abc.contains(&format!("% {line}")), "{abc:?}");
    assert!(
        err.lines().any(|l| l.starts_with("meter: estimated ")),
        "{err}"
    );
    assert!(
        err.lines().any(|l| l.starts_with("downbeat: estimated ")),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_given_tempo_is_not_reported_as_an_estimate() {
    let dir = scratch("capture-given");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav), "--tempo", "120"]);
    ok(&out, "capture melody --tempo 120");
    let err = stderr(&out);
    assert!(!err.contains("tempo: estimated"), "{err}");
    assert!(!err.contains("half"), "{err}");
    let doc = document_of(&dir.join("tune.duet"));
    assert!(
        doc.statements.iter().all(|l| !l.starts_with("tempo:")),
        "{:?}",
        doc.statements
    );
    assert_eq!(doc.body.tempo_bpm, 120.0);
    assert!(read(&dir.join("tune.abc")).contains("Q:1/4=120"));
    // Nor are its half and double, in the analysis.
    let json: serde_json::Value = analysis_of(&dir.join("tune.capture.duet"));
    assert!(json["timing"].get("tempo_alternatives").is_none(), "{json}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_given_downbeat_is_not_reported_as_an_estimate() {
    let dir = scratch("capture-given-downbeat");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav), "--downbeat", "0.5"]);
    ok(&out, "capture melody --downbeat 0.5");
    let err = stderr(&out);
    assert!(!err.contains("downbeat:"), "{err}");
    // The tempo is still estimated and stated: only the downbeat was given.
    assert!(err.contains("tempo: estimated"), "{err}");
    for file in ["tune.duet", "tune.abc"] {
        let text = read(&dir.join(file));
        assert!(!text.contains("downbeat: estimated"), "{file}: {text}");
    }
    let json: serde_json::Value = analysis_of(&dir.join("tune.capture.duet"));
    assert!(
        json["timing"]["downbeat_frame"]["given"].is_number(),
        "{json}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_key_is_stated_when_estimated() {
    let dir = scratch("capture-key");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav)]);
    ok(&out, "capture melody");
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("key: "))
        .unwrap_or_else(|| panic!("no key statement: {err}"));
    assert!(
        line.starts_with("key: estimated Eb major (relative C minor)"),
        "{line}"
    );
    assert!(
        document_of(&dir.join("tune.duet"))
            .statements
            .iter()
            .any(|l| l == line)
    );
    assert!(comments(&read(&dir.join("tune.abc")), "%").contains(&format!("% {line}")));

    let out = capture(
        &dir,
        &[
            "melody",
            s(&wav),
            "-o",
            s(&dir.join("given")),
            "--key",
            "Eb",
            "--tempo",
            "120",
            "--meter",
            "4/4",
            "--downbeat",
            "0.5",
        ],
    );
    ok(&out, "capture melody --key Eb");
    let err = stderr(&out);
    assert!(!err.contains("estimated"), "{err}");
    let abc = read(&dir.join("given.abc"));
    assert!(abc.lines().any(|l| l == "K:Eb"), "{abc}");
    assert!(!abc.contains("estimated"), "{abc}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn rhythm_refuses_key() {
    let dir = scratch("capture-rhythm-key");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = capture(&dir, &["rhythm", s(&wav), "--key", "Eb"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--key"), "{}", stderr(&out));
    assert_eq!(files(&dir), ["beat.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn melody_refuses_lane() {
    let dir = scratch("capture-melody-lane");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(&dir, &["melody", s(&wav), "--lane", "hh"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--lane"), "{}", stderr(&out));
    assert_eq!(files(&dir), ["tune.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_compound_meter_states_its_felt_beat() {
    let dir = scratch("capture-six-eight");
    // Eighths at 270 a minute: three to a dotted quarter of 90.
    let accents = [0.5, 0.15, 0.15, 0.3, 0.15, 0.15];
    let wav = take(
        &dir,
        "jig.wav",
        &click_track(270.0, 0.5, 90, &accents, |_| 0.0),
    );
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh"]);
    ok(&out, "capture rhythm");
    let err = stderr(&out);
    assert!(
        err.contains("tempo: estimated 135 bpm (dotted quarter = 90)"),
        "{err}"
    );
    assert!(err.contains("meter: estimated 6/8"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn rhythm_without_a_lane_is_beatboxed() {
    let dir = scratch("capture-beatbox");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let count = |grid: &DrumGrid, lane: &str| -> usize {
        let row = grid.lanes.iter().position(|l| l.name == lane);
        row.map_or(0, |i| grid.row(i).iter().filter(|v| **v > 0).count())
    };

    let out = capture(
        &dir,
        &["rhythm", s(&wav), "--tempo", "120", "--meter", "4/4"],
    );
    ok(&out, "capture rhythm");
    let grid = grid_of(&dir.join("beat.grid.duet"));
    assert_eq!(
        (count(&grid, "kick"), count(&grid, "hh")),
        (8, 8),
        "{grid:?}"
    );

    let out = capture(
        &dir,
        &[
            "rhythm",
            s(&wav),
            "-o",
            s(&dir.join("tapped")),
            "--tempo",
            "120",
            "--meter",
            "4/4",
            "--lane",
            "hat",
        ],
    );
    ok(&out, "capture rhythm --lane hat");
    let grid = grid_of(&dir.join("tapped.grid.duet"));
    assert_eq!(
        (count(&grid, "kick"), count(&grid, "hh")),
        (0, 16),
        "{grid:?}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// The lanes are the configured map's: a config that moves the kick moves where the beatboxed
/// kicks land.
#[test]
fn rhythm_lanes_come_from_the_config() {
    let dir = scratch("capture-config-lanes");
    let user = dir.join("user");
    std::fs::create_dir_all(&user).expect("user dir");
    std::fs::write(user.join("duet.toml"), "[lanes.kick]\nkey = 100\n").expect("config");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = duet_with(
        &dir,
        &user,
        &[
            "capture",
            "rhythm",
            s(&wav),
            "--tempo",
            "120",
            "--meter",
            "4/4",
        ],
        b"",
    );
    ok(&out, "capture rhythm");
    let grid = grid_of(&dir.join("beat.grid.duet"));
    let kick = grid
        .lanes
        .iter()
        .find(|l| l.name == "kick")
        .expect("a kick lane");
    assert_eq!(kick.key, 100, "{grid:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_fractional_tempo_on_a_melody_is_refused_before_analysis() {
    let dir = scratch("capture-fractional");
    // Not audio at all: refused for its tempo, it is never opened.
    let input = dir.join("tune.wav");
    std::fs::write(&input, "not audio\n").expect("write");
    let out = capture(&dir, &["melody", s(&input), "--tempo", "92.5"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("Q:") && err.contains("whole"), "{err}");
    assert!(!err.contains("cannot decode"), "{err}");
    assert_eq!(files(&dir), ["tune.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_fractional_tempo_on_a_rhythm_is_kept() {
    let dir = scratch("capture-fractional-rhythm");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = capture(&dir, &["rhythm", s(&wav), "--tempo", "119.5"]);
    ok(&out, "capture rhythm --tempo 119.5");
    let notes = song_of(&dir.join("beat.duet"));
    assert_eq!(notes.tempo_bpm, 119.5);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_grid_document_keeps_every_velocity_exactly() {
    let dir = scratch("capture-exact-velocity");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = capture(&dir, &["rhythm", s(&wav), "--tempo", "120"]);
    ok(&out, "capture rhythm");
    let grid = grid_of(&dir.join("beat.grid.duet"));
    // A take's velocities fall between the grid text's levels, and are kept there.
    assert!(
        grid.first_off_level().is_some(),
        "every velocity is on a level"
    );
    let mut in_grid: Vec<u8> = grid.vel.data().iter().copied().filter(|v| *v > 0).collect();
    let mut played: Vec<u8> = song_of(&dir.join("beat.duet")).tracks[0]
        .notes
        .iter()
        .map(|n| n.vel)
        .collect();
    in_grid.sort_unstable();
    played.sort_unstable();
    assert_eq!(
        in_grid, played,
        "a velocity moved between the .duet and the grid"
    );
    // So nothing is snapped, and nothing is stated as snapped.
    assert!(!stderr(&out).contains("snapped"), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// The grid document is a document like any other: `duet drums render` reads it by its
/// envelope, with no `--from`, and it states the estimated tempo in its envelope.
#[test]
fn the_grid_document_renders_and_states_its_estimates() {
    let dir = scratch("capture-grid-render");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    let out = capture(&dir, &["rhythm", s(&wav)]);
    ok(&out, "capture rhythm");
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("tempo: estimated "))
        .unwrap_or_else(|| panic!("no tempo statement: {err}"));
    let path = dir.join("beat.grid.duet");
    let doc = grid_document_of(&path);
    assert_eq!(doc.envelope().kind, "drum-grid");
    assert!(
        doc.statements.iter().any(|l| l == line),
        "{:?}",
        doc.statements
    );
    let rendered = dir.join("rendered.duet");
    ok(
        &duet(&dir, &["drums", "render", s(&path), s(&rendered)]),
        "drums render beat.grid.duet",
    );
    let song = song_of(&rendered);
    let hits = doc.body.vel.data().iter().filter(|v| **v > 0).count();
    assert_eq!(song.tracks[0].notes.len(), hits, "{song:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Every file a rhythm capture writes is refused, rather than overwritten, when it exists.
#[test]
fn rhythm_refuses_to_overwrite_each_of_its_outputs() {
    let dir = scratch("capture-rhythm-refuse-each");
    let wav = take(&dir, "beat.wav", &kick_and_hat());
    for name in ["beat.duet", "beat.grid.duet", "beat.capture.duet"] {
        let path = dir.join(name);
        std::fs::write(&path, "keep me\n").expect("write");
        let out = capture(&dir, &["rhythm", s(&wav)]);
        assert!(!out.status.success(), "{name} was overwritten");
        let err = stderr(&out);
        assert!(
            err.contains(&format!("{name} exists")) && err.contains("never overwrites"),
            "{name}: {err}"
        );
        assert_eq!(read(&path), "keep me\n");
        let mut want = vec![name, "beat.wav"];
        want.sort_unstable();
        assert_eq!(files(&dir), want, "nothing else is written");
        std::fs::remove_file(&path).expect("remove");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_notes_file_is_not_quantized() {
    let dir = scratch("capture-unquantized");
    // Every other click 15 ms late: about a quarter of a sixteenth at 120.
    let wav = take(
        &dir,
        "loose.wav",
        &click_track(
            120.0,
            0.5,
            32,
            &[0.5],
            |n| if n % 2 == 1 { 0.015 } else { 0.0 },
        ),
    );
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh", "--tempo", "120"]);
    ok(&out, "capture rhythm");
    let notes = song_of(&dir.join("loose.duet"));
    let off = ticks(&notes).into_iter().filter(|t| t % 120 != 0).count();
    assert!(off > 0, "every onset is on the 1/16 grid: {notes:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn an_existing_output_is_refused() {
    let dir = scratch("capture-exists");
    let wav = take(&dir, "tune.wav", &eb_major());
    std::fs::write(dir.join("tune.abc"), "keep me\n").expect("write");
    let out = capture(&dir, &["melody", s(&wav)]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("tune.abc"), "{}", stderr(&out));
    assert_eq!(read(&dir.join("tune.abc")), "keep me\n");
    assert_eq!(
        files(&dir),
        ["tune.abc", "tune.wav"],
        "nothing else is written"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_input_is_refused_as_an_output() {
    let dir = scratch("capture-input");
    // A take whose name is what its song document would be.
    let input = take(&dir, "tune.duet", &eb_major());
    let out = capture(&dir, &["melody", s(&input)]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("is the input"), "{}", stderr(&out));
    assert_eq!(files(&dir), ["tune.duet"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn the_analysis_file_is_written_and_reads_back() {
    let dir = scratch("capture-analysis");
    let wav = take(
        &dir,
        "beat.wav",
        &click_track(120.0, 0.5, 32, &[0.5], |_| 0.0),
    );
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh"]);
    ok(&out, "capture rhythm");
    let json: serde_json::Value = analysis_of(&dir.join("beat.capture.duet"));
    for field in ["events", "beats", "offsets"] {
        let list = json[field]
            .as_array()
            .unwrap_or_else(|| panic!("{field}: {json}"));
        assert!(!list.is_empty(), "{field} is empty");
    }
    assert!(
        json["timing"]["tempo_bpm"]["estimated"].is_object(),
        "{json}"
    );
    assert!(json["timing"]["beat_period_frames"].is_number(), "{json}");
    assert_eq!(json["rate"], 22_050);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_drifting_take_shows_its_slide_in_the_analysis() {
    let dir = scratch("capture-drift");
    let wav = take(&dir, "drift.wav", &sliding_clicks(100.0, 106.0, 20.0));
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh"]);
    ok(&out, "capture rhythm");
    let json: serde_json::Value = analysis_of(&dir.join("drift.capture.duet"));
    let bpms: Vec<f64> = json["beats"]
        .as_array()
        .expect("beats")
        .iter()
        .map(|b| b["bpm"].as_f64().expect("bpm"))
        .collect();
    assert!(bpms.len() >= 30, "{bpms:?}");
    assert!((bpms[0] - 100.0).abs() <= 1.5, "{bpms:?}");
    assert!((bpms[bpms.len() - 1] - 106.0).abs() <= 1.5, "{bpms:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn silence_is_refused() {
    let dir = scratch("capture-silence");
    let wav = take(&dir, "quiet.wav", &audio::silence(3.0));
    for (kind, what) in [("melody", "no notes"), ("rhythm", "no hits")] {
        let out = capture(&dir, &[kind, s(&wav)]);
        assert!(!out.status.success(), "{kind}");
        let err = stderr(&out);
        assert!(err.contains(what), "{kind}: {err}");
        assert!(
            err.contains("quiet.wav"),
            "{kind}: the file is not named: {err}"
        );
    }
    assert_eq!(files(&dir), ["quiet.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn too_few_onsets_ask_for_a_tempo() {
    let dir = scratch("capture-few");
    let wav = take(
        &dir,
        "two.wav",
        &click_track(120.0, 0.5, 2, &[0.5], |_| 0.0),
    );
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--tempo"), "{}", stderr(&out));
    assert!(stderr(&out).contains("two.wav"), "{}", stderr(&out));
    assert_eq!(files(&dir), ["two.wav"]);
    let out = capture(&dir, &["rhythm", s(&wav), "--lane", "hh", "--tempo", "120"]);
    ok(&out, "with --tempo");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A note a tick before a given downbeat makes a pickup that quantizing leaves empty: the
/// score drops it and says so, and the `.duet` keeps it, at the note's true time.
#[test]
fn an_empty_lead_in_is_removed_from_the_score_only() {
    let dir = scratch("capture-lead-in");
    let wav = take(&dir, "tune.wav", &eb_major());
    let given = ["--tempo", "120", "--meter", "4/4", "--key", "Eb"];
    let first = dir.join("first");
    let mut args = vec!["melody", s(&wav), "-o", s(&first)];
    args.extend(given);
    ok(&capture(&dir, &args), "first capture");
    let json: serde_json::Value = analysis_of(&dir.join("first.capture.duet"));
    let start = json["events"]
        .as_array()
        .expect("events")
        .iter()
        .find_map(|e| (e["type"] == "note").then(|| e["start"].as_u64().expect("start")))
        .expect("a note");
    // A tick at 120 a minute and 480 a quarter is 22.97 frames: 23 after the note is a tick.
    let downbeat = format!("{:.6}", (start + 23) as f64 / f64::from(audio::RATE));

    let mut args = vec!["melody", s(&wav), "--downbeat", downbeat.as_str()];
    args.extend(given);
    let out = capture(&dir, &args);
    ok(
        &out,
        "capture with the downbeat a tick after the first note",
    );
    let err = stderr(&out);
    assert!(
        err.lines().any(|l| l.contains("removed from the score")),
        "{err}"
    );

    let notes_text = read(&dir.join("tune.duet"));
    let notes = song_of(&dir.join("tune.duet"));
    assert_eq!(notes.time_sig.to_string(), "1/4", "{notes_text}");
    assert_eq!(notes.time_changes.len(), 1, "{notes_text}");
    assert_eq!(ticks(&notes)[0], 479, "{notes_text}");

    let abc = read(&dir.join("tune.abc"));
    assert!(
        comments(&abc, "%")
            .iter()
            .any(|l| l.contains("removed from the score")),
        "{abc}"
    );
    let rendered = dir.join("rendered.duet");
    ok(
        &duet(
            &dir,
            &["abc", "render", s(&dir.join("tune.abc")), s(&rendered)],
        ),
        "abc render",
    );
    let score = song_of(&rendered);
    assert_eq!(score.time_sig.to_string(), "4/4", "{abc}");
    assert!(score.time_changes.is_empty(), "{abc}");
    assert_eq!(ticks(&score)[0], 0, "{abc}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A given tempo and meter are used exactly, on a take whose estimate would differ (120 and
/// 4/4), and neither is stated as an estimate.
#[test]
fn a_given_tempo_and_meter_override_the_estimate() {
    let dir = scratch("capture-given-meter");
    let wav = take(&dir, "tune.wav", &eb_major());
    let out = capture(
        &dir,
        &["melody", s(&wav), "--tempo", "97", "--meter", "3/4"],
    );
    ok(&out, "capture melody --tempo 97 --meter 3/4");
    let err = stderr(&out);
    assert!(!err.contains("tempo: estimated"), "{err}");
    assert!(!err.contains("meter: estimated"), "{err}");
    let abc = read(&dir.join("tune.abc"));
    assert!(abc.lines().any(|l| l == "M:3/4"), "{abc}");
    assert!(abc.lines().any(|l| l == "Q:1/4=97"), "{abc}");
    let notes = song_of(&dir.join("tune.duet"));
    assert_eq!(notes.tempo_bpm, 97.0);
    let meters: Vec<String> = std::iter::once(notes.time_sig)
        .chain(notes.time_changes.iter().map(|c| c.time_sig))
        .map(|t| t.to_string())
        .collect();
    assert_eq!(meters.last().map(String::as_str), Some("3/4"), "{meters:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_tempo_that_is_not_a_number_is_refused() {
    let dir = scratch("capture-nan");
    let wav = take(&dir, "tune.wav", &eb_major());
    for tempo in ["NaN", "inf", "-90"] {
        let out = capture(&dir, &["melody", s(&wav), &format!("--tempo={tempo}")]);
        assert!(!out.status.success(), "{tempo}");
        assert!(
            stderr(&out).contains("positive number"),
            "{tempo}: {}",
            stderr(&out)
        );
    }
    assert_eq!(files(&dir), ["tune.wav"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Audio that cannot be decoded is refused by the binary, naming the file, with nothing
/// written.
#[test]
fn undecodable_audio_is_refused_by_name() {
    let dir = scratch("capture-undecodable");
    let text = dir.join("words.mp3");
    std::fs::write(&text, "this is words, not music\n".repeat(200)).expect("write");
    let he_aac = root().join("crates/duet-capture/tests/fixtures/he-aac.m4a");
    for (input, stem, codec) in [(&text, "words", None), (&he_aac, "he-aac", Some("HE-AAC"))] {
        let out = capture(&dir, &["melody", s(input), "-o", s(&dir.join(stem))]);
        assert!(!out.status.success(), "{stem}");
        let err = stderr(&out);
        let name = input.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(err.contains("cannot decode") && err.contains(name), "{err}");
        if let Some(codec) = codec {
            assert!(err.contains(codec), "{err}");
        }
    }
    assert_eq!(files(&dir), ["words.mp3"]);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// The files a capture of `stem` from `input` writes, with the input: every one of them.
fn all_written(dir: &Path, input: &str, stem: &str, score: &str) {
    let mut want = vec![
        input.to_owned(),
        format!("{stem}.capture.duet"),
        format!("{stem}.duet"),
        format!("{stem}{score}"),
    ];
    // A melody's performance; a rhythm take has none.
    if score == ".abc" {
        want.push(format!("{stem}.performance.duet.jsonl"));
    }
    want.sort();
    assert_eq!(files(dir), want);
}

/// A phrase at 120 a minute from half a second in, with a run of four 80 ms notes (G F E D)
/// in its second bar: quicker than a sixteenth, so two of them share a sixteenth's step.
fn a_quick_run() -> (Vec<f32>, Vec<u8>) {
    let q = 0.5;
    let notes = [
        (60, q),
        (62, q),
        (64, q),
        (65, q),
        (67, 0.08),
        (65, 0.08),
        (64, 0.08),
        (62, 0.08),
        (60, q),
        (64, q),
        (67, q),
        (72, q),
        (60, q),
        (62, q),
        (64, 2.0 * q),
    ];
    let midis = notes.iter().map(|n| n.0).collect();
    (audio::notes(0.5, &notes), midis)
}

/// Snare-band bursts, 50 ms long, at each time.
fn snares(secs: f64, times: &[f64]) -> Vec<f32> {
    let at: Vec<(f64, f64)> = times.iter().map(|&t| (t, 0.05)).collect();
    audio::bursts(secs, &at, 1_000.0, 4_000.0)
}

/// The pitches of a song's notes, in order.
fn midis(song: &Song) -> Vec<u8> {
    song.tracks
        .iter()
        .flat_map(|t| t.notes.iter().map(|n| n.pitch.midi()))
        .collect()
}

/// The events of one type (`note` or `hit`) in a `.capture.duet`.
fn analysed(path: &Path, kind: &str) -> Vec<serde_json::Value> {
    analysis_of(path)["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["type"] == kind)
        .cloned()
        .collect()
}

/// A run quicker than the grid writes all three files: the score's grid is refined until no
/// two notes share a step, the refinement is stated, and the `.duet` keeps every note.
#[test]
fn a_quick_run_refines_the_grid_and_writes_everything() {
    let dir = scratch("capture-run");
    let (samples, played) = a_quick_run();
    let wav = take(&dir, "run.wav", &samples);
    let out = capture(&dir, &["melody", s(&wav), "--tempo", "120"]);
    ok(&out, "capture melody --tempo 120");
    all_written(&dir, "run.wav", "run", ".abc");
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("grid: 1/16 refined to "))
        .unwrap_or_else(|| panic!("no refinement stated: {err}"));
    assert!(
        comments(&read(&dir.join("run.abc")), "%").contains(&format!("% {line}")),
        "the score does not state its grid"
    );
    let notes = song_of(&dir.join("run.duet"));
    let found: Vec<u8> = analysed(&dir.join("run.capture.duet"), "note")
        .iter()
        .map(|e| e["midi"].as_u64().expect("midi") as u8)
        .collect();
    assert_eq!(midis(&notes), found, "the .duet lost a note");
    let run = &played[4..8];
    assert!(
        found.windows(4).any(|w| w == run),
        "the run {run:?} was not heard: {found:?}"
    );
    ok(
        &duet(&dir, &["abc", "check", s(&dir.join("run.abc"))]),
        "abc check",
    );
    // A grid given is refined the same way, and the statement says it was the one given.
    let given = dir.join("given");
    let out = capture(
        &dir,
        &[
            "melody",
            s(&wav),
            "--tempo",
            "120",
            "--grid",
            "1/8",
            "-o",
            s(&given),
        ],
    );
    ok(&out, "capture melody --grid 1/8");
    let err = stderr(&out);
    assert!(err.contains("grid: --grid 1/8 refined to "), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A flam, two snare hits 60 ms apart, writes all three files: the grid is refined until the
/// two hits have a cell each, and it is stated.
#[test]
fn a_flam_refines_the_grid_and_writes_everything() {
    let dir = scratch("capture-flam");
    // Beats at 120 a minute; on beat 6 a grace hit 30 ms before it and the hit 30 ms after.
    let mut times: Vec<f64> = (0..8).map(|n| 0.5 + 0.5 * f64::from(n)).collect();
    times[5] -= 0.03;
    times.insert(6, times[5] + 0.06);
    let wav = take(&dir, "flam.wav", &snares(6.0, &times));
    let out = capture(
        &dir,
        &["rhythm", s(&wav), "--lane", "snare", "--tempo", "120"],
    );
    ok(&out, "capture rhythm --lane snare --tempo 120");
    all_written(&dir, "flam.wav", "flam", ".grid.duet");
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("grid: 1/16 refined to "))
        .unwrap_or_else(|| panic!("no refinement stated: {err}"));
    assert!(
        grid_document_of(&dir.join("flam.grid.duet"))
            .statements
            .iter()
            .any(|l| l == line),
        "the score does not state its grid"
    );
    let notes = song_of(&dir.join("flam.duet"));
    assert_eq!(
        analysed(&dir.join("flam.capture.duet"), "hit").len(),
        times.len(),
        "a hit was not heard"
    );
    assert_eq!(
        notes.tracks[0].notes.len(),
        times.len(),
        "the .duet lost a hit"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Two hits closer together than a 1/64 step, at 20 a minute, where a 1/64 is 187.5 ms: no
/// grid parts them, so they are merged into one in the score, the merge is stated, and the
/// `.duet` keeps both.
#[test]
fn hits_no_grid_parts_are_merged_and_stated() {
    let dir = scratch("capture-merge");
    // The downbeat at 0.5 s, and the next beat 3 s on, with the flam's two hits 30 ms either
    // side of it.
    let times = [0.5, 3.47, 3.53];
    let wav = take(&dir, "flam.wav", &snares(5.0, &times));
    let out = capture(
        &dir,
        &[
            "rhythm",
            s(&wav),
            "--lane",
            "snare",
            "--tempo",
            "20",
            "--meter",
            "4/4",
            "--downbeat",
            "0.5",
        ],
    );
    ok(&out, "capture rhythm --tempo 20");
    all_written(&dir, "flam.wav", "flam", ".grid.duet");
    let err = stderr(&out);
    assert!(err.contains("grid: 1/16 refined to 1/64"), "{err}");
    let line = err
        .lines()
        .find(|l| l.starts_with("merged: "))
        .unwrap_or_else(|| panic!("no merge stated: {err}"));
    assert!(line.contains("snare"), "{line}");
    let doc = grid_document_of(&dir.join("flam.grid.duet"));
    assert!(
        doc.statements.iter().any(|l| l == line),
        "{:?}",
        doc.statements
    );
    let hits = doc.body.vel.data().iter().filter(|v| **v > 0).count();
    assert_eq!(hits, 2, "{:?}", doc.body);
    let notes = song_of(&dir.join("flam.duet"));
    assert_eq!(notes.tracks[0].notes.len(), 3, "the .duet lost a hit");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
