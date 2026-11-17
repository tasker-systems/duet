//! The witnesses of the bus design's build task 1, on the journal writer and reader.
//!
//! - A journal round-trips exactly, and every line the writer writes validates against the
//!   committed schemas: the journal's `$defs/Record`, and its event's own body schema.
//! - Two processes appending 1 000 events each, at once, leave 2 001 whole lines, every `id`
//!   unique.
//! - A file cut mid-line is cut back to the last newline by the next append, and nothing before
//!   it changes.
//! - An unknown event kind and a newer event minor stay byte for byte as they were after later
//!   appends; the check lists them; an unknown *field* in a known event is refused by name.
//! - A union merge of two appended clones reads, and the check reports nothing wrong.
//! - Two answers to one ask (a merge, written by hand) replay: the first in file order stands,
//!   and the check lists the second.
//! - The ask's check and its append are one step: a writer whose view is stale is still refused
//!   `already-answered`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use duet_bus::ask::{AskState, Refusal};
use duet_bus::{Cursor, Error, Polled, Replay, Report, Tail, Writer, after};
use duet_schemas::journal::{
    AskAnswered, AskAsked, AskWithdrawn, At, Audience, Body, By, DocDigest, DocWritten, Entry, Id,
    MomentPlayed, Skip, TakeAdded, VerdictGiven,
};
use duet_schemas::{Journal, JournalHead, Tool, Version};
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "duet-bus-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn tool() -> Tool {
    Tool::new("duet", "0.3.0")
}

fn head() -> JournalHead {
    JournalHead::new("wayfaring", duet_bus::fresh_id().unwrap())
}

fn writer(root: &Path, who: &str) -> Writer {
    Writer::of_project(root, tool(), head(), By::new(who, "cli"))
}

fn text(root: &Path) -> String {
    fs::read_to_string(duet_bus::journal_path(root)).unwrap()
}

fn ask(w: &mut Writer, question: &str) -> Id {
    w.append(AskAsked::new(question), None).unwrap().id
}

/// Every event kind, through the writer.
fn every_event(w: &mut Writer) {
    let mut asked = AskAsked::new("Which ending?");
    asked.about = Some("wayfaring@2.15-3.58".into());
    asked.options = vec!["one".into(), "two".into()];
    let a = w.append(asked, None).unwrap().id;
    let mut answered = AskAnswered::new(a.clone(), "two");
    answered.said = Some("a new word".into());
    let answer = w.append(answered, Some(a.clone())).unwrap().id;
    let b = ask(w, "Again?");
    w.append(AskWithdrawn::new(b), None).unwrap();
    w.append(
        MomentPlayed::new("wayfaring@2.15-3.58", Audience::Both, "melody"),
        None,
    )
    .unwrap();
    let mut verdict = VerdictGiven::new("wayfaring@2.15-3.58", "no audible difference");
    verdict.ask = Some(a);
    w.append(verdict, Some(answer)).unwrap();
    let digest = "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34"
        .parse()
        .unwrap();
    w.append(TakeAdded::new("wayfaring", digest), None).unwrap();
    w.append(
        DocWritten::new(
            "comps/wayfaring.duet".parse().unwrap(),
            "capture-analysis",
            Version::new(1, 13),
            DocDigest::of(b"{}"),
            true,
        ),
        None,
    )
    .unwrap();
}

// --- Validation against the committed schemas ---

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../duet-schemas/schemas")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn validator(schema: &Value, more: &[&Value]) -> jsonschema::Validator {
    let envelope = read_json(&schemas_dir().join("envelope/1.0.json"));
    let mut registry = jsonschema::Registry::new();
    for s in std::iter::once(&envelope).chain(more.iter().copied()) {
        registry = registry.add(s["$id"].as_str().unwrap(), s).unwrap();
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .unwrap()
}

fn assert_valid(v: &jsonschema::Validator, value: &Value, what: &str) {
    let errors: Vec<String> = v.iter_errors(value).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{what}: {}", errors.join("; "));
}

// --- Round trip ---

