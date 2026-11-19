//! The snapshot as a Temper resource: its title, markdown body and open metadata.

use std::fmt::Write as _;
use std::time::SystemTime;

use serde_json::{Value, json};

use crate::draw::{GoalLine, Snapshot};
use crate::read::Records;
use crate::time::{utc_date, utc_time};
use crate::{GOALS_CONTEXT, SNAPSHOT_TAG, TASKS_CONTEXT, WORKSTREAMS};

/// The rule every snapshot is drawn by, in words. It ends every snapshot, and `draw.rs` is the
/// same rule in code.
pub const RECIPE: &str = "\
## Recipe

Every line above was drawn by this recipe, so a reader with the same records draws the same lanes \
and counts.

1. **Read.** Every goal in `+making-music/duet-dev`, of every status, and every task in \
`+making-music/duet-dev-workflows`, each listed whole (`temper resource list --all --with \
open-meta`). A listing that says it is truncated, or holds fewer rows than its total, stops the \
draw, as does any read that fails: nothing is written.
2. **Link.** A task's goal is the peer of its outgoing `advances` edge, read the same way for every \
task (`temper resource show <task> --with edges`). A task with no such edge, or whose edge points \
at a goal that is not open or not in `+making-music/duet-dev`, advances no open goal. A task with \
more than one is named, and counted for no goal.
3. **Open goals.** A goal is open when its `temper-status` is `active` or `paused`. `completed` and \
`cancelled` goals are left out, and counted.
4. **Count.** For each open goal, over the tasks that advance it:
   - *open*: its `temper-stage` is not `done` or `cancelled`;
   - *claimed*: open, and carrying `open_meta.claim_id`;
   - *ready, unclaimed*: open, carrying `open_meta.state = \"ready\"`, and no `claim_id`. A ready \
item that is claimed counts as claimed;
   - *done*: its `temper-stage` is `done`, whether or not it was claimed or ready.

   A cancelled task counts nowhere.
5. **Place.** *Now* when claimed plus ready-unclaimed is above zero; otherwise *Next* when open is \
above zero; otherwise *Later*. Done work never places a goal.
6. **Group and order.** By `open_meta.workstream`, in the order `CLAUDE.md` writes the vocabulary: \
`songs-and-comps`, `bus-and-project`, `workbench`, `capture-and-readings`, `models-and-polyphony`, \
`ci-and-release`. Every workstream is named, and one with no open goal says so. Within a group, \
goals go by lane (Now, Next, Later), then title, then id. An open goal with no workstream, or one \
outside the vocabulary, is listed by name under its own heading, in the same order. No goal is \
ordered by a score.
7. **Leave out, and say so.** The tasks that advance no open goal are counted, unfinished and done \
apart. The goal statuses omitted are named with their counts, and so is every task with more than \
one goal link.

Two draws over the same records differ only in the stated date and the times the records were read.
";

pub(crate) struct Rendered {
    pub title: String,
    pub body: String,
    pub open_meta: Value,
}

/// Text for a table cell or a link's text: on one line, with no `|` or bracket to break it.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn table(out: &mut String, lines: &[GoalLine], with_workstream: bool) {
    let ws = if with_workstream { " Workstream |" } else { "" };
    let ws_rule = if with_workstream { "---|" } else { "" };
    let _ = writeln!(
        out,
        "| Lane | Goal |{ws} Status | Claimed | Ready, unclaimed | Open | Done |"
    );
    let _ = writeln!(out, "|---|---|{ws_rule}---|---:|---:|---:|---:|");
    for l in lines {
        let ws = if with_workstream {
            match &l.workstream {
                Some(w) => format!(" `{}` (not in the vocabulary) |", cell(w)),
                None => " none |".to_owned(),
            }
        } else {
            String::new()
        };
        let c = l.counts;
        let _ = writeln!(
            out,
            "| {} | [{}](./{}) |{ws} {} | {} | {} | {} | {} |",
            l.lane.name(),
            cell(&l.title),
            l.id,
            l.status,
            c.claimed,
            c.ready_unclaimed,
            c.open,
            c.done
        );
    }
}

