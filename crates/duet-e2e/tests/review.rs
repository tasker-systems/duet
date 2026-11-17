//! `duet capture review`: a take reviewed moment by moment on one page.
//!
//! The sheet test reads "Wayfaring Stranger" (`crates/duet-capture/corpus`, CC BY 4.0) with both
//! readers and reviews its committed moments file, the 14 moments the owner ruled on (task D).
//! The refusals and the licence warning use a synthetic take from `capture/audio.rs`. Every
//! sheet is written to a scratch directory.

#[path = "capture/audio.rs"]
#[expect(
    dead_code,
    reason = "the capture tests' shared generators: these tests use the tones, not the clicks or bursts"
)]
mod audio;

use std::path::{Path, PathBuf};

use duet_e2e::{duet, ok, root, s, scratch};

/// The corpus take and its moments file.
fn corpus() -> (PathBuf, PathBuf) {
    let dir = root().join("crates/duet-capture/corpus");
    (
        dir.join("wayfaring-stranger.flac"),
        dir.join("wayfaring-stranger.moments"),
    )
}

/// The moments in a moments file: `(from, to)` in seconds, in order.
fn moments(path: &Path) -> Vec<(f64, f64)> {
    std::fs::read_to_string(path)
        .expect("read moments")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut parts = l.split_whitespace();
            let mut next = || {
                parts
                    .next()
                    .expect("a field")
                    .parse::<f64>()
                    .expect("seconds")
            };
            (next(), next())
        })
        .collect()
}

/// A two-channel float WAV's rate and its length in sample frames.
fn wav_frames(path: &Path) -> (u32, usize) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&bytes[..4], b"RIFF", "{}", path.display());
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
    assert_eq!(u16_at(22), 2, "two channels");
    let mut i = 20 + u32_at(16) as usize;
    while &bytes[i..i + 4] != b"data" {
        i += 8 + u32_at(i + 4) as usize;
    }
    (u32_at(24), u32_at(i + 4) as usize / 8)
}

/// Every `src` of every player on the page.
fn sources(page: &str) -> Vec<String> {
    page.split("<audio ")
        .skip(1)
        .map(|a| {
            let at = a.find("src=\"").expect("a src") + 5;
            a[at..at + a[at..].find('"').expect("a closing quote")].to_owned()
        })
        .collect()
}