#[test]
fn a_journal_round_trips_exactly_and_every_event_validates_against_its_schema() {
    let root = scratch("roundtrip");
    let mut w = writer(&root, "claude");
    every_event(&mut w);
    let written = text(&root);
    let journal = Journal::read(&written).unwrap();
    assert_eq!(journal.lines.len(), 8);
    assert!(journal.lines.iter().all(|l| l.read().is_some()));
    assert_eq!(journal.write().unwrap(), written, "read, then written");

    let stream = read_json(&schemas_dir().join("journal/1.0.json"));
    let id = stream["$id"].as_str().unwrap();
    let head = validator(&stream, &[]);
    let record = validator(
        &json!({ "$ref": format!("{id}#/$defs/Record") }),
        &[&stream],
    );
    let mut lines = written.lines();
    assert_valid(
        &head,
        &serde_json::from_str(lines.next().unwrap()).unwrap(),
        "the head",
    );
    let mut kinds = HashSet::new();
    for (n, line) in lines.enumerate() {
        let entry: Value = serde_json::from_str(line).unwrap();
        assert_valid(&record, &entry, &format!("line {}", n + 2));
        let (event, v) = (
            entry["event"].as_str().unwrap(),
            entry["v"].as_str().unwrap(),
        );
        let body = read_json(&schemas_dir().join(format!("journal/events/{event}/{v}.json")));
        assert_valid(
            &validator(&body, &[]),
            &entry["body"],
            &format!("{event} {v}"),
        );
        kinds.insert(event.to_string());
    }
    let known: HashSet<String> = duet_schemas::EVENTS
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    assert_eq!(kinds, known, "every event kind is written once at least");
}

// --- Two processes at once ---

const CHILD: &str = "DUET_BUS_TEST_CHILD";
const PER_CHILD: usize = 1000;

/// Not a test of its own: the body of each child process of the next test. Run with `CHILD`
/// set to a project root and a name, it appends `PER_CHILD` events there.
#[test]
fn child_appends() {
    let Ok(spec) = std::env::var(CHILD) else {
        return;
    };
    let (root, who) = spec.split_once('|').unwrap();
    // Both children start together: each waits for the parent's `go`.
    let go = Path::new(root).join("go");
    let start = std::time::Instant::now();
    while !go.exists() {
        assert!(start.elapsed().as_secs() < 60, "no go");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut w = writer(Path::new(root), who);
    for n in 0..PER_CHILD {
        w.append(VerdictGiven::new(format!("{who}-{n}"), "heard"), None)
            .unwrap();
    }
}

#[test]
fn two_processes_appending_a_thousand_each_leave_whole_lines_and_unique_ids() {
    let root = scratch("concurrent");
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = ["one", "two"]
        .iter()
        .map(|who| {
            Command::new(&exe)
                .args(["--exact", "child_appends", "--test-threads", "1"])
                .env(CHILD, format!("{}|{who}", root.display()))
                .spawn()
                .unwrap()
        })
        .collect();
    fs::write(root.join("go"), "").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let written = text(&root);
    assert!(written.ends_with('\n'));
    assert_eq!(written.lines().count(), 2 * PER_CHILD + 1, "2 001 lines");
    let journal = Journal::read(&written).unwrap();
    assert!(journal.torn.is_none());
    let ids: HashSet<&Id> = journal.lines.iter().map(|l| &l.id).collect();
    assert_eq!(ids.len(), 2 * PER_CHILD, "every id unique");
    for who in ["one", "two"] {
        let mine: Vec<String> = journal
            .lines
            .iter()
            .filter(|l| l.by.who == who)
            .map(|l| serde_json::from_str::<Value>(&l.text).unwrap()["body"]["about"].to_string())
            .collect();
        let want: Vec<String> = (0..PER_CHILD).map(|n| format!("\"{who}-{n}\"")).collect();
        assert_eq!(mine, want, "{who}'s appends, each once, in its order");
    }
    let turns = journal
        .lines
        .windows(2)
        .filter(|w| w[0].by.who != w[1].by.who)
        .count();
    assert!(
        turns > 1,
        "the two appended at once, their lines interleaved ({turns} turns)"
    );
    assert!(!Report::of(&journal).is_wrong());
}

// --- A torn last line ---

#[test]
fn a_file_cut_mid_line_is_cut_back_by_the_next_append_and_nothing_before_it_changes() {
    let root = scratch("torn");
    let mut w = writer(&root, "claude");
    every_event(&mut w);
    let before = text(&root);
    let path = duet_bus::journal_path(&root);
    let torn = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04"#;
    fs::write(&path, format!("{before}{torn}")).unwrap();

    let journal = Journal::read(&text(&root)).unwrap();
    assert_eq!(
        journal.torn.as_deref(),
        Some(torn),
        "a reader keeps it apart"
    );
    assert_eq!(journal.lines.len(), 8);
    let report = Report::of(&journal);
    assert!(!report.is_wrong() && report.torn == Some(torn.len()));

    let mut next = writer(&root, "owner");
    let appended = next.append(AskAsked::new("Now?"), None).unwrap();
    let cut = appended.cut.expect("the append says it cut");
    assert_eq!(cut.dropped, torn);
    assert_eq!(cut.to, before.len() as u64);
    let after = text(&root);
    assert!(after.starts_with(&before), "nothing before the cut changes");
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    assert_eq!(appended.seq, 9);
    assert!(Journal::read(&after).unwrap().torn.is_none());
}

#[test]
fn a_torn_head_is_cut_and_the_journal_started_again() {
    let root = scratch("torn-head");
    let path = duet_bus::journal_path(&root);
    fs::write(
        &path,
        r#"{"$schema":"https://raw.githubusercontent.com/tas"#,
    )
    .unwrap();
    let appended = writer(&root, "claude")
        .append(AskAsked::new("Begin?"), None)
        .unwrap();
    assert!(appended.started && appended.cut.is_some() && appended.seq == 1);
    assert_eq!(Journal::read(&text(&root)).unwrap().lines.len(), 1);
}

// --- What this build does not read ---

/// A line of an event kind this build does not know, with its keys out of order and spaces
/// a writer of today would not write: kept byte for byte.
const UNKNOWN: &str = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"app"},"event":"lyric.sung","v":"1.0","body":{ "z": 1, "a": [2, 3] }}"#;

