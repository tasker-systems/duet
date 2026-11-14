//! `our-abc-reading-agrees-with-abc2midi`: our reading of every corpus tune, and of the ABC
//! our writer makes from it, against abc2midi's.
//!
//! **In CI** (plain `cargo test`), the first two tests compare against the committed
//! recordings of abc2midi's reading. abc2midi never runs in CI.
//!
//! **Locally**, `cargo test -p duet-abc --features oracle` also runs abc2midi, as a
//! separate program, on every tune and on our writer's ABC for it, and fails when its
//! fresh reading differs from the recording, when its version is not the manifest's, or
//! when it prints an `Error` or `Warning` line. Re-recording is its own explicit command:
//!
//! ```text
//! DUET_ABC_RECORD=1 cargo test -p duet-abc --features oracle recordings_match_live_abc2midi
//! ```
//!
//! and its diff is reviewed. When our reading disagrees with a recording, re-recording is
//! never the fix: either our reading of the standard is wrong and is fixed against the
//! standard, or abc2midi differs from the standard and the difference is listed in the
//! manifest with its reason.

mod support;

use std::collections::BTreeSet;

use support::{
    Difference, N, apply_differences, compare, manifest, mapped_back, notes_of, our_song,
    our_written, read_file, recording,
};

/// Every corpus tune has its three recorded files, and every tune in the corpus files is
/// in the manifest.
fn every_tune_is_recorded() {
    let m = manifest();
    let mut listed = BTreeSet::new();
    for entry in &m.tune {
        listed.insert((entry.file.clone(), entry.x));
        let mut files = vec![format!("{}.abc2midi.txt", entry.name)];
        if entry.writer_refuses.is_none() {
            files.push(format!("{}.written.abc", entry.name));
            files.push(format!("{}.written.abc2midi.txt", entry.name));
        }
        for file in files {
            assert!(
                support::corpus().join(&file).exists(),
                "{}: {file} is missing; a corpus tune needs a local oracle run to record it",
                entry.name
            );
        }
    }
    // Every recording belongs to a tune the manifest lists, so a stale one cannot linger.
    let recorded: BTreeSet<String> = m
        .tune
        .iter()
        .flat_map(|e| {
            [
                format!("{}.abc2midi.txt", e.name),
                format!("{}.written.abc", e.name),
                format!("{}.written.abc2midi.txt", e.name),
            ]
        })
        .collect();
    for dir_entry in std::fs::read_dir(support::corpus()).expect("corpus dir") {
        let path = dir_entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        if name.ends_with(".abc2midi.txt") || name.ends_with(".written.abc") {
            assert!(
                recorded.contains(&name),
                "{name} is a recording of no tune in the manifest; remove it"
            );
            continue;
        }
        if !name.ends_with(".abc") {
            continue;
        }
        let book = duet_abc::Book::parse(&read_file(&name)).expect("corpus file parses");
        for tune in book.tunes() {
            assert!(
                listed.contains(&(name.clone(), tune.x())),
                "{name} X:{} is not in the manifest, so it has no recording",
                tune.x()
            );
        }
    }
}

/// The checks the comparison itself must pass, on made-up notes: a listed difference that
/// no longer occurs fails, and so does a recording without abc2midi's one-tick onset.
fn the_comparison_fails_where_it_must() {
    let song = duet_core::Song::from_text("[track \"P\" ch1]\n1.1.0 C4 1 95\n").expect("song");
    let ours = notes_of(&song);
    let stale = Difference {
        what: "velocity".into(),
        notes: "all".into(),
        ours: Some(toml::Value::Integer(80)),
        theirs: Some(toml::Value::Integer(95)),
        reason: "made up".into(),
    };
    assert!(
        apply_differences(&song, &ours, &[stale]).is_err(),
        "a listed difference that no longer occurs must fail"
    );
    let on_time: Vec<N> = vec![(0, 480, 60, 1, 95)];
    assert!(
        mapped_back(&on_time).is_err(),
        "a recording whose notes start on time has lost the one-tick onset"
    );

    // The tempo map and programs are compared too, not just the notes.
    let ours = duet_core::Song::from_text(
        "tempo 168\ntempo 80 at 2.1.0\n\n[track \"P\" ch1 prog73]\n1.1.0 C4 1 95\n",
    )
    .expect("song");
    let theirs = |header: &str, prog: u8| {
        duet_core::Song::from_text(&format!(
            "{header}\n\n[track \"P\" ch1]\n1.1.1 prog {prog}\n1.1.1 C4 0.998 95\n"
        ))
        .expect("recording")
    };
    let agrees = theirs("tempo 168.0004\ntempo 80 at 2.1.1", 73);
    assert!(
        compare(&ours, &agrees, &[]).is_ok(),
        "{:?}",
        compare(&ours, &agrees, &[])
    );
    for (wrong, what) in [
        (
            theirs("tempo 56\ntempo 80 at 2.1.1", 73),
            "a different tempo",
        ),
        (
            theirs("tempo 168\ntempo 80 at 2.2.1", 73),
            "a tempo change elsewhere",
        ),
        (
            theirs("tempo 168\ntempo 80 at 2.1.1", 74),
            "a different program",
        ),
    ] {
        assert!(compare(&ours, &wrong, &[]).is_err(), "{what} must fail");
    }
}

