//! The witnesses of the bus design's build task 1, on the journal's index and segments
//! (decisions 16–23):
//!
//! - The index and every segment round-trip exactly, and every event and every index line
//!   validates against its committed schema.
//! - Two processes appending 1 000 events each, at once, rolling several times, leave 2 000
//!   whole entries, every `id` unique, every named segment present, one open entry.
//! - A segment or the index cut mid-line is cut back by the next append, and nothing before the
//!   cut changes.
//! - A segment a crashed roll left with only its head is adopted by the next append.
//! - An unknown event kind and a newer event minor stay byte for byte after later appends and
//!   across a roll; the check lists them; an unknown *field* in a known event is refused by name.
//! - A union merge of two appended clones reads, and the check reports nothing wrong, whether
//!   neither, one or both rolled.
//! - Two answers to one ask in different segments: the first in order stands, the check lists
//!   the second.
//! - A cursor taken before a roll resumes after a restart; a cursor into a segment that was
//!   merged into gets resync.
//! - The ask's check and its append are one step: a writer with a stale view is refused.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use duet_bus::ask::{AskState, Refusal};
use duet_bus::{Cursor, Error, Place, Polled, ProjectJournal, Replay, Report, Tail, Writer};
use duet_schemas::journal::{
    AskAnswered, AskAsked, AskWithdrawn, At, Audience, Body, By, DocDigest, DocWritten, Entry,
    Event, Id, JournalHead, MomentPlayed, SegmentStamp, Skip, TakeAdded, VerdictGiven,
};
use duet_schemas::{Journal, JournalIndex, Tool, Version};
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

/// Rolls after every entry: any segment with one entry is full.
const TINY: u64 = 1;
/// Never rolls in a test.
const HUGE: u64 = 1 << 40;

fn writer(root: &Path, who: &str, size: u64) -> Writer {
    Writer::new(
        root,
        Tool::new("duet", "0.3.0"),
        "wayfaring",
        By::new(who, "cli"),
    )
    .segment_size(size)
}

fn journal(root: &Path) -> ProjectJournal {
    duet_bus::read(root).unwrap()
}

fn index_text(root: &Path) -> String {
    fs::read_to_string(duet_bus::index_path(root)).unwrap()
}

fn segment_text(root: &Path, stamp: SegmentStamp) -> String {
    fs::read_to_string(duet_bus::segment_path(root, stamp)).unwrap()
}

fn current(root: &Path) -> SegmentStamp {
    journal(root).index.current().unwrap().segment
}

fn ask(w: &mut Writer, question: &str) -> Id {
    w.append(AskAsked::new(question), None).unwrap().id
}

