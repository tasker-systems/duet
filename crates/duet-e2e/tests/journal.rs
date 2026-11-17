//! `duet journal show | check`, on journals written as duet writes them and as a merge or a
//! newer duet leaves them: the check lists what this duet keeps unread, and what replay skips,
//! and exits 1 only when something is wrong.

use std::path::Path;

use duet_e2e::{duet, ok, root, s, scratch};
use duet_schemas::journal::{AskAnswered, AskAsked, At, By, Entry, Id};
use duet_schemas::{Journal, JournalHead, Tool};

fn id(n: u8) -> Id {
    Id::from_parts(1_795_446_245_000 + u64::from(n), [n; 10])
}

fn entry(n: u8, who: &str, event: impl Into<duet_schemas::journal::Event>) -> String {
    let at = At::from_unix_ms(1_795_446_245_000 + 1000 * u64::from(n));
    Entry::new(id(n), at, By::new(who, "cli"), event)
        .line()
        .unwrap()
}

/// A journal: its head, then `lines`.
fn journal(dir: &Path, lines: &[String]) -> std::path::PathBuf {
    let head = JournalHead::new("wayfaring", id(0));
    let mut text = Journal::head_line(&Tool::new("duet", "0.3.0"), &head).unwrap();
    for line in lines {
        text.push_str(line);
    }
    let path = dir.join("journal.duet.jsonl");
    std::fs::write(&path, text).unwrap();
    path
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_committed_example_checks_clean_and_shows_every_entry() {
    let dir = scratch("journal-example");
    let example = root().join("crates/duet-schemas/schemas/journal/examples/1.0.duet.jsonl");
    let out = duet(&dir, &["journal", "check", s(&example)]);
    ok(&out, "journal check");
    assert_eq!(
        stdout(&out),
        "journal 1.0 of wayfaring: 8 entries, 0 asks open\nnothing wrong\n"
    );
    let out = duet(&dir, &["journal", "show", s(&example)]);
    ok(&out, "journal show");
    let shown = stdout(&out);
    assert_eq!(shown.lines().count(), 8, "{shown}");
    assert!(
        shown.lines().next().unwrap().starts_with(
            "   1  2026-11-23T15:04:06.000Z  claude/cli  ask.asked 1.0  {\"question\":"
        ),
        "{shown}"
    );
}

#[test]
fn with_no_path_it_reads_the_journal_in_the_working_directory() {
    let dir = scratch("journal-default");
    journal(&dir, &[entry(1, "claude", AskAsked::new("Which ending?"))]);
    let out = duet(&dir, &["journal", "check"]);
    ok(&out, "journal check");
    assert!(
        stdout(&out).contains("1 entry, 1 ask open"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn entries_this_duet_does_not_read_are_listed_and_are_not_wrong() {
    let dir = scratch("journal-unread");
    let unknown = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"app"},"event":"lyric.sung","v":"1.0","body":{"z":1}}"#;
    let newer = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"mcp"},"event":"ask.asked","v":"1.7","body":{"question":"Louder?","urgency":"high"}}"#;
    let path = journal(
        &dir,
        &[
            entry(1, "claude", AskAsked::new("Which ending?")),
            format!("{unknown}\n"),
            format!("{newer}\n"),
        ],
    );
    let out = duet(&dir, &["journal", "check", s(&path)]);
    ok(&out, "journal check");
    let shown = stdout(&out);
    assert!(
        shown.contains("2 entries of a kind this duet does not read, kept as they are:"),
        "{shown}"
    );
    assert!(
        shown.contains(
            "seq 2 (0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6): lyric.sung 1.0, an event kind this \
             duet does not know"
        ),
        "{shown}"
    );
    assert!(
        shown.contains(
            "seq 3 (0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7): ask.asked 1.7, newer than the 1.0 \
             this duet reads"
        ),
        "{shown}"
    );
    assert!(shown.ends_with("nothing wrong\n"), "{shown}");
    let out = duet(&dir, &["journal", "show", s(&path)]);
    ok(&out, "journal show");
    assert!(
        stdout(&out).contains("lyric.sung 1.0  {\"z\":1}  (kept, not read: an event kind"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a_second_answer_to_one_ask_is_listed_and_the_check_exits_1() {
    let dir = scratch("journal-two-answers");
    let path = journal(
        &dir,
        &[
            entry(1, "claude", AskAsked::new("Which ending?")),
            entry(2, "owner", AskAnswered::new(id(1), "one")),
            entry(3, "owner", AskAnswered::new(id(1), "two")),
        ],
    );
    let out = duet(&dir, &["journal", "check", s(&path)]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(
        shown.contains(&format!(
            "1 transition skipped on replay, the first in file order standing:\n  seq 3 ({}): \
             ask.answered of ask {}, refused already-answered; seq 2 stands",
            id(3),
            id(1)
        )),
        "{shown}"
    );
    assert!(
        shown.contains("0 asks open") && shown.ends_with("1 thing wrong\n"),
        "{shown}"
    );
    assert!(
        stderr(&out).contains("the check found something wrong"),
        "{}",
        stderr(&out)
    );
    let out = duet(&dir, &["journal", "show", s(&path)]);
    ok(&out, "journal show");
    let shown = stdout(&out);
    let third = shown.lines().nth(2).unwrap();
    assert!(
        third.ends_with("(skipped on replay: already-answered)"),
        "{shown}"
    );
    assert!(
        !shown.lines().nth(1).unwrap().contains("skipped"),
        "{shown}"
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
    let path = journal(
        &dir,
        &[
            entry(1, "claude", AskAsked::new("Which ending?")),
            format!("{line}\n"),
        ],
    );
    for command in ["check", "show"] {
        let out = duet(&dir, &["journal", command, s(&path)]);
        assert_eq!(out.status.code(), Some(1), "journal {command}");
        let err = stderr(&out);
        assert!(
            err.contains("line 3") && err.contains("ask.answered 1.0") && err.contains("`mood`"),
            "journal {command}: {err}"
        );
    }
}