#[test]
fn a_sheet_reviews_every_moment_with_each_readings_plot_clips_and_verdict_box() {
    let dir = scratch("review-sheet");
    let (take, moments_file) = corpus();
    let hmm = dir.join("ws");
    let rules = dir.join("ws-rules");
    ok(
        &duet(&dir, &["capture", "melody", s(&take), "-o", s(&hmm)]),
        "capture melody",
    );
    ok(
        &duet(
            &dir,
            &[
                "capture",
                "melody",
                s(&take),
                "--reader",
                "rules",
                "-o",
                s(&rules),
            ],
        ),
        "capture melody --reader rules",
    );
    let sheet = dir.join("sheet");
    let rules_reading = format!("rules={}", s(&rules.with_extension("capture.duet")));
    let out = duet(
        &dir,
        &[
            "capture",
            "review",
            s(&take),
            "-o",
            s(&hmm),
            "--reading",
            &rules_reading,
            "--moments",
            s(&moments_file),
            "--dir",
            s(&sheet),
        ],
    );
    ok(&out, "capture review");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("CC BY 4.0"),
        "the licence is stated: {stderr}"
    );

    let page = std::fs::read_to_string(sheet.join("index.html")).expect("the sheet");
    let windows = moments(&moments_file);
    assert_eq!(
        windows.len(),
        14,
        "the committed moments file names 14 moments"
    );
    let readings = ["hmm", "rules"];

    // Each moment: a section, and per reading its five clips, each the moment's length exactly.
    let srcs = sources(&page);
    for (i, &(from, to)) in windows.iter().enumerate() {
        let n = i + 1;
        assert!(
            page.contains(&format!("<section id=\"m{n}\">")),
            "moment {n}'s section"
        );
        assert!(
            page.contains(&format!("<textarea data-moment=\"{n}\"")),
            "moment {n}'s verdict box"
        );
        for reading in readings {
            for rendering in ["melody", "steps", "bends", "legato", "gapped"] {
                let src = format!("clips/m{n}-{reading}-{rendering}.wav");
                assert!(srcs.contains(&src), "the page plays {src}");
                let (rate, frames) = wav_frames(&sheet.join(&src));
                let expected = (to * f64::from(rate)).round() as usize
                    - (from * f64::from(rate)).round() as usize;
                assert_eq!(frames, expected, "{src} is {from}–{to} s long");
            }
        }
    }
    // The whole take's melody per reading, and nothing the page does not play.
    for reading in readings {
        assert!(srcs.contains(&format!("clips/take-{reading}.wav")));
    }
    assert_eq!(srcs.len(), 14 * 2 * 5 + 2);
    let on_disk = std::fs::read_dir(sheet.join("clips"))
        .expect("clips")
        .count();
    assert_eq!(
        on_disk,
        srcs.len(),
        "every clip is played, and every player has its clip"
    );
    // A plot per moment per reading, and per reading for the whole take, inline.
    assert_eq!(page.matches("<svg class=\"plot\"").count(), 14 * 2 + 2);
    assert_eq!(page.matches("<textarea ").count(), 14);
    // The page explains itself.
    for explained in [
        "How to read this sheet",
        "<b>The plot.</b>",
        "<i>bends</i>",
        "A short gap",
        "An excursion",
    ] {
        assert!(page.contains(explained), "the page explains: {explained}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A short synthetic take captured at `dir/take`, with no licence file beside it.
fn synthetic(dir: &Path) -> PathBuf {
    let take = dir.join("take.wav");
    audio::write_wav(
        &take,
        &audio::notes_off(0.5, &[(60, 0.6), (64, 0.6), (67, 0.9)], 0.0),
    );
    ok(
        &duet(dir, &["capture", "melody", s(&take), "--tempo", "120"]),
        "capture melody",
    );
    take
}

fn moments_file(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("take.moments");
    std::fs::write(&path, text).expect("write moments");
    path
}

#[test]
fn a_moment_past_the_takes_end_is_refused_before_anything_is_written() {
    let dir = scratch("review-outside");
    let take = synthetic(&dir);
    let moments = moments_file(
        &dir,
        "0.5 1.0 the first note\n# a comment\n5.0 6.0 past the end\n",
    );
    let out = duet(
        &dir,
        &["capture", "review", s(&take), "--moments", s(&moments)],
    );
    assert!(!out.status.success(), "a moment past the end is refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("moment 2 (line 3)") && stderr.contains("past the take's end"),
        "{stderr}"
    );
    assert!(!dir.join("take.review").exists(), "nothing is written");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_sheet_is_never_overwritten_and_a_take_with_no_licence_is_kept_out_of_git() {
    let dir = scratch("review-twice");
    let take = synthetic(&dir);
    let moments = moments_file(&dir, "0.5 1.0 the first note\n1.1 1.6 the second\n");
    let args = ["capture", "review", s(&take), "--moments", s(&moments)];
    let first = duet(&dir, &args);
    ok(&first, "capture review");
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(
        stderr.contains("no ")
            && stderr.contains(".license beside the take")
            && stderr.contains("out of git"),
        "{stderr}"
    );
    let sheet = dir.join("take.review");
    let page = std::fs::read(sheet.join("index.html")).expect("the sheet");
    assert!(String::from_utf8_lossy(&page).contains("No licence file sits beside this take"));
    let clips = std::fs::read_dir(sheet.join("clips"))
        .expect("clips")
        .count();

    let second = duet(&dir, &args);
    assert!(!second.status.success(), "a second run is refused");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("never overwrites"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        std::fs::read(sheet.join("index.html")).expect("the sheet"),
        page
    );
    assert_eq!(
        std::fs::read_dir(sheet.join("clips"))
            .expect("clips")
            .count(),
        clips
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