/// `ask.asked` at a newer minor, with a field 1.0 does not hold: kept byte for byte.
const NEWER: &str = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"mcp"},"event":"ask.asked","v":"1.7","body":{"question":"Louder?","urgency":"high"}}"#;

#[test]
fn unknown_events_and_newer_minors_stay_byte_for_byte_and_the_check_lists_them() {
    let root = scratch("unknown");
    let mut w = writer(&root, "claude");
    ask(&mut w, "First?");
    let path = duet_bus::journal_path(&root);
    let mut file = text(&root);
    file.push_str(&format!("{UNKNOWN}\n{NEWER}\n"));
    fs::write(&path, &file).unwrap();
    let kept = file.clone();

    let mut later = writer(&root, "owner");
    every_event(&mut later);
    let after = text(&root);
    assert!(after.starts_with(&kept), "both lines stay as they were");
    assert!(after.contains(&format!("\n{UNKNOWN}\n{NEWER}\n")));

    let journal = Journal::read(&after).unwrap();
    assert_eq!(journal.write().unwrap(), after, "read, then written");
    let skipped: Vec<(&str, &Body)> = journal
        .lines
        .iter()
        .filter(|l| l.read().is_none())
        .map(|l| (l.event.as_str(), &l.body))
        .collect();
    assert_eq!(
        skipped,
        [
            ("lyric.sung", &Body::Skipped(Skip::UnknownEvent)),
            (
                "ask.asked",
                &Body::Skipped(Skip::NewerMinor {
                    reads: Version::new(1, 0)
                })
            ),
        ]
    );
    let report = Report::of(&journal);
    assert!(!report.is_wrong(), "{report}");
    assert_eq!(
        report.unread.iter().map(|u| u.seq).collect::<Vec<_>>(),
        [2, 3]
    );
    let shown = report.to_string();
    assert!(
        shown.contains("lyric.sung 1.0, an event kind this duet does not know"),
        "{shown}"
    );
    assert!(
        shown.contains("ask.asked 1.7, newer than the 1.0 this duet reads"),
        "{shown}"
    );
}

