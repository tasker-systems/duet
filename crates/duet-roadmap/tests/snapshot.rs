//! The snapshot drawn from fixed records: its lanes and counts, its determinism, what it does when a
//! read fails, and that its only write is a create. No network: `common::Fake` answers every call.

mod common;

use std::path::Path;

use common::{CREATED_ID, Fake, Fault, clock};
use duet_roadmap::{Call, Counts, Lane, Mode, WORKSTREAMS, draw_snapshot};

const G: fn(u64) -> String = |n| format!("10000000-0000-7000-8000-{n:012}");
const T: fn(u64) -> String = |n| format!("20000000-0000-7000-8000-{n:012}");

fn counts(claimed: usize, ready_unclaimed: usize, open: usize, done: usize) -> Counts {
    Counts {
        claimed,
        ready_unclaimed,
        open,
        done,
    }
}

#[test]
fn fixed_records_draw_the_expected_lanes_and_counts() {
    let mut fake = Fake::new();
    let drawn = draw_snapshot(&mut fake, &mut clock("2026-11-19T03:00:00Z"), Mode::DryRun)
        .expect("the fixture draws");
    let snap = &drawn.snapshot;

    // Every workstream in the vocabulary appears, in the vocabulary's order, empty ones included.
    let names: Vec<&str> = snap.groups.iter().map(|g| g.workstream.as_str()).collect();
    assert_eq!(names, WORKSTREAMS);

    let lines: Vec<(&str, String, Lane, Counts)> = snap
        .groups
        .iter()
        .flat_map(|g| {
            g.goals
                .iter()
                .map(move |l| (g.workstream.as_str(), l.id.clone(), l.lane, l.counts))
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            // Two claimed (one of them also ready), and the claimed item that is done counts as done.
            ("songs-and-comps", G(1), Lane::Now, counts(2, 0, 2, 1)),
            // Ready and unclaimed puts a goal in Now; the relates_to edge to goal 1 is not a link.
            ("bus-and-project", G(2), Lane::Now, counts(0, 1, 2, 0)),
            // A paused goal is open; its cancelled item counts nowhere.
            ("workbench", G(3), Lane::Next, counts(0, 0, 1, 0)),
            // Ordered by lane before title: "Zither…" (Now) before "A capture…" (Later).
            ("capture-and-readings", G(10), Lane::Now, counts(1, 0, 1, 0)),
            // Only done work: Later, with its done item counted.
            (
                "capture-and-readings",
                G(4),
                Lane::Later,
                counts(0, 0, 0, 1)
            ),
            // No work at all: Later.
            (
                "models-and-polyphony",
                G(5),
                Lane::Later,
                counts(0, 0, 0, 0)
            ),
        ]
    );
    let ci = snap
        .groups
        .iter()
        .find(|g| g.workstream == "ci-and-release")
        .expect("named");
    assert!(
        ci.goals.is_empty(),
        "ci-and-release has no open goal, so it is named as empty"
    );

    // Open goals with no workstream, or one outside the vocabulary, are named, each in its lane,
    // ordered by lane then title like any group.
    let ungrouped: Vec<(String, Option<&str>, Lane, Counts)> = snap
        .ungrouped
        .iter()
        .map(|l| (l.id.clone(), l.workstream.as_deref(), l.lane, l.counts))
        .collect();
    assert_eq!(
        ungrouped,
        vec![
            (
                G(9),
                Some("not-a-workstream"),
                Lane::Now,
                counts(0, 1, 1, 0)
            ),
            (G(6), None, Lane::Next, counts(0, 0, 1, 0)),
        ]
    );

    // Work advancing no open goal is counted from the full task list: a link to a completed goal
    // (10, 11), no link (12, 13), a link outside duet-dev (15), and a relates_to only (19).
    assert_eq!((snap.unlinked_open, snap.unlinked_done), (4, 2));
    // A task with two goal links is named and counted for neither.
    let multi: Vec<&str> = snap.multiple_goals.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(multi, vec![T(16).as_str()]);
    assert_eq!(
        snap.omitted,
        vec![("completed".to_owned(), 1), ("cancelled".to_owned(), 1)]
    );

    assert_eq!(drawn.title, "duet roadmap snapshot, 2026-11-19");
    let expected = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/expected.md"),
    )
    .expect("the expected snapshot is readable");
    assert_eq!(
        drawn.body, expected,
        "the rendered snapshot matches tests/fixture/expected.md"
    );
}

