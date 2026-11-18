//! `duet convert` and drum grids: a drum-grid document is read as `duet drums render` reads
//! it and written as `duet drums read` writes it, byte for byte and refusal for refusal; grid
//! flags are never guessed and never silently ignored; and a grid survives a round trip through
//! MIDI by `convert` alone. A grid's text is a view: `duet apply` reads it into a drum-grid
//! document and `duet view` shows one as text, and `convert` reads and writes neither.

use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, duet_with, golden, notes_doc, ok, root, s, scratch, view_text};

/// The grids to read, the sample and the golden, each read into a drum-grid document in `dir`
/// with `duet apply`.
fn grids(dir: &Path) -> Vec<PathBuf> {
    [
        ("sample", root().join("samples/groove.drums")),
        ("golden", golden("groove.drums")),
    ]
    .into_iter()
    .map(|(name, text)| applied(dir, name, &text))
    .collect()
}

/// A grid's text read into a drum-grid document `dir/<name>.duet` with `duet apply`.
fn applied(dir: &Path, name: &str, text: &Path) -> PathBuf {
    let doc = dir.join(format!("{name}.duet"));
    ok(
        &duet(dir, &["apply", s(text), "--as", "grid", "-o", s(&doc)]),
        &format!("apply {}", text.display()),
    );
    doc
}

/// A part's notes text from a file, read into a part document with `duet apply`.
fn notes_of(dir: &Path, name: &str, path: &Path) -> PathBuf {
    notes_doc(dir, name, &std::fs::read_to_string(path).expect("notes"))
}