#[test]
fn an_unknown_field_in_a_known_event_is_refused_by_name() {
    let root = scratch("unknown-field");
    let mut w = writer(&root, "claude");
    let a = ask(&mut w, "First?");
    let line = format!(
        r#"{{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{{"who":"owner","face":"cli"}},"event":"ask.answered","v":"1.0","body":{{"ask":"{a}","answer":"yes","mood":"glad"}}}}"#
    );
    let path = duet_bus::journal_path(&root);
    fs::write(&path, format!("{}{line}\n", text(&root))).unwrap();
    let err = Journal::read(&text(&root)).unwrap_err().to_string();
    assert!(err.contains("line 3") && err.contains("`mood`"), "{err}");
    assert!(err.contains("ask.answered 1.0"), "{err}");
    // A writer will not append to what it cannot read, and says why.
    let err = writer(&root, "owner")
        .append(AskAsked::new("Next?"), None)
        .unwrap_err();
    let err = chain(&err);
    assert!(err.contains("`mood`"), "{err}");
    // In the frame too.
    let frame = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"cli"},"event":"ask.asked","v":"1.0","seq":4,"body":{"question":"?"}}"#;
    let err = duet_schemas::journal::read_line(frame)
        .unwrap_err()
        .to_string();
    assert!(err.contains("`seq`"), "{err}");
}

/// An error and every source under it, as `{:#}` prints a chain.
fn chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(e) = source {
        text.push_str(&format!(": {e}"));
        source = e.source();
    }
    text
}

// --- Union merge ---

#[test]
fn a_union_merge_of_two_appended_clones_reads_and_the_check_reports_nothing_wrong() {
    let base_root = scratch("union-base");
    let mut w = writer(&base_root, "claude");
    let first = ask(&mut w, "Which ending?");
    let second = ask(&mut w, "Which tempo?");
    let base = text(&base_root);

    // Two clones of the base, each appended by its own duet.
    let (a, b) = (scratch("union-a"), scratch("union-b"));
    for clone in [&a, &b] {
        fs::write(duet_bus::journal_path(clone), &base).unwrap();
    }
    let mut on_a = writer(&a, "owner");
    on_a.append(AskAnswered::new(first.clone(), "two"), None)
        .unwrap();
    let third = ask(&mut on_a, "And the bridge?");
    on_a.append(AskAnswered::new(third, "keep it"), None)
        .unwrap();
    let mut on_b = writer(&b, "claude");
    on_b.append(AskWithdrawn::new(second.clone()), None)
        .unwrap();
    on_b.append(
        MomentPlayed::new("wayfaring@2.15-3.58", Audience::Owner, "take"),
        None,
    )
    .unwrap();

    // git's union driver: the common lines once, then ours, then theirs.
    let (ta, tb) = (text(&a), text(&b));
    let merged = format!("{base}{}{}", &ta[base.len()..], &tb[base.len()..]);
    let journal = Journal::read(&merged).unwrap();
    assert_eq!(journal.lines.len(), 2 + 3 + 2);
    let report = Report::of(&journal);
    assert!(!report.is_wrong(), "{report}");
    assert!(report.unread.is_empty() && report.duplicates.is_empty());
    assert!(report.to_string().ends_with("nothing wrong"));
    let replay = Replay::of(&journal.lines);
    assert_eq!(replay.ask(&first).unwrap().state, AskState::Answered);
    assert_eq!(replay.ask(&second).unwrap().state, AskState::Withdrawn);
    assert!(replay.open().is_empty());

    // A writer of clone A, which read A as it was, folds the merge again before appending.
    fs::write(duet_bus::journal_path(&a), &merged).unwrap();
    let appended = on_a
        .append(AskAsked::new("After the merge?"), None)
        .unwrap();
    assert_eq!(appended.seq, 8);
    let err = on_a.append(AskWithdrawn::new(second), None).unwrap_err();
    assert!(
        matches!(
            err,
            Error::Refused {
                refusal: Refusal::AlreadyWithdrawn,
                ..
            }
        ),
        "{err}"
    );

    // A subscriber at a cursor from before the merge is sent a resync, not "up to date".
    let theirs = Journal::read(&tb).unwrap();
    let cursor = Cursor::new(4, theirs.lines[3].id.clone());
    assert!(matches!(after(&journal, &cursor), Polled::Resync(all) if all.len() == 7));
}

// --- Two answers to one ask ---