pub(crate) fn render(
    snap: &Snapshot,
    records: &Records,
    started: SystemTime,
    finished: SystemTime,
) -> Rendered {
    let date = utc_date(started);
    let title = format!("duet roadmap snapshot, {date}");
    let mut out = String::new();
    let _ = writeln!(out, "# {title}\n");
    let _ = writeln!(out, "- **Date described:** {date}, in UTC.");
    let _ = writeln!(
        out,
        "- **Records read:** from {} to {}, by `duet-roadmap snapshot`.",
        utc_time(started),
        utc_time(finished)
    );
    match records.newest_update() {
        Some(at) => {
            let _ = writeln!(out, "- **Newest record read:** last changed at {at}.");
        }
        None => {
            let _ = writeln!(
                out,
                "- **Newest record read:** none; no records were listed."
            );
        }
    }
    let _ = writeln!(
        out,
        "- **Sources:** {} in `{GOALS_CONTEXT}`, of every status; {} in `{TASKS_CONTEXT}`; and each \
         task's edges. Every listing was read whole.",
        plural(snap.goals_read, "goal", "goals"),
        plural(snap.tasks_read, "task", "tasks"),
    );
    let _ = writeln!(
        out,
        "- **How each lane was read:** Now, Next and Later were each read from the records, by the \
         recipe at the end. No lane was assigned by hand.\n"
    );
    let _ = writeln!(
        out,
        "Goals are grouped by `open_meta.workstream`, and ordered within a group by lane, then title. \
         Nothing is scored.\n"
    );

    for group in &snap.groups {
        let _ = writeln!(out, "## {}\n", group.workstream);
        if group.goals.is_empty() {
            let _ = writeln!(out, "No open goal.\n");
        } else {
            table(&mut out, &group.goals, false);
            out.push('\n');
        }
    }

    let _ = writeln!(out, "## What this snapshot leaves out\n");
    let _ = writeln!(
        out,
        "- **Work that advances no open goal:** {} and {} in `{TASKS_CONTEXT}`. A cancelled task is \
         counted nowhere.",
        plural(snap.unlinked_open, "unfinished task", "unfinished tasks"),
        plural(snap.unlinked_done, "done task", "done tasks"),
    );
    let omitted: Vec<String> = snap
        .omitted
        .iter()
        .map(|(status, n)| format!("`{status}` ({})", plural(*n, "goal", "goals")))
        .collect();
    let _ = writeln!(
        out,
        "- **Goal statuses omitted:** {}.",
        omitted.join(" and ")
    );
    if snap.multiple_goals.is_empty() {
        let _ = writeln!(out, "- **Tasks with more than one goal link:** none.");
    } else {
        let _ = writeln!(
            out,
            "- **Tasks with more than one goal link,** counted for no goal:"
        );
        for t in &snap.multiple_goals {
            let _ = writeln!(out, "  - [{}](./{})", cell(&t.title), t.id);
        }
    }
    let _ = writeln!(out, "\n### Open goals with no workstream\n");
    if snap.ungrouped.is_empty() {
        let _ = writeln!(
            out,
            "None: every open goal carries one of the {} workstreams.\n",
            WORKSTREAMS.len()
        );
    } else {
        let _ = writeln!(
            out,
            "Each carries no `open_meta.workstream`, or one outside `CLAUDE.md`'s vocabulary, so it \
             sits in no group above.\n"
        );
        table(&mut out, &snap.ungrouped, true);
        out.push('\n');
    }
    out.push_str(RECIPE);

    let open_meta = json!({
        "tags": [SNAPSHOT_TAG],
        "date": date,
        "records_read_from": utc_time(started),
        "records_read_to": utc_time(finished),
    });
    Rendered {
        title,
        body: out,
        open_meta,
    }
}
