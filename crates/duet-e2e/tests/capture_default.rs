//! `duet capture melody` with nothing asked: what it writes, byte for byte.
//!
//! The goldens in `golden/capture/PLATFORM/` were written by `main`'s binary at `de2e831`, before `duet
//! capture disagree` existed (temper `01a1f6e0-d939-76d8-af14-0a932a1f5819`). This test holds that
//! a capture run without asking for a disagreement behaves exactly as it did then: every file
//! written, standard output and standard error, and the set of files, so no `.disagree.moments`
//! appears unasked. The one field masked is the part's `body.id`, which is minted fresh each run.
//!
//! The takes are the corpus's synthetic ones, both MIT: `synthetic-disagree.flac`, the witness
//! take of `duet capture disagree`, and `synthetic-melody.flac`. "Wayfaring Stranger" is left to
//! its listened corpus test: a golden of its readings would commit data derived from a CC BY take.
//!
//! **One set per platform** (`macos-aarch64`, `linux-x86_64`): the pitch track's last f32 digits
//! differ between macOS's and glibc's maths, so one platform's bytes are not another's. Each set
//! is exact on its own platform. A platform with no set fails, naming how to write one.
//! `macos-aarch64` was written on the owner's machine, `linux-x86_64` in a `linux/amd64` container
//! (`rust:1`, Rust 1.100.0), each from `main` at `de2e831`.
//!
//! **Rewriting the goldens** is for a change that means to change the default capture, never to
//! make this pass. Build the binary to bless (for this test's own goldens, `main`'s, in a separate
//! checkout) and run
//! `DUET_E2E_BIN=<it> DUET_E2E_BLESS=1 cargo nextest run -p duet-e2e --test capture_default`.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, golden_dir, ok, root, scratch, without_id};

/// The takes, in the corpus, captured with nothing asked.
const TAKES: [&str; 2] = ["synthetic-disagree", "synthetic-melody"];

/// What `duet capture melody TAKE.flac` writes beside the take.
const WRITTEN: [&str; 4] = [".duet", ".abc", ".capture.duet", ".performance.duet.jsonl"];

/// This platform's goldens.
fn goldens() -> PathBuf {
    golden_dir().join("capture").join(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

/// `take` captured with nothing asked, in a scratch directory of its own: the files written,
/// each as `(suffix, bytes)` with a `.duet`'s id masked, then `.stdout` and `.stderr`.
fn capture(take: &str) -> Vec<(String, Vec<u8>)> {
    let dir = scratch(&format!("capture-default-{take}"));
    let flac = format!("{take}.flac");
    std::fs::copy(
        root().join("crates/duet-capture/corpus").join(&flac),
        dir.join(&flac),
    )
    .expect("copy the take");
    let out = duet(&dir, &["capture", "melody", &flac]);
    ok(&out, &format!("capture melody {flac}"));

    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .expect("read the scratch dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n != &flac && n != "no-config")
        .collect();
    found.sort();
    let mut want: Vec<String> = WRITTEN.iter().map(|s| format!("{take}{s}")).collect();
    want.sort();
    assert_eq!(found, want, "{take}: the files written, and no other");

    let mut files: Vec<(String, Vec<u8>)> = WRITTEN
        .iter()
        .map(|suffix| {
            let bytes = std::fs::read(dir.join(format!("{take}{suffix}"))).expect("written");
            let bytes = if *suffix == ".duet" {
                without_id(&bytes)
            } else {
                bytes
            };
            ((*suffix).to_owned(), bytes)
        })
        .collect();
    files.push((".stdout".to_owned(), out.stdout));
    files.push((".stderr".to_owned(), out.stderr));
    files
}

fn golden_path(take: &str, suffix: &str) -> PathBuf {
    goldens().join(format!("{take}{suffix}"))
}

fn first_difference(a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let line = a[..at].iter().filter(|&&c| c == b'\n').count() + 1;
    format!("first difference at byte {at} (line {line})")
}

fn bless(take: &str, files: &[(String, Vec<u8>)]) {
    std::fs::create_dir_all(goldens()).expect("golden/capture");
    for (suffix, bytes) in files {
        std::fs::write(golden_path(take, suffix), bytes).expect("write a golden");
    }
}

fn compare(take: &str, files: &[(String, Vec<u8>)]) {
    for (suffix, got) in files {
        let path = golden_path(take, suffix);
        let want = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; this platform has no goldens, so write them from main's binary \
                 (DUET_E2E_BLESS, above)",
                path.display()
            )
        });
        assert!(
            *got == want,
            "{take}{suffix} differs from {}: {}",
            path.display(),
            first_difference(got, &want)
        );
    }
}

fn holds(take: &str) {
    let files = capture(take);
    if std::env::var_os("DUET_E2E_BLESS").is_some() {
        bless(take, &files);
    } else {
        compare(take, &files);
    }
}

#[test]
fn the_witness_take_captures_exactly_as_before_disagree() {
    holds(TAKES[0]);
}

#[test]
fn the_synthetic_melody_captures_exactly_as_before_disagree() {
    holds(TAKES[1]);
}

#[test]
fn every_golden_is_of_a_take_and_a_file_the_capture_writes() {
    let mut names: Vec<String> = std::fs::read_dir(goldens())
        .expect("golden/capture")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut want: Vec<String> = TAKES
        .iter()
        .flat_map(|take| {
            WRITTEN
                .iter()
                .copied()
                .chain([".stdout", ".stderr"])
                .map(move |suffix| format!("{take}{suffix}"))
        })
        .collect();
    want.sort();
    assert_eq!(names, want, "{}", goldens().display());
}

#[test]
fn every_platforms_goldens_hold_the_same_files() {
    let platforms: Vec<PathBuf> = std::fs::read_dir(golden_dir().join("capture"))
        .expect("golden/capture")
        .map(|e| e.expect("entry").path())
        .collect();
    assert!(platforms.len() >= 2, "{platforms:?}");
    let names = |dir: &Path| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("a platform's goldens")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    for dir in &platforms {
        assert_eq!(names(dir), names(&platforms[0]), "{}", dir.display());
        // Only the numbers may differ: the text that is not a number is the same.
        for name in names(dir) {
            let strip = |p: &Path| -> String {
                std::fs::read_to_string(p.join(&name))
                    .expect("a golden")
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .collect()
            };
            assert_eq!(strip(dir), strip(&platforms[0]), "{} {name}", dir.display());
        }
    }
}