#[test]
fn two_answers_to_one_ask_replay_the_first_standing_and_the_check_lists_the_second() {
    let root = scratch("two-answers");
    let mut w = writer(&root, "claude");
    let a = ask(&mut w, "Which ending?");
    // The merge, by hand: a second answer to the same ask, as another clone wrote it.
    let by = By::new("owner", "cli");
    let at = At::from_unix_ms(1_795_446_245_000);
    let one = Entry::new(
        duet_bus::fresh_id().unwrap(),
        at,
        by.clone(),
        AskAnswered::new(a.clone(), "one"),
    );
    let two = Entry::new(
        duet_bus::fresh_id().unwrap(),
        at,
        by,
        AskAnswered::new(a.clone(), "two"),
    );
    let path = duet_bus::journal_path(&root);
    fs::write(
        &path,
        format!(
            "{}{}{}",
            text(&root),
            one.line().unwrap(),
            two.line().unwrap()
        ),
    )
    .unwrap();

    let journal = Journal::read(&text(&root)).unwrap();
    let replay = Replay::of(&journal.lines);
    let record = replay.ask(&a).unwrap();
    assert_eq!(
        (record.state, record.settled),
        (AskState::Answered, Some(2))
    );
    let answer = journal.lines[1].read().unwrap();
    assert!(matches!(answer, duet_schemas::journal::Event::AskAnswered(x) if x.answer == "one"));

    let report = Report::of(&journal);
    assert!(report.is_wrong());
    assert_eq!(report.refused.len(), 1);
    let refused = &report.refused[0];
    assert_eq!((refused.seq, refused.stands), (3, Some(2)));
    assert_eq!(refused.refusal, Refusal::AlreadyAnswered);
    assert_eq!(refused.id, two.id);
    let shown = report.to_string();
    assert!(
        shown.contains(&format!(
            "seq 3 ({}): ask.answered of ask {a}, refused already-answered; seq 2 stands",
            two.id
        )),
        "{shown}"
    );
}

// --- The check and the append are one step ---

#[test]
fn a_writer_with_a_stale_view_is_refused_already_answered() {
    let root = scratch("check-and-append");
    let mut asker = writer(&root, "claude");
    let a = ask(&mut asker, "Which ending?");
    let mut first = writer(&root, "owner");
    let mut second = writer(&root, "owner");
    // Both have read the ask open.
    first
        .append(MomentPlayed::new("x", Audience::Owner, "take"), None)
        .unwrap();
    second
        .append(MomentPlayed::new("x", Audience::Owner, "take"), None)
        .unwrap();
    assert!(second.replay().open().len() == 1);

    first
        .append(AskAnswered::new(a.clone(), "one"), None)
        .unwrap();
    let before = text(&root);
    let err = second.append(AskAnswered::new(a, "two"), None).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::Refused {
                refusal: Refusal::AlreadyAnswered,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("already-answered"), "{err}");
    assert_eq!(text(&root), before, "nothing appended");
    assert!(!Report::of(&Journal::read(&before).unwrap()).is_wrong());
}

// --- The tail ---

#[test]
fn a_tail_reads_from_its_cursor_and_never_a_torn_line() {
    let root = scratch("tail");
    let mut w = writer(&root, "claude");
    ask(&mut w, "One?");
    ask(&mut w, "Two?");
    let path = duet_bus::journal_path(&root);
    let (mut tail, now) = Tail::open(&path, None).unwrap();
    let Polled::After(all) = now else {
        panic!("from the start")
    };
    assert_eq!(all.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(tail.poll().unwrap(), Polled::After(Vec::new()));

    ask(&mut w, "Three?");
    fs::write(&path, format!("{}{{\"id\":\"01", text(&root))).unwrap();
    let Polled::After(new) = tail.poll().unwrap() else {
        panic!("no resync")
    };
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].0, 3);
    let cursor = tail.cursor().clone();
    assert_eq!(cursor.seq, 3);

    // The next append cuts the torn line; the tail reads on from its cursor.
    ask(&mut w, "Four?");
    let Polled::After(new) = tail.poll().unwrap() else {
        panic!("no resync")
    };
    assert_eq!(new.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [4]);

    // Reopened from a cursor, it reads what came after.
    let (_, from) = Tail::open(&path, Some(cursor)).unwrap();
    assert!(matches!(from, Polled::After(v) if v.len() == 1 && v[0].0 == 4));

    // A journal rewritten under it is a resync.
    let other = scratch("tail-other");
    let mut o = writer(&other, "claude");
    ask(&mut o, "Elsewhere?");
    fs::copy(duet_bus::journal_path(&other), &path).unwrap();
    assert!(matches!(tail.poll().unwrap(), Polled::Resync(v) if v.len() == 1));
}