fn moment(w: &mut Writer) -> Id {
    w.append(
        MomentPlayed::new("wayfaring@1.1-2.1", Audience::Owner, "take"),
        None,
    )
    .unwrap()
    .id
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
    moment(w);
    let mut verdict = VerdictGiven::new("wayfaring@2.15-3.58", "no audible difference");
    verdict.ask = Some(a);
    w.append(verdict, Some(answer)).unwrap();
    let digest = "sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34"
        .parse()
        .unwrap();
    w.append(TakeAdded::new("wayfaring", digest), None).unwrap();
    w.append(
        DocWritten::new(
            "songs/wayfaring/parts/lead.duet".parse().unwrap(),
            "part",
            Version::new(1, 0),
            DocDigest::of(b"{}"),
            true,
        ),
        None,
    )
    .unwrap();
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

/// A stream's head and record validators, from its committed schema.
fn stream_validators(kind: &str) -> (jsonschema::Validator, jsonschema::Validator) {
    let stream = read_json(&schemas_dir().join(format!("{kind}/1.0.json")));
    let id = stream["$id"].as_str().unwrap();
    (
        validator(&stream, &[]),
        validator(
            &json!({ "$ref": format!("{id}#/$defs/Record") }),
            &[&stream],
        ),
    )
}

// --- Round trip ---

#[test]
fn the_index_and_every_segment_round_trip_exactly_and_validate() {
    let root = scratch("roundtrip");
    let mut w = writer(&root, "claude", 600);
    every_event(&mut w);
    every_event(&mut w);
    let read = journal(&root);
    assert!(
        read.segments.len() > 2,
        "it rolled: {}",
        read.segments.len()
    );
    assert_eq!(read.len(), 16);
    assert!(read.missing.is_empty() && read.orphans.is_empty());

    let index = index_text(&root);
    assert_eq!(
        read.index.write().unwrap(),
        index,
        "the index, read then written"
    );
    let (index_head, index_line) = stream_validators("journal-index");
    let mut lines = index.lines();
    assert_valid(
        &index_head,
        &serde_json::from_str(lines.next().unwrap()).unwrap(),
        "index head",
    );
    for (n, line) in lines.enumerate() {
        assert_valid(
            &index_line,
            &serde_json::from_str(line).unwrap(),
            &format!("index line {}", n + 2),
        );
    }

    let (head, record) = stream_validators("journal");
    let mut kinds = HashSet::new();
    for segment in &read.segments {
        let text = segment_text(&root, segment.stamp);
        assert_eq!(
            segment.journal.write().unwrap(),
            text,
            "{}, read then written",
            segment.stamp
        );
        let mut lines = text.lines();
        assert_valid(
            &head,
            &serde_json::from_str(lines.next().unwrap()).unwrap(),
            "segment head",
        );
        for line in lines {
            let entry: Value = serde_json::from_str(line).unwrap();
            assert_valid(&record, &entry, line);
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
    }
    let known: HashSet<String> = duet_schemas::EVENTS
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    assert_eq!(kinds, known, "every event kind is written once at least");

    // Each closing summary says what its segment holds, and the songs, parts and takes it concerns.
    let closes: Vec<_> = read
        .index
        .lines
        .iter()
        .filter_map(|l| l.line.closes.as_ref())
        .collect();
    assert_eq!(closes.len(), read.segments.len() - 1);
    for c in &closes {
        let segment = read.segments.iter().find(|s| s.stamp == c.segment).unwrap();
        assert_eq!(c.entries, segment.journal.lines.len() as u64);
        assert_eq!(&c.last, &segment.journal.lines.last().unwrap().id);
    }
    assert!(
        closes
            .iter()
            .any(|c| c.parts == ["wayfaring/lead"] && c.songs == ["wayfaring"])
    );
    assert!(
        closes
            .iter()
            .any(|c| c.takes.len() == 1 && c.takes[0].take == "wayfaring")
    );
    assert!(!Report::of(&read).is_wrong());
}

// --- Two processes at once ---

const CHILD: &str = "DUET_BUS_TEST_CHILD";
const PER_CHILD: usize = 1000;
/// About 70 entries a segment: each child rolls many times.
const CONCURRENT_SIZE: u64 = 16 * 1024;

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
    let mut w = writer(Path::new(root), who, CONCURRENT_SIZE);
    for n in 0..PER_CHILD {
        w.append(VerdictGiven::new(format!("{who}-{n}"), "heard"), None)
            .unwrap();
    }
}

#[test]
fn two_processes_appending_a_thousand_each_roll_and_leave_whole_unique_entries() {
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
    let read = journal(&root);
    assert!(
        read.segments.len() > 10,
        "rolled {} times",
        read.segments.len() - 1
    );
    assert!(
        read.missing.is_empty() && read.orphans.is_empty(),
        "every named segment exists"
    );
    assert!(read.index.torn.is_none() && index_text(&root).ends_with('\n'));
    for segment in &read.segments {
        assert!(
            segment.journal.torn.is_none(),
            "{} has a torn line",
            segment.stamp
        );
        assert!(segment_text(&root, segment.stamp).ends_with('\n'));
    }
    assert_eq!(read.len(), 2 * PER_CHILD, "2 000 entries");
    let ids: HashSet<&Id> = read.entries().map(|(_, l)| &l.id).collect();
    assert_eq!(ids.len(), 2 * PER_CHILD, "every id unique");
    // Exactly one open entry: every line after the first closes the segment the line before it
    // opened, so only the last is open.
    let lines = &read.index.lines;
    assert!(lines[0].line.closes.is_none());
    for pair in lines.windows(2) {
        let closes = pair[1]
            .line
            .closes
            .as_ref()
            .expect("every later line closes one");
        assert_eq!(closes.segment, pair[0].line.opens.segment);
    }
    for who in ["one", "two"] {
        let mine: Vec<String> = read
            .entries()
            .filter(|(_, l)| l.by.who == who)
            .map(|(_, l)| {
                serde_json::from_str::<Value>(&l.text).unwrap()["body"]["about"].to_string()
            })
            .collect();
        let want: Vec<String> = (0..PER_CHILD).map(|n| format!("\"{who}-{n}\"")).collect();
        assert_eq!(mine, want, "{who}'s appends, each once, in its order");
    }
    let entries: Vec<_> = read.entries().collect();
    let turns = entries
        .windows(2)
        .filter(|w| w[0].1.by.who != w[1].1.by.who)
        .count();
    assert!(turns > 1, "the two appended at once ({turns} turns)");
    assert!(!Report::of(&read).is_wrong());
}

// --- Torn last lines ---

#[test]
fn a_segment_or_the_index_cut_mid_line_is_cut_back_and_nothing_before_it_changes() {
    let root = scratch("torn");
    let mut w = writer(&root, "claude", HUGE);
    every_event(&mut w);
    let stamp = current(&root);
    let (index_before, segment_before) = (index_text(&root), segment_text(&root, stamp));
    let torn_entry = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04"#;
    let torn_index = r#"{"opens":{"segment":"2026111"#;
    fs::write(
        duet_bus::segment_path(&root, stamp),
        format!("{segment_before}{torn_entry}"),
    )
    .unwrap();
    fs::write(
        duet_bus::index_path(&root),
        format!("{index_before}{torn_index}"),
    )
    .unwrap();

    let read = journal(&root);
    assert_eq!(read.len(), 8, "a reader reads every whole line");
    let report = Report::of(&read);
    assert!(!report.is_wrong() && report.torn.len() == 2, "{report}");

    let appended = writer(&root, "owner", HUGE)
        .append(AskAsked::new("Now?"), None)
        .unwrap();
    let dropped: Vec<&str> = appended.cuts.iter().map(|c| c.dropped.as_str()).collect();
    assert_eq!(dropped, [torn_index, torn_entry]);
    assert_eq!(
        index_text(&root),
        index_before,
        "nothing before the index's cut changes"
    );
    let after = segment_text(&root, stamp);
    assert!(
        after.starts_with(&segment_before),
        "nothing before the segment's cut changes"
    );
    assert_eq!(after.lines().count(), segment_before.lines().count() + 1);
    assert_eq!(appended.place, Place::new(stamp, 9));
}

#[test]
fn a_torn_index_head_is_cut_and_the_journal_started_again() {
    let root = scratch("torn-head");
    fs::write(
        duet_bus::index_path(&root),
        r#"{"$schema":"https://raw.githubusercontent.com/tas"#,
    )
    .unwrap();
    let appended = writer(&root, "claude", HUGE)
        .append(AskAsked::new("Begin?"), None)
        .unwrap();
    assert!(appended.started && appended.cuts.len() == 1 && appended.place.seq == 1);
    assert_eq!(journal(&root).len(), 1);
}

// --- A roll that crashed between its two writes ---

#[test]
fn a_segment_left_without_its_index_line_is_adopted_by_the_next_append() {
    let root = scratch("adopt");
    let mut w = writer(&root, "claude", HUGE);
    ask(&mut w, "First?");
    moment(&mut w);
    let old = current(&root);
    let index_before = index_text(&root);
    // The crash: the new segment's head is written, and its index line never is.
    let stamp = old.next().next();
    let head = JournalHead::new("wayfaring", duet_bus::fresh_id().unwrap(), stamp);
    let head_line = Journal::head_line(&Tool::new("duet", "0.3.0"), &head).unwrap();
    fs::write(duet_bus::segment_path(&root, stamp), &head_line).unwrap();
    let report = Report::of(&journal(&root));
    assert_eq!(report.adoptable, [stamp], "{report}");
    assert!(!report.is_wrong());

    let appended = writer(&root, "owner", HUGE)
        .append(AskAsked::new("Second?"), None)
        .unwrap();
    assert_eq!(appended.adopted, Some(stamp));
    assert_eq!(
        appended.place,
        Place::new(stamp, 1),
        "the entry lands in the adopted segment"
    );
    let index = index_text(&root);
    assert!(index.starts_with(&index_before));
    assert_eq!(
        index.lines().count(),
        index_before.lines().count() + 1,
        "one index line"
    );
    let read = journal(&root);
    let last = &read.index.lines.last().unwrap().line;
    assert_eq!(last.opens.segment, stamp);
    assert_eq!(last.opens.id, head.id);
    let closes = last.closes.as_ref().unwrap();
    assert_eq!((closes.segment, closes.entries), (old, 2));
    assert!(read.orphans.is_empty() && !Report::of(&read).is_wrong());
}

// --- What this build does not read ---

/// A line of an event kind this build does not know, keys out of order and spaced as no writer
/// of today writes: kept byte for byte.
const UNKNOWN: &str = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"app"},"event":"lyric.sung","v":"1.0","body":{ "z": 1, "a": [2, 3] }}"#;