/// Equivalence, reading: for every sample and golden grid, by name and on standard output,
/// `convert` writes the same bytes, and says the same on standard error, as `drums render`.
#[test]
fn convert_reads_every_grid_as_drums_render_does() {
    let dir = scratch("read");
    let mut compared = 0;
    for grid in grids(&dir) {
        let stem = grid.file_stem().and_then(|n| n.to_str()).expect("a name");
        // A .duet from convert keeps the grid (a drum-grid document); `drums render` makes a
        // part, so the two are compared on MIDI, by name and on standard output. (ABC refuses
        // a drum part.)
        for (ext, to) in [("mid", None), ("-", Some("midi"))] {
            let (a, b) = if ext == "-" {
                ("-".to_owned(), "-".to_owned())
            } else {
                (
                    s(&dir.join(format!("{stem}.convert.{ext}"))).to_owned(),
                    s(&dir.join(format!("{stem}.render.{ext}"))).to_owned(),
                )
            };
            let mut conv_args = vec!["convert", s(&grid), a.as_str()];
            let mut rend_args = vec!["drums", "render", s(&grid), b.as_str()];
            if let Some(to) = to {
                conv_args.extend(["--to", to]);
                rend_args.extend(["--to", to]);
            }
            let conv = duet(&dir, &conv_args);
            let rend = duet(&dir, &rend_args);
            ok(&conv, &format!("convert {stem} .{ext}"));
            ok(&rend, &format!("render {stem} .{ext}"));
            assert_eq!(conv.stderr, rend.stderr, "{stem} .{ext}: stderr");
            let (a, b) = if ext == "-" {
                (conv.stdout, rend.stdout)
            } else {
                (std::fs::read(&a).expect("a"), std::fs::read(&b).expect("b"))
            };
            assert!(!a.is_empty(), "{stem} .{ext}: something is written");
            assert!(a == b, "{stem} to .{ext}: convert and drums render differ");
            compared += 1;
        }
    }
    assert_eq!(compared, 4);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A grid's text keeps the `map gm` rule as `duet apply` reads it: a config that moves gm lanes
/// does not move them in text, the move is said on standard error, and the alias the config
/// adds still reads. `convert` and `drums render` then make the same part of the document.
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
    let plain_doc = applied(&dir, "plain", &plain_beat);
    ok(&duet(&dir, &["convert", s(&plain_doc), s(&plain)]), "plain");

    let doc = dir.join("beat.duet");
    let applied = duet_with(
        &dir,
        &user,
        &["apply", s(&beat), "--as", "grid", "-o", s(&doc)],
        b"",
    );
    ok(&applied, "apply under the config");
    let err = String::from_utf8_lossy(&applied.stderr);
    assert!(err.contains("lane kick: key 100 does not apply"), "{err}");

    let (a, b) = (dir.join("a.mid"), dir.join("b.mid"));
    let conv = duet_with(&dir, &user, &["convert", s(&doc), s(&a)], b"");
    let rend = duet_with(&dir, &user, &["drums", "render", s(&doc), s(&b)], b"");
    ok(&conv, "convert");
    ok(&rend, "render");
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
fn same_as_drums_read(dir: &Path, part: &Path, out_name: &str, flags: &[&str]) -> Output {
    let a = dir.join(format!("convert-{out_name}"));
    let b = dir.join(format!("read-{out_name}"));
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    let mut conv_args = vec!["convert", s(part), s(&a)];
    conv_args.extend(flags);
    let mut read_args = vec!["drums", "read", s(part), s(&b)];
    read_args.extend(flags);
    let conv = duet(dir, &conv_args);
    let read = duet(dir, &read_args);
    let what = format!("{} to {out_name} {flags:?}", part.display());
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

/// Equivalence, writing: for the golden parts with a drum track, and a part with other
/// tracks and controls, `convert … g.duet --grid` writes the same drum-grid document and the
/// same standard error as `drums read`, with every flag.
#[test]
fn convert_writes_every_grid_as_drums_read_does() {
    let dir = scratch("write");
    let band = notes_doc(
        &dir,
        "band.notes",
        "[track \"Keys\" ch1]\n1.1.0 C4 1 80\n\n[track \"Kit\" ch10]\n1.1.0 C2 0.25 120\n\
         1.1.0 F#2 0.25 90\n1.2.0 F#2 0.25 90\n1.3.0 D2 0.25 120\n1.3.0 F#2 0.25 90\n\
         1.4.0 F#2 0.25 40\n1.1.0 cc4 90\n",
    );
    let doc = dir.join("groove.duet");
    ok(
        &duet(&dir, &["convert", s(&golden("groove.mid")), s(&doc)]),
        "groove.duet",
    );
    let parts: [PathBuf; 4] = [
        golden("groove.mid"),
        notes_of(&dir, "groove-notes.notes", &golden("groove.notes")),
        doc,
        band,
    ];
    let flag_sets: [&[&str]; 4] = [
        &["--grid", "1/16"],
        &["--grid", "1/16", "--bars", "4"],
        &["--grid", "1/16", "--snap-levels"],
        &["--grid", "1/16", "--track", "Kit"],
    ];
    let mut written = 0;
    for part in &parts {
        for flags in flag_sets {
            if same_as_drums_read(&dir, part, "g.duet", flags)
                .status
                .success()
            {
                written += 1;
            }
        }
    }
    // groove's track is "Drums", so --track Kit is refused for it, the same both ways.
    assert_eq!(written, 3 * 3 + 4);
    let out = same_as_drums_read(&dir, &parts[3], "g.duet", &["--grid", "1/16"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("left out 1 other track(s), and 1 control(s)"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// `convert` refuses what `drums read` refuses, with the same words: a note off the grid,
/// a key no lane plays, a hit past --bars, and a part with no drum track. A velocity off the
/// levels is no refusal for either, since a drum-grid document keeps it exactly: the two
/// agree on it, and the grid's view is what refuses it, until --snap-levels.
#[test]
fn convert_refuses_what_drums_read_refuses() {
    let dir = scratch("refuse");
    let cases = [
        (
            "off-grid",
            "[track \"Kit\" ch10]\n1.1.10 C2 0.25 120\n",
            false,
        ),
        (
            "no-lane",
            "[track \"Kit\" ch10]\n1.1.0 C1 0.25 120\n",
            false,
        ),
        (
            "off-level",
            "[track \"Kit\" ch10]\n1.1.0 C2 0.25 100\n",
            true,
        ),
        (
            "past-bars",
            "[track \"Kit\" ch10]\n3.1.0 C2 0.25 120\n",
            false,
        ),
    ];
    for (name, text, written) in cases {
        let part = notes_doc(&dir, &format!("{name}.notes"), text);
        let conv = same_as_drums_read(&dir, &part, "g.duet", &["--grid", "1/16", "--bars", "1"]);
        assert_eq!(conv.status.success(), written, "{name}");
    }
    let off_level = dir.join("off-level.duet");
    let conv = same_as_drums_read(&dir, &off_level, "g.duet", &["--grid", "1/16"]);
    ok(&conv, "a document keeps an off-level velocity");
    let view = duet(
        &dir,
        &["view", s(&dir.join("convert-g.duet")), "--as", "grid"],
    );
    assert!(!view.status.success(), "the view refuses it");
    assert!(String::from_utf8_lossy(&view.stderr).contains("--snap-levels"));
    let sketch = notes_of(&dir, "sketch.notes", &golden("sketch.notes"));
    let conv = same_as_drums_read(&dir, &sketch, "g.duet", &["--grid", "1/16"]);
    assert!(!conv.status.success(), "sketch has no drum track");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A grid output without `--grid` is refused naming `--grid`, and each grid flag given
/// for an output that is not a grid is refused by name. A grid's text is no output of
/// `convert`. Nothing is written.
#[test]
fn grid_flags_are_never_guessed_and_never_ignored() {
    let dir = scratch("flags");
    let groove = golden("groove.mid");
    for flag in [
        &["--track", "Drums"][..],
        &["--bars", "2"],
        &["--snap-levels"],
    ] {
        let path = dir.join("g.duet");
        let mut args = vec!["convert", s(&groove), s(&path)];
        args.extend(flag);
        let o = duet(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            !o.status.success() && err.contains("needs --grid"),
            "{flag:?}: {err}"
        );
        assert!(!path.exists(), "{flag:?}: nothing written");
    }
    for out in ["g.drums", "g.grid"] {
        let path = dir.join(out);
        let o = duet(&dir, &["convert", s(&groove), s(&path), "--grid", "1/16"]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            !o.status.success() && err.contains("written only by `duet view`"),
            "{out}: {err}"
        );
        assert!(!path.exists(), "{out}: nothing written");
    }
    let o = duet(&dir, &["convert", s(&groove), "-", "--snap-levels"]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("needs --grid"));
    assert!(o.stdout.is_empty());

    let flags: [&[&str]; 4] = [
        &["--grid", "1/16"],
        &["--track", "Drums"],
        &["--bars", "2"],
        &["--snap-levels"],
    ];
    for out in ["g.mid", "g.abc"] {
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
            "midi",
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
    assert!(o.stdout.is_empty());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A grid's text goes through standard input and output by `duet apply` and `duet view`, and a
/// drum-grid document between them through `convert`'s.
#[test]
fn a_grid_text_on_standard_input_and_output() {
    let dir = scratch("stdio");
    let none = dir.join("no-config");
    let text = std::fs::read(golden("groove.drums")).expect("golden");
    let applied_out = duet_with(
        &dir,
        &none,
        &["apply", "-", "--as", "grid", "-o", "-"],
        &text,
    );
    ok(&applied_out, "a grid's text to a document");
    let o = duet_with(
        &dir,
        &none,
        &["convert", "-", "-", "--grid", "1/16"],
        &applied_out.stdout,
    );
    ok(&o, "a document to a document");
    let viewed = duet_with(&dir, &none, &["view", "-", "--as", "grid"], &o.stdout);
    ok(&viewed, "a document to its view");
    // The golden predates the `end` line; the writer's form of it today is what comes back.
    let canon = view_text(
        &dir,
        &applied(&dir, "canon", &golden("groove.drums")),
        "grid",
        &[],
    );
    let viewed = String::from_utf8(viewed.stdout).expect("utf-8");
    assert_eq!(
        viewed.split_once('\n').map(|(_, rest)| rest),
        Some(canon.as_str()),
        "the same grid through standard input and output"
    );

    let o = duet_with(
        &dir,
        &none,
        &["drums", "render", "-", "-"],
        &applied_out.stdout,
    );
    ok(&o, "a grid to a part");
    let part = duet_with(&dir, &none, &["view", "-", "--as", "notes"], &o.stdout);
    ok(&part, "the part's view");
    let want = std::fs::read(golden("groove.notes")).expect("golden");
    let got = String::from_utf8(part.stdout).expect("utf-8");
    let got = got.split_once('\n').map_or("", |(_, rest)| rest);
    assert_eq!(
        got.replace("end 3.1.0\n", ""),
        String::from_utf8(want).expect("utf-8"),
        "the grid as a part"
    );

    // A grid's text in, a document out: the drum-grid document `drums snap` writes.
    let copied = dir.join("copied.duet");
    ok(
        &duet(
            &dir,
            &["drums", "snap", s(&dir.join("canon.duet")), s(&copied)],
        ),
        "snap to .duet",
    );
    assert!(applied_out.stdout == std::fs::read(&copied).expect("copied"));
    let doc = String::from_utf8(applied_out.stdout).expect("utf-8");
    assert!(doc.contains("\"kind\": \"drum-grid\""), "{doc}");
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

/// A grid's view text: a meter, a step, some bars, and lanes in row order each played
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
/// the sample and the golden, and for generated grids, as their views and as drum-grid
/// documents with velocities off the view's levels, which a document keeps exactly.
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
        view_text(&dir, out, "grid", &[])
    };
    for grid in grids(&dir) {
        let name = grid.file_stem().and_then(|n| n.to_str()).expect("a name");
        let back = through(&grid, &dir.join(format!("back-{name}.duet")), "1/16");
        // The sample is hand-written; the writer's form of it is what comes back.
        let canon = view_text(&dir, &grid, "grid", &[]);
        assert_eq!(back, canon, "{name} came back changed");
    }

    let mut rng = Lcg(0x5eed);
    for i in 0..40 {
        let (text, step) = generated(&mut rng);
        let made = dir.join(format!("gen-{i}.drums"));
        std::fs::write(&made, &text).expect("gen");
        let doc = applied(&dir, &format!("gen-{i}"), &made);
        let canon = view_text(&dir, &doc, "grid", &[]);
        let back = through(&doc, &dir.join(format!("gen-{i}-back.duet")), step);
        assert_eq!(back, canon, "generated grid {i} came back changed:\n{text}");

        // As a document, with velocities off the levels: .duet → .mid → .duet.
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&doc).expect("doc")).expect("json");
        for v in json["body"]["vel"]["data"]
            .as_array_mut()
            .expect("vel data")
        {
            if v.as_u64() != Some(0) {
                *v = serde_json::Value::from(1 + rng.next(127));
            }
        }
        let mut bytes = serde_json::to_string_pretty(&json).expect("json");
        bytes.push('\n');
        std::fs::write(&doc, &bytes).expect("doc");
        let mid = dir.join("through-doc.mid");
        ok(
            &duet(&dir, &["convert", s(&doc), s(&mid)]),
            "a document to MIDI",
        );
        let back = dir.join(format!("gen-{i}-back-exact.duet"));
        ok(
            &duet(&dir, &["convert", s(&mid), s(&back), "--grid", step]),
            "MIDI to a document",
        );
        let back: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&back).expect("back")).expect("json back");
        // `Value` keeps keys sorted, not in the writer's order, so compare as JSON.
        assert_eq!(
            back["body"], json["body"],
            "generated grid {i}'s document came back changed"
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}