#[test]
fn our_abc_reading_agrees_with_recorded_abc2midi() {
    the_comparison_fails_where_it_must();
    every_tune_is_recorded();
    let m = manifest();
    let mut failures = Vec::new();
    let mut compared = 0;
    for entry in &m.tune {
        let ours = our_song(entry);
        let rec = match recording(&format!("{}.abc2midi.txt", entry.name)) {
            Ok(r) => r,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        match compare(&ours, &rec, &entry.difference) {
            Ok(n) => compared += n,
            Err(e) => failures.push(format!("{}: {e}", entry.name)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(compared > 0, "the corpus compared no notes");
}

#[test]
fn our_written_abc_agrees_with_recorded_abc2midi() {
    every_tune_is_recorded();
    let m = manifest();
    let mut failures = Vec::new();
    for entry in &m.tune {
        if let Some(reason) = &entry.writer_refuses {
            // A tune listed as one the writer refuses must still be refused; when it is
            // not, it needs its written files recorded.
            let written = duet_abc::write(&our_song(entry), &duet_abc::WriteOptions::default());
            assert!(
                matches!(&written, Err(duet_abc::Error::Unwritable { why, .. }) if why.contains("triplet")),
                "{}: the manifest says the writer refuses it ({reason}), but it wrote it; \
                 record its written files and drop writer_refuses",
                entry.name
            );
            continue;
        }
        // The writer still makes exactly the ABC its recording was made from.
        let written = our_written(entry);
        let committed = read_file(&format!("{}.written.abc", entry.name));
        if written != committed {
            failures.push(format!(
                "{}: our writer's ABC differs from {}.written.abc, which its recording was \
                 made from; a writer change needs a local oracle run to re-record.\n--- \
                 committed\n{committed}--- now\n{written}",
                entry.name, entry.name
            ));
            continue;
        }
        // abc2midi's reading of it carries the song's notes.
        let ours = our_song(entry);
        let rec = match recording(&format!("{}.written.abc2midi.txt", entry.name)) {
            Ok(r) => r,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        match compare(&ours, &rec, &entry.difference) {
            Ok(_) => {}
            Err(e) => failures.push(format!("{} (written): {e}", entry.name)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[cfg(feature = "oracle")]
mod live {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use duet_core::Song;
    use duet_midi::Midi;

    use super::support::{Manifest, corpus, manifest, our_written, read_file};

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-abc-oracle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// The text with the preamble inserted after tune `x`'s `X:` line, and again at each
    /// voice in its body: abc2midi starts each voice's playback settings afresh, so a
    /// preamble in the header reaches the first voice only. After a `V:` line it goes as
    /// `%%` lines, and after an inline `[V:]` as `[I:...]` fields.
    fn with_preamble(text: &str, x: u32, preamble: &[String]) -> String {
        let inline: String = preamble
            .iter()
            .map(|p| format!("[I:{}]", p.trim_start_matches('%')))
            .collect();
        let mut out = String::new();
        let mut state = 0; // 0 before the tune, 1 in its header, 2 in its body, 3 after it
        for line in text.lines() {
            let is_x = line
                .strip_prefix("X:")
                .and_then(|v| v.split('%').next())
                .is_some_and(|v| v.trim().parse::<u32>().ok() == Some(x));
            if state == 0 && is_x {
                state = 1;
                out.push_str(line);
                out.push('\n');
                for p in preamble {
                    out.push_str(p);
                    out.push('\n');
                }
                continue;
            }
            if state > 0 && state < 3 && line.trim().is_empty() {
                state = 3;
            }
            if state == 1 && line.starts_with("K:") {
                state = 2;
            }
            if state == 2 && line.starts_with("V:") {
                out.push_str(line);
                out.push('\n');
                for p in preamble {
                    out.push_str(p);
                    out.push('\n');
                }
                continue;
            }
            if state == 2 && line.contains("[V:") {
                let mut rest = line;
                while let Some(i) = rest.find("[V:") {
                    let close = rest[i..].find(']').map_or(rest.len(), |c| i + c + 1);
                    out.push_str(&rest[..close]);
                    out.push_str(&inline);
                    rest = &rest[close..];
                }
                out.push_str(rest);
                out.push('\n');
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        assert!(state > 0, "no X:{x} line to put the preamble after");
        out
    }

    /// Runs abc2midi on tune `x` of `text`, and returns its reading as notefmt text.
    fn abc2midi(m: &Manifest, label: &str, text: &str, x: u32) -> Result<String, String> {
        let dir = scratch();
        let abc = dir.join(format!("{label}.abc"));
        let mid = dir.join(format!("{label}.mid"));
        std::fs::write(&abc, with_preamble(text, x, &m.preamble)).expect("write tune");
        let _ = std::fs::remove_file(&mid);
        let out = Command::new("abc2midi")
            .arg(&abc)
            .arg(x.to_string())
            .arg("-o")
            .arg(&mid)
            .args(&m.arguments)
            .output()
            .map_err(|e| format!("running abc2midi: {e}"))?;
        // Every problem is printed on a line of its own, which is the first check. The
        // exit status is the second: the pinned version exits 1 after an error (older
        // ones exited 0, which is why the lines come first).
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let problems: Vec<&str> = printed
            .lines()
            .filter(|l| {
                l.starts_with("Error in line-char") || l.starts_with("Warning in line-char")
            })
            .collect();
        if !problems.is_empty() {
            return Err(format!("{label}: abc2midi says:\n{}", problems.join("\n")));
        }
        if !out.status.success() {
            return Err(format!(
                "{label}: abc2midi exited with {}:\n{printed}",
                out.status
            ));
        }
        let bytes = std::fs::read(&mid).map_err(|e| format!("{label}: no MIDI file: {e}"))?;
        let song = Song::from_midi(&bytes).map_err(|e| format!("{label}: {e}"))?;
        Ok(song.to_text())
    }

    fn version() -> String {
        let out = Command::new("abc2midi")
            .arg("-ver")
            .output()
            .expect("abc2midi is installed (brew install abcmidi)");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn check_or_record(path: &Path, fresh: &str, record: bool, failures: &mut Vec<String>) {
        if record {
            std::fs::write(path, fresh).expect("write recording");
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(committed) if committed == fresh => {}
            Ok(committed) => failures.push(format!(
                "{}: abc2midi's fresh reading differs from the recording.\n--- recorded\n\
                 {committed}--- fresh\n{fresh}",
                path.display()
            )),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    #[test]
    fn recordings_match_live_abc2midi() {
        let m = manifest();
        let record = std::env::var_os("DUET_ABC_RECORD").is_some_and(|v| v == "1");
        let have = version();
        assert_eq!(
            have, m.abc2midi_version,
            "the recordings were made with abc2midi '{}', and this is '{have}'; install that \
             version (see the README), or set abc2midi_version in the manifest to this one, \
             re-record, and review the diff",
            m.abc2midi_version
        );
        let mut failures = Vec::new();
        for entry in &m.tune {
            let text = read_file(&entry.file);
            match abc2midi(&m, &entry.name, &text, entry.x) {
                Ok(fresh) => check_or_record(
                    &corpus().join(format!("{}.abc2midi.txt", entry.name)),
                    &fresh,
                    record,
                    &mut failures,
                ),
                Err(e) => failures.push(e),
            }
            if entry.writer_refuses.is_some() {
                continue;
            }
            let written = our_written(entry);
            let written_path = corpus().join(format!("{}.written.abc", entry.name));
            if record {
                std::fs::write(&written_path, &written).expect("write written ABC");
            }
            match abc2midi(&m, &format!("{}.written", entry.name), &written, 1) {
                Ok(fresh) => check_or_record(
                    &corpus().join(format!("{}.written.abc2midi.txt", entry.name)),
                    &fresh,
                    record,
                    &mut failures,
                ),
                Err(e) => failures.push(e),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