/// `ask.asked` at a newer minor, with a field 1.0 does not hold: kept byte for byte.
const NEWER: &str = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f7","at":"2026-11-17T15:04:06.000Z","by":{"who":"claude","face":"mcp"},"event":"ask.asked","v":"1.7","body":{"question":"Louder?","urgency":"high"}}"#;

#[test]
fn unknown_events_and_newer_minors_stay_byte_for_byte_across_a_roll_and_are_listed() {
    let root = scratch("unknown");
    let mut w = writer(&root, "claude", HUGE);
    ask(&mut w, "First?");
    let stamp = current(&root);
    let path = duet_bus::segment_path(&root, stamp);
    let kept = format!("{}{UNKNOWN}\n{NEWER}\n", segment_text(&root, stamp));
    fs::write(&path, &kept).unwrap();

    // Later appends, by a writer that rolls: the first rolls away from the segment.
    let mut later = writer(&root, "owner", TINY);
    every_event(&mut later);
    assert_eq!(
        segment_text(&root, stamp),
        kept,
        "both lines stay as they were"
    );
    let read = journal(&root);
    assert!(read.segments.len() > 2);
    assert_eq!(
        read.segments[0].journal.write().unwrap(),
        kept,
        "read, then written"
    );
    let skipped: Vec<(Place, &str, &Body)> = read
        .entries()
        .filter(|(_, l)| l.read().is_none())
        .map(|(p, l)| (p, l.event.as_str(), &l.body))
        .collect();
    assert_eq!(
        skipped,
        [
            (
                Place::new(stamp, 2),
                "lyric.sung",
                &Body::Skipped(Skip::UnknownEvent)
            ),
            (
                Place::new(stamp, 3),
                "ask.asked",
                &Body::Skipped(Skip::NewerMinor {
                    reads: Version::new(1, 0)
                })
            ),
        ]
    );
    let report = Report::of(&read);
    assert!(!report.is_wrong(), "{report}");
    let shown = report.to_string();
    assert!(shown.contains(&format!("{stamp} seq 2 (0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6): lyric.sung 1.0, an event kind this duet does not know")), "{shown}");
    assert!(
        shown.contains("ask.asked 1.7, newer than the 1.0 this duet reads"),
        "{shown}"
    );
}

