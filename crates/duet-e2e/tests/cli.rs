//! The `duet` binary end to end: every conversion writes the same music it wrote before
//! songs could carry an end, now stating where it ends, and a drum grid that ends in
//! empty bars comes back whole through MIDI with no bar count.

use std::path::Path;

use duet_e2e::{root, scratch};

/// Runs duet with no config at all: an empty user config directory, and a working
/// directory with no `.duet` above it.
fn duet(args: &[&Path]) {
    let none = scratch("no-config");
    let out = duet_in(&none, &none.join("user"), args);
    assert!(
        out.status.success(),
        "duet {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs duet in `cwd`, with `user` as its user config directory.
fn duet_in(cwd: &Path, user: &Path, args: &[&Path]) -> std::process::Output {
    duet_e2e::duet_with(cwd, user, args, b"")
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

/// The golden files in `golden/` were written by duet at 145077f, before a song
/// could name an end, from `samples/sketch.txt` and `samples/groove.drums` (which now
/// state their ends, `end 5.1.0` and `end 3.1.0`, the defaults). Every file written now
/// holds exactly the same music, every note, control and conductor event at the same
/// tick, and differs only by stating its end: an `end` line in text, an `end` field in
/// a song's JSON and TOML, and every MIDI track ending there.
#[test]
fn every_output_holds_the_same_music_and_states_its_end() {
    let dir = scratch("golden");
    let golden = duet_e2e::golden_dir();
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

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
    std::fs::write(path, text).expect("write");
}

/// Witness for config precedence: the user's `duet.toml`, then the user's
/// `duet.local.toml`, then the project's `.duet/duet.toml` (found from a directory below
/// it), then the project's `.duet/duet.local.toml`, each over the one before, lane by
/// lane and field by field. `duet config show` names where each value came from, and a
/// configured end default changes what is written, the same way every time.
#[test]
fn config_files_merge_in_order_and_the_end_default_is_stable() {
    let dir = scratch("precedence");
    let user = dir.join("user");
    let project = dir.join("project");
    let below = project.join("songs/sketches");
    std::fs::create_dir_all(&below).expect("dirs");
    write(
        &user.join("duet.toml"),
        "[song]\nend = { round-up = 4 }\n\n[lanes.shaker-2]\nkey = 82\naliases = [\"sh2\"]\n",
    );
    write(
        &user.join("duet.local.toml"),
        "[lanes.shaker-2]\nkey = 83\n",
    );
    write(
        &project.join(".duet/duet.toml"),
        "[song]\nend = { round-up = 2 }\n\n[lanes.kick]\naliases = [\"boom\"]\n",
    );
    write(
        &project.join(".duet/duet.local.toml"),
        "[lanes.shaker-2]\ngroup = \"percussion\"\n",
    );
    let p = |s: &'static str| Path::new(s);
    let run = |cwd: &Path, args: &[&Path]| {
        let out = duet_in(cwd, &user, args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    };

    let show = run(&below, &[p("config"), p("show")]);
    let from = |f: &str| f.to_owned();
    let user_toml = from(&user.join("duet.toml").display().to_string());
    let user_local = from(&user.join("duet.local.toml").display().to_string());
    let proj_toml = from(&project.join(".duet/duet.toml").display().to_string());
    let proj_local = from(&project.join(".duet/duet.local.toml").display().to_string());
    for want in [
        format!("end = {{ round-up = 2 }}  # {proj_toml}\n"),
        format!("[lanes.shaker-2]\nkey = 83  # {user_local}\n"),
        format!("group = \"percussion\"  # {proj_local}\naliases = [\"sh2\"]  # {user_toml}\n"),
        format!(
            "[lanes.kick]\nkey = 36  # built-in\ngroup = \"kick\"  # built-in\naliases = [\"boom\"]  # {proj_toml}\n"
        ),
    ] {
        assert!(
            show.contains(&want),
            "config show should hold\n{want}\n{show}"
        );
    }
    let paths = run(&below, &[p("config"), p("path")]);
    assert!(
        paths.contains(&format!("{}  (found)", proj_local)),
        "{paths}"
    );

    // The end default: round-up 2 in the project, 4 for the user alone, the built-in
    // with no config; the same bytes every time.
    let take = dir.join("take.txt");
    write(&take, "[track \"P\" ch1]\n1.1.0 C4 1 80\n");
    let convert = |cwd: &Path, user_dir: &Path| {
        let out = duet_in(cwd, user_dir, &[p("convert"), &take, p("-")]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    };
    let in_project = convert(&below, &user);
    assert!(in_project.contains("\nend 3.1.0\n"), "{in_project}");
    assert_eq!(convert(&below, &user), in_project, "stable");
    assert!(
        convert(&dir, &user).contains("\nend 5.1.0\n"),
        "the user's round-up 4"
    );
    assert!(
        convert(&dir, &dir.join("none")).contains("\nend 2.1.0\n"),
        "built-in"
    );
    let midi = |cwd: &Path| {
        let out = duet_in(
            cwd,
            &user,
            &[p("convert"), &take, p("--to"), p("midi"), p("-")],
        );
        assert!(out.status.success());
        out.stdout
    };
    assert_eq!(midi(&below), midi(&below), "the same MIDI every time");

    // Config lanes read without lane lines, and a .drums file written under the config
    // reads the same with none.
    let hand = dir.join("hand.drums");
    write(
        &hand,
        "grid 1/8\n\n[bars 1]\nsh2  | x . x . x . x . |\nboom | X . . . X . . . |\n",
    );
    let written = dir.join("written.drums");
    let out = duet_in(&below, &user, &[p("drums"), p("snap"), &hand, &written]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&written).expect("written");
    assert!(text.contains("\nmap gm\nlane shaker-2 83\n"), "{text}");
    assert!(
        text.contains("\nkick     | X"),
        "the lane's own name: {text}"
    );
    let elsewhere = dir.join("elsewhere.drums");
    let out = duet_in(
        &dir,
        &dir.join("none"),
        &[p("drums"), p("snap"), &written, &elsewhere],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&elsewhere).expect("read"),
        text,
        "self-contained"
    );
    let out = duet_in(
        &dir,
        &dir.join("none"),
        &[p("drums"), p("snap"), &hand, &elsewhere],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("'sh2' is not a lane"),
        "{err}"
    );

    // Reading a drum track uses the config's lanes, and runs to its end default: a
    // one-bar take is two bars under round-up 2.
    let kit = dir.join("kit.txt");
    write(
        &kit,
        "[track \"Kit\" ch10]\n1.1.0 B5 0.25 90\n1.3.0 C2 0.25 90\n",
    );
    let grid = dir.join("kit.drums");
    let args = [p("drums"), p("read"), &kit, &grid, p("--grid"), p("1/8")];
    let out = duet_in(&below, &user, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&grid).expect("grid");
    assert!(
        text.contains("\nend 3.1.0\n") && text.contains("[bars 1-2]"),
        "{text}"
    );
    assert!(
        text.contains("\nshaker-2 | x"),
        "key 83 is the config's shaker-2: {text}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A config file that breaks a rule is refused, naming its path and the rule.
#[test]
fn a_config_file_that_breaks_a_rule_is_refused_naming_it() {
    let dir = scratch("refusals");
    let user = dir.join("user");
    let file = user.join("duet.toml");
    let cases = [
        ("[song\nend = 4\n", "TOML parse error"),
        ("[song]\nlength = 4\n", "unknown field `length`"),
        (
            "[song]\nend = { round-up = 0 }\n",
            "round-up is a whole number of bars, 1 or more, not 0",
        ),
        (
            "[song]\nend = \"bar\"\n",
            "[song] end is \"last-bar\" or { round-up = <bars> }",
        ),
        (
            "[lanes.kick]\nkey = 38\n",
            "lanes kick and snare both play key 38",
        ),
        (
            "[lanes.kick]\ncolour = 1\n",
            "[lanes.kick]: unknown field `colour`",
        ),
        (
            "[lanes.bell-2]\ngroup = \"other\"\n",
            "lane bell-2 is not in the gm map, so it needs a key",
        ),
        ("[drums]\nchannel = 9\n", "unknown field `drums`"),
    ];
    let p = |s: &'static str| Path::new(s);
    for (text, want) in cases {
        write(&file, text);
        let out = duet_in(&dir, &user, &[p("config"), p("show")]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{text} should be refused");
        assert!(
            err.contains(&file.display().to_string()),
            "names the file: {err}"
        );
        assert!(err.contains(want), "{err}\nshould say: {want}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `map gm` reads the same on every machine: under a config that moves gm lanes, a
/// .drums file written with no config reads, and is written back, the same as with no
/// config, and the move is said on standard error and in `config show`. Reading a MIDI
/// drum track still uses the moved keys: there the config describes the user's kit.
#[test]
fn a_config_that_moves_a_gm_lane_never_changes_how_drums_text_reads() {
    let dir = scratch("gm-stays");
    let user = dir.join("user");
    write(
        &user.join("duet.toml"),
        "[lanes.kick]\nkey = 100\naliases = [\"boom\"]\n\n[lanes.snare]\nkey = 101\n",
    );
    let p = |s: &'static str| Path::new(s);
    let beat = dir.join("beat.drums");
    write(
        &beat,
        "grid 1/8\n\n[bars 1]\nsnare | . . X . . . X . |\nkick  | X . . . X . . . |\n",
    );
    let snap = |user_dir: &Path, input: &Path, out: &Path| {
        let o = duet_in(&dir, user_dir, &[p("drums"), p("snap"), input, out]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        (
            std::fs::read_to_string(out).expect("written"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (plain, _) = snap(&dir.join("none"), &beat, &dir.join("plain.drums"));
    let (moved, err) = snap(&user, &beat, &dir.join("moved.drums"));
    assert_eq!(
        moved, plain,
        "the config does not move kick or snare in text"
    );
    assert!(!moved.contains("\nlane "), "{moved}");
    for want in ["lane kick: key 100 does not apply", "lane snare: key 101"] {
        assert!(err.contains(want), "said on stderr: {err}");
    }

    // The alias the config adds still reads, as gm's kick.
    let hand = dir.join("hand.drums");
    write(
        &hand,
        "grid 1/8\n\n[bars 1]\nsnare | . . X . . . X . |\nboom  | X . . . X . . . |\n",
    );
    assert_eq!(snap(&user, &hand, &dir.join("hand-out.drums")).0, plain);

    let show = duet_in(&dir, &user, &[p("config"), p("show")]);
    let show = String::from_utf8_lossy(&show.stdout);
    assert!(
        show.contains("# reading .drums text, map gm stays General MIDI:\n# lane kick: key 100"),
        "{show}"
    );

    // A MIDI drum track on key 100 reads as the config's kick.
    let kit = dir.join("kit.txt");
    write(&kit, "[track \"Kit\" ch10]\n1.1.0 E7 0.25 90\n");
    let grid = dir.join("kit.drums");
    let args = [p("drums"), p("read"), &kit, &grid, p("--grid"), p("1/8")];
    let out = duet_in(&dir, &user, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&grid).expect("grid");
    assert!(
        text.contains("\nlane kick 100\n") && text.contains("\nkick | x"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn abc_tunes_render_convert_and_are_refused_by_name() {
    let dir = scratch("abc");
    let none = scratch("abc-no-config");
    let user = none.join("user");
    let run = |args: &[&Path]| duet_in(&none, &user, args);
    let tunes = root().join("crates/duet-abc/corpus/several-tunes.abc");

    let out = run(&[Path::new("abc"), Path::new("check"), &tunes]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("X:2 The second of two tunes: read"),
        "{listed}"
    );

    // A file of two tunes needs --tune; the second is in 3/4.
    let txt = dir.join("second.txt");
    let out = run(&[Path::new("abc"), Path::new("render"), &tunes, &txt]);
    assert!(!out.status.success(), "two tunes and no --tune is refused");
    let out = run(&[
        Path::new("abc"),
        Path::new("render"),
        &tunes,
        &txt,
        Path::new("--tune"),
        Path::new("2"),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&txt).expect("rendered");
    assert!(
        text.contains("time 3/4") && text.contains("key G major"),
        "{text}"
    );

    // convert goes both ways through .abc.
    let abc = dir.join("second.abc");
    duet(&[Path::new("convert"), &txt, &abc]);
    let written = std::fs::read_to_string(&abc).expect("written");
    assert!(
        written.starts_with("X:1\n") && written.contains("M:3/4"),
        "{written}"
    );
    let back = dir.join("back.txt");
    duet(&[Path::new("convert"), &abc, &back]);
    assert_eq!(std::fs::read_to_string(&back).expect("back"), text);

    // A construct outside the subset is refused by name, with its line.
    let grace = dir.join("grace.abc");
    std::fs::write(&grace, "X:1\nT:g\nM:4/4\nL:1/8\nK:C\nC2 {g}D2 E4 |]\n").expect("write");
    let out = run(&[Path::new("abc"), Path::new("check"), &grace]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("line 6, col 4") && said.contains("grace notes"),
        "{said}"
    );
}
