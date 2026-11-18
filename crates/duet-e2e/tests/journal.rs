//! `duet journal show | check`, on a project's journal, its index and segments, as duet writes
//! it and as a merge, a crash or a newer duet leaves it: the check lists what this duet keeps
//! unread, what replay skips and what is missing, and exits 1 only when something is wrong.

use std::path::{Path, PathBuf};

use duet_e2e::{duet, ok, s, scratch};
use duet_schemas::journal::{
    AskAnswered, AskAsked, At, By, Closes, Concerns, Entry, Event, Id, IndexHead, IndexLine,
    JournalHead, Opens, SegmentStamp,
};
use duet_schemas::{Journal, JournalIndex, Tool};

fn id(n: u8) -> Id {
    Id::from_parts(1_795_446_245_000 + u64::from(n), [n; 10])
}

fn stamp(n: u64) -> SegmentStamp {
    SegmentStamp::new(At::from_unix_ms(1_795_446_245_000 + 86_400_000 * n))
}

fn entry(n: u8, who: &str, event: impl Into<Event>) -> String {
    let at = At::from_unix_ms(1_795_446_245_000 + 1000 * u64::from(n));
    Entry::new(id(n), at, By::new(who, "cli"), event)
        .line()
        .unwrap()
}

/// A project whose journal has one segment per item of `segments`, each holding those lines,
/// and an index that opens each and closes each before the last. Segments named in `absent` are
/// in the index and not on disk.
fn project(dir: &Path, segments: &[&[String]], absent: &[usize]) -> PathBuf {
    let tool = Tool::new("duet", "0.3.0");
    let mut index = JournalIndex::head_line(&tool, &IndexHead::new("wayfaring", id(200))).unwrap();
    for (n, lines) in segments.iter().enumerate() {
        let (stamp, seg_id) = (stamp(n as u64), id(100 + n as u8));
        let closes = n.checked_sub(1).map(|p| {
            let before = segments[p];
            Closes::new(
                self::stamp(p as u64),
                stamp.at(),
                before.len() as u64,
                id(1),
                Concerns::new(),
            )
        });
        index.push_str(
            &IndexLine::new(Opens::new(stamp, seg_id.clone(), stamp.at()), closes)
                .line()
                .unwrap(),
        );
        if absent.contains(&n) {
            continue;
        }
        let mut text =
            Journal::head_line(&tool, &JournalHead::new("wayfaring", seg_id, stamp)).unwrap();
        for line in *lines {
            text.push_str(line);
        }
        std::fs::write(dir.join(stamp.file_name()), text).unwrap();
    }
    std::fs::write(dir.join("journal.duet.jsonl"), index).unwrap();
    dir.to_path_buf()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_clean_journal_across_two_segments_checks_clean_and_shows_every_entry() {
    let dir = scratch("journal-clean");
    project(
        &dir,
        &[
            &[entry(1, "claude", AskAsked::new("Which ending?"))],
            &[entry(2, "owner", AskAnswered::new(id(1), "two"))],
        ],
        &[],
    );
    let out = duet(&dir, &["journal", "check", s(&dir)]);
    ok(&out, "journal check");
    assert_eq!(
        stdout(&out),
        "journal of wayfaring: 2 segments, 2 entries, 0 asks open\nnothing wrong\n"
    );
    let out = duet(&dir, &["journal", "show", s(&dir)]);
    ok(&out, "journal show");
    let shown = stdout(&out);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), 2, "{shown}");
    assert!(
        lines[0].starts_with(&format!(
            "{}    1  2026-11-23T15:04:06.000Z  claude/cli  ask.asked 1.0  {{\"question\":",
            stamp(0)
        )),
        "{shown}"
    );
    assert!(
        lines[1].starts_with(&format!("{}    1  ", stamp(1))),
        "{shown}"
    );
}

#[test]
fn it_reads_the_working_directory_or_the_index_named() {
    let dir = scratch("journal-default");
    project(
        &dir,
        &[&[entry(1, "claude", AskAsked::new("Which ending?"))]],
        &[],
    );
    for args in [
        vec!["journal", "check"],
        vec!["journal", "check", "journal.duet.jsonl"],
    ] {
        let out = duet(&dir, &args);
        ok(&out, &format!("{args:?}"));
        assert!(
            stdout(&out).contains("1 segment, 1 entry, 1 ask open"),
            "{}",
            stdout(&out)
        );
    }
    let empty = scratch("journal-none");
    let out = duet(&empty, &["journal", "check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no journal here"), "{}", stderr(&out));
}