#[test]
fn an_unknown_field_in_a_known_event_is_refused_by_name() {
    let root = scratch("unknown-field");
    let mut w = writer(&root, "claude", HUGE);
    let a = ask(&mut w, "First?");
    let stamp = current(&root);
    let line = format!(
        r#"{{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{{"who":"owner","face":"cli"}},"event":"ask.answered","v":"1.0","body":{{"ask":"{a}","answer":"yes","mood":"glad"}}}}"#
    );
    let path = duet_bus::segment_path(&root, stamp);
    fs::write(&path, format!("{}{line}\n", segment_text(&root, stamp))).unwrap();
    let err = chain(&duet_bus::read(&root).unwrap_err());
    assert!(
        err.contains(&stamp.file_name()) && err.contains("line 3"),
        "{err}"
    );
    assert!(
        err.contains("ask.answered 1.0") && err.contains("`mood`"),
        "{err}"
    );
    // A writer will not append to what it cannot read, and says why.
    let err = writer(&root, "owner", HUGE)
        .append(AskAsked::new("Next?"), None)
        .unwrap_err();
    assert!(chain(&err).contains("`mood`"), "{}", chain(&err));
    // In the frame too.
    let frame = r#"{"id":"0196f1a2-7b3c-7d4e-8f90-a1b2c3d4e5f6","at":"2026-11-17T15:04:05.123Z","by":{"who":"owner","face":"cli"},"event":"ask.asked","v":"1.0","seq":4,"body":{"question":"?"}}"#;
    let err = duet_schemas::journal::read_line(frame)
        .unwrap_err()
        .to_string();
    assert!(err.contains("`seq`"), "{err}");
}

