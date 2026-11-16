//! `duet record`: what it refuses before any audio device is opened. Recording itself needs a
//! microphone, which CI has none of; the file it writes is `duet-capture`'s `TakeWriter`,
//! tested there, and captured end to end in `capture.rs`.

use duet_e2e::{duet, s, scratch};

#[test]
fn a_take_never_replaces_a_file() {
    let dir = scratch("record-exists");
    let take = dir.join("take.flac");
    std::fs::write(&take, b"an earlier take").expect("write");
    let out = duet(&dir, &["record", s(&take), "--seconds", "1"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("exists"), "{err}");
    assert_eq!(std::fs::read(&take).expect("read"), b"an earlier take");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_take_is_named_flac() {
    let dir = scratch("record-name");
    let out = duet(
        &dir,
        &["record", s(&dir.join("take.wav")), "--seconds", "1"],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("take.flac"), "{err}");
    assert!(!dir.join("take.wav").exists() && !dir.join("take.flac").exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_take_goes_in_a_directory_that_exists() {
    let dir = scratch("record-dir");
    let take = dir.join("missing").join("take.flac");
    let out = duet(&dir, &["record", s(&take), "--seconds", "1"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not a directory"), "{err}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn seconds_are_above_zero() {
    let dir = scratch("record-seconds");
    let out = duet(
        &dir,
        &["record", s(&dir.join("take.flac")), "--seconds", "0"],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("above zero"), "{err}");
    assert!(!dir.join("take.flac").exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}
