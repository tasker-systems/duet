//! `duet convert` and drum grids: a grid is read as `duet drums render` reads it and
//! written as `duet drums read` writes it, byte for byte and refusal for refusal; grid
//! flags are never guessed and never silently ignored; and a grid survives a round trip
//! through MIDI by `convert` alone.

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, duet_with, golden, ok, root, s, scratch};

/// The grids to read: the sample, the goldens, and the same grid in its TOML form.
fn grids(dir: &Path) -> Vec<PathBuf> {
    let toml = dir.join("groove.drums.toml");
    let out = duet(
        dir,
        &[
            "drums",
            "read",
            s(&golden("groove.mid")),
            s(&toml),
            "--grid",
            "1/16",
        ],
    );
    ok(&out, "groove.drums.toml");
    vec![
        root().join("samples/groove.drums"),
        golden("groove.drums"),
        golden("groove.drums.json"),
        toml,
    ]
}

/// Equivalence, reading: for every sample and golden grid and every output format,
/// `convert` writes the same bytes, and says the same on standard error, as
/// `drums render`.
#[test]
fn convert_reads_every_grid_as_drums_render_does() {
    let dir = scratch("read");
    let mut compared = 0;
    for grid in grids(&dir) {
        let stem = grid.file_name().and_then(|n| n.to_str()).expect("a name");
        for ext in ["mid", "notes", "json", "toml"] {
            let a = dir.join(format!("{stem}.convert.{ext}"));
            let b = dir.join(format!("{stem}.render.{ext}"));
            let conv = duet(&dir, &["convert", s(&grid), s(&a)]);
            let rend = duet(&dir, &["drums", "render", s(&grid), s(&b)]);
            ok(&conv, &format!("convert {stem} .{ext}"));
            ok(&rend, &format!("render {stem} .{ext}"));
            assert_eq!(conv.stderr, rend.stderr, "{stem} .{ext}: stderr");
            assert!(
                std::fs::read(&a).expect("a") == std::fs::read(&b).expect("b"),
                "{stem} to .{ext}: convert and drums render differ"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 16);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `.drums` text read by `convert` keeps the `map gm` rule: a config that moves gm lanes
/// does not move them in text, the move is said on standard error as `drums render` says
/// it, and the alias the config adds still reads.
#[test]
fn convert_reads_drums_text_under_the_map_gm_rule() {
    let dir = scratch("gm");
    let user = dir.join("user");
    std::fs::create_dir_all(&user).expect("user dir");
    std::fs::write(
        user.join("duet.toml"),
        "[lanes.kick]\nkey = 100\naliases = [\"boom\"]\n\n[lanes.snare]\nkey = 101\n",
    )
    .expect("config");
    let beat = dir.join("beat.drums");
    std::fs::write(
        &beat,
        "grid 1/8\n\n[bars 1]\nsnare | . . X . . . X . |\nboom  | X . . . X . . . |\n",
    )
    .expect("beat");
    let plain = dir.join("plain.mid");
    let plain_beat = dir.join("plain.drums");
    std::fs::write(
        &plain_beat,
        "grid 1/8\n\n[bars 1]\nsnare | . . X . . . X . |\nkick  | X . . . X . . . |\n",
    )
    .expect("plain");
    ok(
        &duet(&dir, &["convert", s(&plain_beat), s(&plain)]),
        "plain",
    );

    let (a, b) = (dir.join("a.mid"), dir.join("b.mid"));
    let conv = duet_with(&dir, &user, &["convert", s(&beat), s(&a)], b"");
    let rend = duet_with(&dir, &user, &["drums", "render", s(&beat), s(&b)], b"");
    ok(&conv, "convert");
    ok(&rend, "render");
    let err = String::from_utf8_lossy(&conv.stderr);
    assert!(err.contains("lane kick: key 100 does not apply"), "{err}");
    assert_eq!(conv.stderr, rend.stderr, "the same notices");
    let bytes = std::fs::read(&a).expect("a");
    assert!(
        bytes == std::fs::read(&b).expect("b"),
        "convert and render agree"
    );
    assert!(
        bytes == std::fs::read(&plain).expect("plain"),
        "kick and snare stay General MIDI"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Runs `convert` and `drums read` with the same arguments and checks they agree: the
/// same exit, the same standard error, and the same bytes written.
fn same_as_drums_read(dir: &Path, song: &Path, out_name: &str, flags: &[&str]) -> Output {
    let a = dir.join(format!("convert-{out_name}"));
    let b = dir.join(format!("read-{out_name}"));
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    let mut conv_args = vec!["convert", s(song), s(&a)];
    conv_args.extend(flags);
    let mut read_args = vec!["drums", "read", s(song), s(&b)];
    read_args.extend(flags);
    let conv = duet(dir, &conv_args);
    let read = duet(dir, &read_args);
    let what = format!("{} to {out_name} {flags:?}", song.display());
    assert_eq!(conv.status.code(), read.status.code(), "{what}: exit");
    // The two name their own output files; nothing else may differ.
    let conv_err = String::from_utf8_lossy(&conv.stderr).replace(s(&a), "OUT");
    let read_err = String::from_utf8_lossy(&read.stderr).replace(s(&b), "OUT");
    assert_eq!(conv_err, read_err, "{what}: stderr");
    if read.status.success() {
        assert!(
            std::fs::read(&a).expect("a") == std::fs::read(&b).expect("b"),
            "{what}: convert and drums read differ"
        );
    } else {
        assert!(!a.exists(), "{what}: a refusal writes nothing");
    }
    conv
}

/// Equivalence, writing: for the golden songs with a drum track, and a song with other
/// tracks and controls, `convert … --grid` writes the same bytes and the same standard
/// error as `drums read`, in every grid form and with every flag.
#[test]
fn convert_writes_every_grid_as_drums_read_does() {
    let dir = scratch("write");
    let band = dir.join("band.notes");
    std::fs::write(
        &band,
        "[track \"Keys\" ch1]\n1.1.0 C4 1 80\n\n[track \"Kit\" ch10]\n1.1.0 C2 0.25 120\n\
         1.1.0 F#2 0.25 90\n1.2.0 F#2 0.25 90\n1.3.0 D2 0.25 120\n1.3.0 F#2 0.25 90\n\
         1.4.0 F#2 0.25 40\n1.1.0 cc4 90\n",
    )
    .expect("band");
    let songs = [
        golden("groove.mid"),
        golden("groove.notes"),
        golden("groove.json"),
        band,
    ];
    let flag_sets: [&[&str]; 4] = [
        &["--grid", "1/16"],
        &["--grid", "1/16", "--bars", "4"],
        &["--grid", "1/16", "--snap-levels"],
        &["--grid", "1/16", "--track", "Kit"],
    ];
    let mut written = 0;
    for song in &songs {
        for form in ["g.drums", "g.drums.json", "g.drums.toml"] {
            for flags in flag_sets {
                if same_as_drums_read(&dir, song, form, flags).status.success() {
                    written += 1;
                }
            }
        }
    }
    // groove's track is "Drums", so --track Kit is refused for it, the same both ways.
    assert_eq!(written, 3 * 3 * 3 + 3 * 4);
    let out = same_as_drums_read(&dir, &songs[3], "g.drums", &["--grid", "1/16"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("left out 1 other track(s), and 1 control(s)"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `convert` refuses what `drums read` refuses, with the same words: a note off the grid,
/// a key no lane plays, a velocity off the levels as text, a hit past --bars, and a song
/// with no drum track.
#[test]
fn convert_refuses_what_drums_read_refuses() {
    let dir = scratch("refuse");
    let cases = [
        (
            "off-grid",
            "[track \"Kit\" ch10]\n1.1.10 C2 0.25 120\n",
            "g.drums",
        ),
        (
            "no-lane",
            "[track \"Kit\" ch10]\n1.1.0 C1 0.25 120\n",
            "g.drums",
        ),
        (
            "off-level",
            "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n",
            "g.drums",
        ),
        (
            "past-bars",
            "[track \"Kit\" ch10]\n3.1.0 C2 0.25 120\n",
            "g.drums.json",
        ),
    ];
    for (name, text, out) in cases {
        let song = dir.join(format!("{name}.notes"));
        std::fs::write(&song, text).expect("song");
        let conv = same_as_drums_read(&dir, &song, out, &["--grid", "1/16", "--bars", "1"]);
        assert!(!conv.status.success(), "{name} is refused");
    }
    let off_level = dir.join("off-level.notes");
    let conv = same_as_drums_read(&dir, &off_level, "g.drums", &["--grid", "1/16"]);
    assert!(String::from_utf8_lossy(&conv.stderr).contains("--snap-levels"));
    let conv = same_as_drums_read(
        &dir,
        &golden("sketch.notes"),
        "g.drums",
        &["--grid", "1/16"],
    );
    assert!(!conv.status.success(), "sketch has no drum track");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A grid output without `--grid` is refused naming `--grid`, and each grid flag given
/// for an output that is not a grid is refused by name. Nothing is written.
#[test]
fn grid_flags_are_never_guessed_and_never_ignored() {
    let dir = scratch("flags");
    let groove = golden("groove.mid");
    for out in ["g.drums", "g.drums.json", "g.drums.toml"] {
        let path = dir.join(out);
        let o = duet(&dir, &["convert", s(&groove), s(&path)]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            !o.status.success() && err.contains("needs --grid"),
            "{out}: {err}"
        );
        assert!(!path.exists(), "{out}: nothing written");
    }
    let o = duet(&dir, &["convert", s(&groove), "-", "--to", "drums"]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("needs --grid"));
    assert!(o.stdout.is_empty());

    let flags: [&[&str]; 4] = [
        &["--grid", "1/16"],
        &["--track", "Drums"],
        &["--bars", "2"],
        &["--snap-levels"],
    ];
    for out in ["g.mid", "g.notes", "g.json", "g.toml"] {
        for flag in flags {
            let path = dir.join(out);
            let mut args = vec!["convert", s(&groove), s(&path)];
            args.extend(flag);
            let o = duet(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr);
            assert!(
                !o.status.success()
                    && err.contains(&format!("{} is for writing a drum grid", flag[0])),
                "{out} {flag:?}: {err}"
            );
            assert!(!path.exists(), "{out}: nothing written");
        }
    }
    let o = duet(
        &dir,
        &[
            "convert",
            s(&groove),
            "-",
            "--to",
            "text",
            "--grid",
            "1/16",
            "--bars",
            "2",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("--grid, --bars are for writing a drum grid"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `--from` and `--to` name the grid forms, so standard input and output carry them; a
/// plain `.json` is still a song, and a `.drums.json` is a grid.
#[test]
fn grid_forms_by_name_on_standard_input_and_output() {
    let dir = scratch("stdio");
    let none = dir.join("no-config");
    let text = std::fs::read(golden("groove.drums")).expect("golden");
    let o = duet_with(
        &dir,
        &none,
        &[
            "convert", "-", "-", "--from", "drums", "--to", "drums", "--grid", "1/16",
        ],
        &text,
    );
    ok(&o, "drums to drums");
    // The golden predates the `end` line; the writer's form of it today is what comes back.
    let canon = dir.join("canon.drums");
    ok(
        &duet(
            &dir,
            &["drums", "snap", s(&golden("groove.drums")), s(&canon)],
        ),
        "canon",
    );
    assert!(
        o.stdout == std::fs::read(&canon).expect("canon"),
        "the same grid through standard input and output"
    );

    let exact = std::fs::read(golden("groove.drums.json")).expect("golden");
    let o = duet_with(
        &dir,
        &none,
        &[
            "convert",
            "-",
            "-",
            "--from",
            "drums-json",
            "--to",
            "drums-toml",
            "--grid",
            "1/16",
        ],
        &exact,
    );
    ok(&o, "drums-json to drums-toml");
    let toml_path = dir.join("g.drums.toml");
    ok(
        &duet(
            &dir,
            &[
                "drums",
                "read",
                s(&golden("groove.mid")),
                s(&toml_path),
                "--grid",
                "1/16",
            ],
        ),
        "toml",
    );
    assert!(o.stdout == std::fs::read(&toml_path).expect("toml"));

    let o = duet_with(
        &dir,
        &none,
        &["convert", "-", "-", "--from", "drums"],
        &text,
    );
    ok(&o, "drums to text");
    let want = std::fs::read(golden("groove.notes")).expect("golden");
    let got = String::from_utf8(o.stdout).expect("utf-8");
    assert_eq!(
        got.replace("end 3.1.0\n", ""),
        String::from_utf8(want).expect("utf-8"),
        "the grid as a song"
    );

    // A plain .json is a song; a song written to a name ending .drums.json is read as a grid,
    // and refused as one.
    let song_json = dir.join("song.json");
    ok(
        &duet(
            &dir,
            &["convert", s(&golden("groove.notes")), s(&song_json)],
        ),
        "song.json",
    );
    ok(
        &duet(&dir, &["convert", s(&song_json), s(&dir.join("song.mid"))]),
        "song.json read",
    );
    let named = dir.join("song.drums.json");
    std::fs::copy(&song_json, &named).expect("copy");
    let o = duet(&dir, &["convert", s(&named), s(&dir.join("named.mid"))]);
    assert!(
        !o.status.success(),
        "a song named .drums.json is read as a grid"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A small deterministic generator, so the generated grids are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// Lanes in the gm map's row order.
const LANES: [&str; 9] = [
    "crash",
    "ride",
    "hh",
    "hh-open",
    "tom-high",
    "tom-floor",
    "snare",
    "kick",
    "cowbell",
];

/// A grid in .drums text: a meter, a step, some bars, and lanes in row order each played
/// at least once.
fn generated(rng: &mut Lcg) -> (String, &'static str) {
    let (time, beats) = [("4/4", 4), ("3/4", 3), ("6/8", 3)][rng.next(3) as usize];
    // The grid and its steps per quarter note. `beats` counts quarters: 6/8 is three long.
    let (grid, per_quarter) = [("1/16", 4), ("1/8", 2), ("1/8t", 3)][rng.next(3) as usize];
    let steps = beats * per_quarter;
    let bars = 1 + rng.next(4) as u32;
    let lanes: Vec<&str> = LANES.iter().copied().filter(|_| rng.next(2) == 0).collect();
    let lanes = if lanes.is_empty() {
        vec!["kick"]
    } else {
        lanes
    };
    let mut text = format!(
        "tempo {}\ntime {time}\ngrid {grid}\n\n[bars 1-{bars}]\n",
        60 + rng.next(120)
    );
    for lane in &lanes {
        let mut cells: Vec<char> = (0..bars * steps)
            .map(|_| ['.', '.', '.', 'g', 'x', 'X'][rng.next(6) as usize])
            .collect();
        if cells.iter().all(|c| *c == '.') {
            let at = rng.next(cells.len() as u64) as usize;
            cells[at] = 'x';
        }
        text.push_str(&format!("{lane:<9} |"));
        for bar in cells.chunks(steps as usize) {
            for c in bar {
                text.push(' ');
                text.push(*c);
            }
            text.push_str(" |");
        }
        text.push('\n');
    }
    (text, grid)
}

/// Round trips: grid → `convert` → .mid → `convert --grid` → grid gives the same grid, for
/// the sample and the goldens, and for generated grids in the text and the exact form.
#[test]
fn a_grid_survives_midi_by_convert_alone() {
    let dir = scratch("roundtrip");
    let through = |grid: &Path, out: &Path, step: &str| {
        let mid = dir.join("through.mid");
        ok(&duet(&dir, &["convert", s(grid), s(&mid)]), "to MIDI");
        ok(
            &duet(&dir, &["convert", s(&mid), s(out), "--grid", step]),
            "back",
        );
        std::fs::read(out).expect("back")
    };
    for grid in grids(&dir) {
        let name = grid.file_name().and_then(|n| n.to_str()).expect("a name");
        let back = through(&grid, &dir.join(format!("back-{name}")), "1/16");
        // The sample is hand-written; the writer's form of it is what comes back.
        let canon = dir.join(format!("canon-{name}"));
        ok(
            &duet(&dir, &["drums", "snap", s(&grid), s(&canon)]),
            "canon",
        );
        assert!(
            back == std::fs::read(&canon).expect("canon"),
            "{name} came back changed"
        );
    }

    let mut rng = Lcg(0x5eed);
    for i in 0..40 {
        let (text, step) = generated(&mut rng);
        let made = dir.join(format!("gen-{i}.drums"));
        std::fs::write(&made, &text).expect("gen");
        let canon = dir.join(format!("gen-{i}-canon.drums"));
        ok(&duet(&dir, &["drums", "snap", s(&made), s(&canon)]), &text);
        let back = through(&made, &dir.join(format!("gen-{i}-back.drums")), step);
        assert!(
            back == std::fs::read(&canon).expect("canon"),
            "generated grid {i} came back changed:\n{text}\n{}",
            String::from_utf8_lossy(&back)
        );

        // The exact form, with velocities off the levels.
        let exact = dir.join(format!("gen-{i}.drums.json"));
        ok(
            &duet(&dir, &["convert", s(&made), s(&exact), "--grid", step]),
            "exact",
        );
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&exact).expect("exact")).expect("json");
        for bar in json["bars"].as_array_mut().expect("bars") {
            for row in bar["vel"].as_array_mut().expect("vel") {
                for v in row.as_array_mut().expect("row") {
                    if v.as_u64() != Some(0) {
                        *v = serde_json::Value::from(1 + rng.next(127));
                    }
                }
            }
        }
        let mut bytes = serde_json::to_string_pretty(&json).expect("json");
        bytes.push('\n');
        std::fs::write(&exact, &bytes).expect("exact");
        let back = through(&exact, &dir.join(format!("gen-{i}-back.drums.json")), step);
        // `Value` keeps keys sorted, not in the writer's order, so compare as JSON.
        let back: serde_json::Value = serde_json::from_slice(&back).expect("json back");
        assert_eq!(back, json, "generated exact grid {i} came back changed");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}