// --- Union merge ---

/// Copies every file of `from` (not `.duet/`) into a new clone.
fn clone(from: &Path, name: &str) -> PathBuf {
    let to = scratch(name);
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
    to
}

/// git's union driver over every journal file, without git: a file in one clone only is taken
/// as it is; a file in both is the base's lines once, then ours, then theirs.
fn union(base: &Path, ours: &Path, theirs: &Path) -> PathBuf {
    let out = scratch("union-merged");
    let mut names = HashSet::new();
    for dir in [ours, theirs] {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                names.insert(entry.file_name());
            }
        }
    }
    for name in names {
        let read = |dir: &Path| fs::read_to_string(dir.join(&name)).ok();
        let text = match (read(base), read(ours), read(theirs)) {
            (_, Some(a), None) => a,
            (_, None, Some(b)) => b,
            (base, Some(a), Some(b)) => {
                let base = base.unwrap_or_default();
                assert!(a.starts_with(&base) && b.starts_with(&base), "append-only");
                format!("{base}{}{}", &a[base.len()..], &b[base.len()..])
            }
            (_, None, None) => unreachable!(),
        };
        fs::write(out.join(name), text).unwrap();
    }
    out
}

/// A base journal of two asks, and two clones of it.
fn base_and_clones() -> (PathBuf, PathBuf, PathBuf, Id, Id) {
    let base = scratch("union-base");
    let mut w = writer(&base, "claude", HUGE);
    let first = ask(&mut w, "Which ending?");
    let second = ask(&mut w, "Which tempo?");
    let (a, b) = (clone(&base, "union-a"), clone(&base, "union-b"));
    (base, a, b, first, second)
}

fn merged_is_whole(merged: &Path, entries: usize, first: &Id, second: &Id) -> ProjectJournal {
    let read = journal(merged);
    assert_eq!(read.len(), entries, "every entry of both clones");
    let report = Report::of(&read);
    assert!(!report.is_wrong(), "{report}");
    assert!(report.unread.is_empty() && report.duplicates.is_empty());
    assert!(report.to_string().ends_with("nothing wrong"), "{report}");
    let replay = Replay::of(&read);
    assert_eq!(replay.ask(first).unwrap().state, AskState::Answered);
    assert_eq!(replay.ask(second).unwrap().state, AskState::Withdrawn);
    // A writer appends to the merge, folding it from the start.
    writer(merged, "owner", HUGE)
        .append(AskAsked::new("After the merge?"), None)
        .unwrap();
    assert!(!Report::of(&journal(merged)).is_wrong());
    read
}