/// The lines that carry the stated date and read time; nothing else may differ between runs.
fn without_date(body: &str) -> String {
    body.lines()
        .filter(|l| {
            !(l.starts_with("# duet roadmap snapshot, ")
                || l.starts_with("- **Date described:**")
                || l.starts_with("- **Records read:**"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn two_runs_over_the_same_records_differ_only_in_the_date() {
    let mut first = Fake::new();
    let a =
        draw_snapshot(&mut first, &mut clock("2026-11-19T03:00:00Z"), Mode::DryRun).expect("draws");
    // The same records, served in the opposite order, read on another day.
    let mut second = Fake {
        reverse: true,
        ..Fake::new()
    };
    let b = draw_snapshot(
        &mut second,
        &mut clock("2026-12-25T18:30:00Z"),
        Mode::DryRun,
    )
    .expect("draws");

    assert_ne!(a.body, b.body, "the stated date and read time differ");
    assert_eq!(without_date(&a.body), without_date(&b.body));
    assert_eq!(a.snapshot, b.snapshot);
}

#[test]
fn a_failed_or_truncated_read_writes_nothing_and_says_which() {
    let cases = [
        (Fault::TruncatedGoals, "truncated"),
        (Fault::ShortTasks, "18 of 19"),
        (Fault::FailShow(T(9)), &*T(9)),
        (Fault::Garbage, "not JSON"),
    ];
    for (fault, names) in cases {
        let mut fake = Fake::with_fault(fault.clone());
        let err = draw_snapshot(&mut fake, &mut clock("2026-11-19T03:00:00Z"), Mode::Write)
            .expect_err(&format!("{fault:?} must fail"));
        let message = err.to_string();
        assert!(
            message.contains(names),
            "{fault:?}: {message:?} should name {names:?}"
        );
        assert!(
            fake.creates().is_empty(),
            "{fault:?}: nothing may be written"
        );
    }
}

#[test]
fn the_only_write_is_one_create() {
    let mut fake = Fake::new();
    let drawn =
        draw_snapshot(&mut fake, &mut clock("2026-11-19T03:00:00Z"), Mode::Write).expect("draws");
    assert_eq!(drawn.created.as_deref(), Some(CREATED_ID));

    let creates = fake.creates();
    assert_eq!(creates.len(), 1, "exactly one write");
    let Call::CreateSnapshot {
        title,
        body,
        open_meta,
    } = creates[0]
    else {
        unreachable!()
    };
    assert_eq!(title, &drawn.title);
    assert_eq!(
        body, &drawn.body,
        "what is written is what a dry run prints"
    );
    assert_eq!(open_meta["tags"], serde_json::json!(["roadmap-snapshot"]));
    assert_eq!(open_meta["date"], "2026-11-19");

    // Every other call is a read: one listing each, and one edge read per task.
    let reads = fake
        .calls
        .iter()
        .filter(|c| !matches!(c, Call::CreateSnapshot { .. }))
        .count();
    assert_eq!(reads, 2 + 19);
    for call in &fake.calls {
        let argv = call.argv();
        let verb = (argv[0].as_str(), argv[1].as_str());
        match call {
            Call::CreateSnapshot { .. } => {
                assert_eq!(verb, ("resource", "create"));
                let joined = argv.join(" ");
                assert!(
                    joined.contains("--type research --context +making-music/duet-dev "),
                    "{joined}"
                );
            }
            _ => assert!(
                verb == ("resource", "list") || verb == ("resource", "show"),
                "{argv:?}"
            ),
        }
    }

    // A dry run makes the same reads and writes nothing.
    let mut dry = Fake::new();
    let drawn =
        draw_snapshot(&mut dry, &mut clock("2026-11-19T03:00:00Z"), Mode::DryRun).expect("draws");
    assert!(dry.creates().is_empty());
    assert_eq!(drawn.created, None);
}

/// The tool's workstreams are CLAUDE.md's, in its order, so a vocabulary change fails here first.
#[test]
fn the_workstreams_are_claude_md_s_vocabulary() {
    let claude =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CLAUDE.md"))
            .expect("CLAUDE.md is readable");
    let start = claude
        .find("**The grouping key (ROAD-3):**")
        .expect("the grouping key is written");
    let rest = &claude[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    let values: Vec<&str> = rest[..end]
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|v| *v != "open_meta.workstream")
        .collect();
    assert_eq!(values, WORKSTREAMS);
}