#[test]
fn entries_this_duet_does_not_read_are_listed_and_are_not_wrong() {
    let dir = scratch("journal-unread");
    let unknown = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"app"},"event":"lyric.sung","v":"1.0","body":{"z":1}}"#;
    let newer = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"mcp"},"event":"ask.asked","v":"1.7","body":{"question":"Louder?","urgency":"high"}}"#;
    project(
        &dir,
        &[
            &[
                entry(1, "claude", AskAsked::new("Which ending?")),
                format!("{unknown}\n"),
            ],
            &[format!("{newer}\n")],
        ],
        &[],
    );
    let out = duet(&dir, &["journal", "check"]);
    ok(&out, "journal check");
    let shown = stdout(&out);
    assert!(
        shown.contains("2 entries of a kind this duet does not read, kept as they are:"),
        "{shown}"
    );
    assert!(
        shown.contains(&format!(
            "{} seq 2 (0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6): lyric.sung 1.0, an event kind this \
             duet does not know",
            stamp(0)
        )),
        "{shown}"
    );
    assert!(
        shown.contains(&format!(
            "{} seq 1 (0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7): ask.asked 1.7, newer than the 1.0 \
             this duet reads",
            stamp(1)
        )),
        "{shown}"
    );
    assert!(shown.ends_with("nothing wrong\n"), "{shown}");
    let out = duet(&dir, &["journal", "show"]);
    ok(&out, "journal show");
    assert!(
        stdout(&out).contains("lyric.sung 1.0  {\"z\":1}  (kept, not read: an event kind"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a_second_answer_in_a_later_segment_is_listed_and_the_check_exits_1() {
    let dir = scratch("journal-two-answers");
    project(
        &dir,
        &[
            &[
                entry(1, "claude", AskAsked::new("Which ending?")),
                entry(2, "owner", AskAnswered::new(id(1), "one")),
            ],
            &[entry(3, "owner", AskAnswered::new(id(1), "two"))],
        ],
        &[],
    );
    let out = duet(&dir, &["journal", "check"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(
        shown.contains(&format!(
            "1 transition skipped on replay, the first in order standing:\n  {} seq 1 ({}): \
             ask.answered of ask {}, refused already-answered; {} seq 2 stands",
            stamp(1),
            id(3),
            id(1),
            stamp(0)
        )),
        "{shown}"
    );
    assert!(shown.ends_with("1 thing wrong\n"), "{shown}");
    assert!(
        stderr(&out).contains("the check found something wrong"),
        "{}",
        stderr(&out)
    );
    let out = duet(&dir, &["journal", "show"]);
    ok(&out, "journal show");
    let shown = stdout(&out);
    assert!(
        shown
            .lines()
            .nth(2)
            .unwrap()
            .ends_with("(skipped on replay: already-answered)"),
        "{shown}"
    );
    assert!(
        !shown.lines().nth(1).unwrap().contains("skipped"),
        "{shown}"
    );
}

#[test]
fn a_segment_the_index_names_and_is_missing_is_wrong() {
    let dir = scratch("journal-missing");
    project(
        &dir,
        &[
            &[entry(1, "claude", AskAsked::new("Which ending?"))],
            &[entry(2, "owner", AskAnswered::new(id(1), "two"))],
        ],
        &[0],
    );
    let out = duet(&dir, &["journal", "check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains(&format!(
            "the index names segment {}, and {} is missing",
            stamp(0),
            stamp(0).file_name()
        )),
        "{}",
        stdout(&out)
    );
}

#[test]
fn an_unknown_field_in_a_known_event_is_refused_by_name() {
    let dir = scratch("journal-unknown-field");
    let line = format!(
        r#"{{"id":"{}","at":"2026-11-17T15:04:05.123Z","by":{{"who":"owner","face":"cli"}},"event":"ask.answered","v":"1.0","body":{{"ask":"{}","answer":"yes","mood":"glad"}}}}"#,
        id(2),
        id(1)
    );
    project(
        &dir,
        &[&[
            entry(1, "claude", AskAsked::new("Which ending?")),
            format!("{line}\n"),
        ]],
        &[],
    );
    for command in ["check", "show"] {
        let out = duet(&dir, &["journal", command]);
        assert_eq!(out.status.code(), Some(1), "journal {command}");
        let err = stderr(&out);
        assert!(
            err.contains(&stamp(0).file_name())
                && err.contains("line 3")
                && err.contains("ask.answered 1.0")
                && err.contains("`mood`"),
            "journal {command}: {err}"
        );
    }
}