#[test]
fn a_union_merge_where_neither_clone_rolled_reads_and_checks_clean() {
    let (base, a, b, first, second) = base_and_clones();
    let mut on_a = writer(&a, "owner", HUGE);
    on_a.append(AskAnswered::new(first.clone(), "two"), None)
        .unwrap();
    moment(&mut on_a);
    let mut on_b = writer(&b, "claude", HUGE);
    on_b.append(AskWithdrawn::new(second.clone()), None)
        .unwrap();
    let b_last = moment(&mut on_b);
    let theirs_place = Place::new(current(&b), 4);
    let merged = union(&base, &a, &b);
    let read = merged_is_whole(&merged, 6, &first, &second);
    assert_eq!(read.segments.len(), 1);

    // A cursor into the segment that was merged into gets resync: theirs' last line moved.
    let cursor = Cursor::at(theirs_place, b_last);
    let Polled::Resync(again) = duet_bus::after(&read, &cursor) else {
        panic!("a resync")
    };
    assert_eq!(again.len(), 6);
    assert_eq!(again[0].0, Place::new(read.segments[0].stamp, 1));
}

#[test]
fn a_union_merge_where_one_clone_rolled_reads_and_states_the_differing_close() {
    let (base, a, b, first, second) = base_and_clones();
    let mut on_a = writer(&a, "owner", TINY);
    on_a.append(AskAnswered::new(first.clone(), "two"), None)
        .unwrap();
    moment(&mut on_a);
    let mut on_b = writer(&b, "claude", HUGE);
    on_b.append(AskWithdrawn::new(second.clone()), None)
        .unwrap();
    let merged = union(&base, &a, &b);
    let read = merged_is_whole(&merged, 5, &first, &second);
    assert_eq!(read.segments.len(), 3);
    let report = Report::of(&read);
    assert_eq!(report.close_differs.len(), 1, "{report}");
    assert!(
        report.to_string().contains("by a merge; the file stands"),
        "{report}"
    );
}

#[test]
fn a_union_merge_where_both_clones_rolled_reads_every_segment_and_states_the_merge() {
    let (base, a, b, first, second) = base_and_clones();
    let mut on_a = writer(&a, "owner", TINY);
    on_a.append(AskAnswered::new(first.clone(), "two"), None)
        .unwrap();
    let mut on_b = writer(&b, "claude", TINY);
    on_b.append(AskWithdrawn::new(second.clone()), None)
        .unwrap();
    let merged = union(&base, &a, &b);
    let read = merged_is_whole(&merged, 4, &first, &second);
    // The old segment, then ours, then theirs: both new segments, every entry of each.
    assert_eq!(read.segments.len(), 3);
    assert_eq!(read.segments[1].stamp, current(&a));
    assert_eq!(read.segments[2].stamp, current(&b));
    assert_eq!(
        read.index.current().unwrap().segment,
        current(&b),
        "the last line is current"
    );
    let report = Report::of(&read);
    assert_eq!(report.closed_twice, [read.segments[0].stamp], "{report}");
    assert!(
        report
            .to_string()
            .contains("closed by more than one index line"),
        "{report}"
    );
    // Ours was closed implicitly, by theirs' line: it has no summary, and is stated so.
    assert_eq!(report.unsummarised, [current(&a)], "{report}");
    assert!(report.to_string().contains("is unsummarised"), "{report}");
    assert!(!read.index.lines.iter().any(|l| {
        l.line
            .closes
            .as_ref()
            .is_some_and(|c| c.segment == current(&a))
    }));
}

// --- Two answers to one ask ---

#[test]
fn two_answers_to_one_ask_in_different_segments_replay_the_first_standing() {
    let root = scratch("two-answers");
    let mut w = writer(&root, "claude", TINY);
    let a = ask(&mut w, "Which ending?");
    let one = w.append(AskAnswered::new(a.clone(), "one"), None).unwrap();
    moment(&mut w);
    // The merge, by hand: a second answer to the same ask, in a later segment.
    let at = At::from_unix_ms(1_795_446_245_000);
    let two = Entry::new(
        duet_bus::fresh_id().unwrap(),
        at,
        By::new("owner", "cli"),
        AskAnswered::new(a.clone(), "two"),
    );
    let stamp = current(&root);
    let path = duet_bus::segment_path(&root, stamp);
    fs::write(
        &path,
        format!("{}{}", segment_text(&root, stamp), two.line().unwrap()),
    )
    .unwrap();

    let read = journal(&root);
    assert_eq!(read.segments.len(), 3);
    let replay = Replay::of(&read);
    let record = replay.ask(&a).unwrap();
    assert_eq!(
        (record.state, record.settled),
        (AskState::Answered, Some(one.place))
    );
    let report = Report::of(&read);
    assert!(report.is_wrong());
    let refused = &report.refused[..];
    assert_eq!(refused.len(), 1);
    assert_eq!(
        (refused[0].place, refused[0].stands),
        (Place::new(stamp, 2), Some(one.place))
    );
    assert_eq!(refused[0].refusal, Refusal::AlreadyAnswered);
    let shown = report.to_string();
    assert!(
        shown.contains(&format!(
            "{stamp} seq 2 ({}): ask.answered of ask {a}, refused already-answered; {} stands",
            two.id, one.place
        )),
        "{shown}"
    );
}

// --- The check and the append are one step ---

#[test]
fn a_writer_with_a_stale_view_is_refused_already_answered() {
    let root = scratch("check-and-append");
    let mut asker = writer(&root, "claude", HUGE);
    let a = ask(&mut asker, "Which ending?");
    let mut first = writer(&root, "owner", HUGE);
    let mut second = writer(&root, "owner", HUGE);
    moment(&mut first);
    moment(&mut second);
    assert_eq!(
        second.replay().open().len(),
        1,
        "both have read the ask open"
    );

    first
        .append(AskAnswered::new(a.clone(), "one"), None)
        .unwrap();
    let before = segment_text(&root, current(&root));
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
    assert_eq!(
        segment_text(&root, current(&root)),
        before,
        "nothing appended"
    );
}

// --- Cursors and the tail ---

#[test]
fn a_cursor_taken_before_a_roll_resumes_after_a_restart() {
    let root = scratch("cursor");
    let mut w = writer(&root, "claude", HUGE);
    ask(&mut w, "One?");
    ask(&mut w, "Two?");
    let first = current(&root);
    let (tail, now) = Tail::open(&root, None).unwrap();
    let Polled::After(all) = now else {
        panic!("from the start")
    };
    assert_eq!(all.len(), 2);
    let cursor = tail.cursor().unwrap().clone();
    assert_eq!((cursor.segment, cursor.seq), (first, 2));
    drop(tail); // the restart

    // More in the first segment, then a roll, then more in the second.
    ask(&mut w, "Three?");
    let mut rolling = writer(&root, "owner", TINY);
    let rolled = rolling.append(AskAsked::new("Four?"), None).unwrap();
    assert!(rolled.rolled.is_some());
    rolling.append(AskAsked::new("Five?"), None).unwrap();

    let (mut tail, from) = Tail::open(&root, Some(cursor)).unwrap();
    let Polled::After(next) = from else {
        panic!("the cursor still holds")
    };
    let places: Vec<Place> = next.iter().map(|(p, _)| *p).collect();
    assert_eq!(
        places[0],
        Place::new(first, 3),
        "the rest of its segment first"
    );
    assert_eq!(places[1], rolled.place, "then the next segment");
    assert_eq!(
        places.len(),
        3,
        "three, the last rolled into a third segment"
    );
    assert_eq!(tail.poll().unwrap(), Polled::After(Vec::new()));

    // A torn line is never read; the next append cuts it, and the tail reads on.
    let stamp = current(&root);
    let path = duet_bus::segment_path(&root, stamp);
    fs::write(
        &path,
        format!("{}{{\"id\":\"01", segment_text(&root, stamp)),
    )
    .unwrap();
    assert_eq!(tail.poll().unwrap(), Polled::After(Vec::new()));
    let six = rolling.append(AskAsked::new("Six?"), None).unwrap();
    let Polled::After(new) = tail.poll().unwrap() else {
        panic!("no resync")
    };
    assert_eq!(new.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [six.place]);
}

#[test]
fn a_cursor_whose_line_moved_resyncs_from_the_start_of_its_segment() {
    let root = scratch("cursor-moved");
    let mut w = writer(&root, "claude", TINY);
    for q in ["One?", "Two?", "Three?"] {
        ask(&mut w, q);
    }
    let read = journal(&root);
    let second = read.segments[1].stamp;
    let cursor = Cursor::at(
        Place::new(second, 1),
        read.segments[1].journal.lines[0].id.clone(),
    );
    // A hand edit puts a line before the cursor's in its segment.
    let late = Entry::new(
        duet_bus::fresh_id().unwrap(),
        At::from_unix_ms(1_795_446_245_000),
        By::new("owner", "cli"),
        Event::from(AskAsked::new("Earlier?")),
    );
    let text = segment_text(&root, second);
    let (head, rest) = text.split_at(text.find('\n').unwrap() + 1);
    let path = duet_bus::segment_path(&root, second);
    fs::write(&path, format!("{head}{}{rest}", late.line().unwrap())).unwrap();
    let read = journal(&root);
    let Polled::Resync(again) = duet_bus::after(&read, &cursor) else {
        panic!("a resync")
    };
    assert_eq!(
        again[0].0,
        Place::new(second, 1),
        "from the start of its segment"
    );
    assert_eq!(again.len(), 3, "that segment's two, then the third's one");
}

#[test]
fn a_tail_is_sent_resync_when_a_segment_before_its_cursor_is_merged_into() {
    let root = scratch("tail-merge");
    let mut w = writer(&root, "claude", TINY);
    ask(&mut w, "One?");
    ask(&mut w, "Two?");
    ask(&mut w, "Three?");
    let first = journal(&root).segments[1].stamp;
    let (mut tail, _) = Tail::open(&root, None).unwrap();
    // Another clone's entry lands in the first segment, after it was closed: a merge.
    let late = Entry::new(
        duet_bus::fresh_id().unwrap(),
        At::from_unix_ms(1_795_446_245_000),
        By::new("owner", "cli"),
        Event::from(AskAsked::new("Late?")),
    );
    let path = duet_bus::segment_path(&root, first);
    fs::write(
        &path,
        format!("{}{}", segment_text(&root, first), late.line().unwrap()),
    )
    .unwrap();
    let Polled::Resync(again) = tail.poll().unwrap() else {
        panic!("a resync")
    };
    assert_eq!(
        again[0].0,
        Place::new(first, 1),
        "from the start of the segment that grew"
    );
    assert_eq!(again.len(), 3, "its two, then the third's one");
}

#[test]
fn the_index_reads_and_writes_without_the_writer() {
    // The reader of the index on its own: the committed example round-trips.
    let example =
        fs::read_to_string(schemas_dir().join("journal-index/examples/1.0.duet.jsonl")).unwrap();
    let index = JournalIndex::read(&example).unwrap();
    assert_eq!(index.lines.len(), 2);
    assert_eq!(index.write().unwrap(), example);
}
