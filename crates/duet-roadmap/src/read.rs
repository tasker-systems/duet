//! Reading the records: whole listings, checked, and each task's goal link.

use serde_json::Value;

use crate::Error;
use crate::temper::{self, Call, Temper};

/// The goal statuses the baseline names. `active` and `paused` are open.
pub(crate) const GOAL_STATUSES: [&str; 4] = ["active", "paused", "completed", "cancelled"];
/// The task stages Temper names. `done` and `cancelled` are finished.
pub(crate) const TASK_STAGES: [&str; 4] = ["backlog", "in-progress", "done", "cancelled"];

/// A goal, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    /// Its id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// `managed_meta.temper-status`, one of the four goal statuses.
    pub status: String,
    /// `open_meta.workstream`, if it carries a string there.
    pub workstream: Option<String>,
    /// When the record last changed.
    pub updated: String,
}

impl Goal {
    /// Whether the goal is open: `active` or `paused`.
    pub fn is_open(&self) -> bool {
        matches!(self.status.as_str(), "active" | "paused")
    }
}

/// A task, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Its id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// `managed_meta.temper-stage`.
    pub stage: String,
    /// It carries `open_meta.claim_id`.
    pub has_claim_id: bool,
    /// It carries `open_meta.state = "ready"`.
    pub is_ready: bool,
    /// The peers of its outgoing `advances` edges, sorted, without repeats.
    pub goal_links: Vec<String>,
    /// When the record last changed.
    pub updated: String,
}

impl Task {
    /// Not `done` or `cancelled`.
    pub fn is_unfinished(&self) -> bool {
        !matches!(self.stage.as_str(), "done" | "cancelled")
    }
}

/// Everything a snapshot is drawn from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Records {
    /// Every goal in the goals context, sorted by id.
    pub goals: Vec<Goal>,
    /// Every task in the tasks context, sorted by id.
    pub tasks: Vec<Task>,
}

impl Records {
    /// The newest `updated` among the records read, if any were.
    pub fn newest_update(&self) -> Option<&str> {
        let goals = self.goals.iter().map(|g| g.updated.as_str());
        let tasks = self.tasks.iter().map(|t| t.updated.as_str());
        goals.chain(tasks).max()
    }
}

pub(crate) fn read(temper: &mut dyn Temper) -> Result<Records, Error> {
    let mut goals = listing(temper, &Call::ListGoals, "goal")?
        .iter()
        .map(|row| goal(row, &Call::ListGoals))
        .collect::<Result<Vec<_>, _>>()?;
    goals.sort_by(|a, b| a.id.cmp(&b.id));

    let rows = listing(temper, &Call::ListTasks, "task")?;
    let mut tasks = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut task = task(row, &Call::ListTasks)?;
        task.goal_links = goal_links(temper, &task.id)?;
        tasks.push(task);
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Records { goals, tasks })
}

/// A listing's rows, only if it is whole: not marked truncated, and holding as many rows as it
/// says it returned and as its total.
fn listing(temper: &mut dyn Temper, call: &Call, what: &'static str) -> Result<Vec<Value>, Error> {
    let page = temper::json(temper, call)?;
    let shape = |detail: &str| Error::Shape {
        command: call.to_string(),
        detail: detail.to_owned(),
    };
    let truncated = |detail: String| Error::Truncated {
        what,
        command: call.to_string(),
        detail,
    };

    let rows = page["rows"]
        .as_array()
        .ok_or_else(|| shape("it has no `rows` list"))?;
    let total = page["total"]
        .as_u64()
        .ok_or_else(|| shape("it has no `total`"))?;
    let returned = page["returned"]
        .as_u64()
        .ok_or_else(|| shape("it has no `returned`"))?;
    let flagged = page["truncated"]
        .as_bool()
        .ok_or_else(|| shape("it has no `truncated` flag"))?;
    let held = rows.len() as u64;
    if flagged {
        return Err(truncated(format!(
            "it says so (`truncated: true`), holding {held} of {total} rows"
        )));
    }
    if returned != held {
        return Err(truncated(format!(
            "it says it returned {returned} rows but holds {held}"
        )));
    }
    if held != total {
        return Err(truncated(format!("it returned {held} of {total} rows")));
    }
    Ok(rows.clone())
}

fn string(row: &Value, key: &str, call: &Call) -> Result<String, Error> {
    row[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Shape {
            command: call.to_string(),
            detail: format!("a row has no `{key}`"),
        })
}

fn goal(row: &Value, call: &Call) -> Result<Goal, Error> {
    let id = string(row, "id", call)?;
    let title = string(row, "title", call)?;
    let status = row["managed_meta"]["temper-status"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if !GOAL_STATUSES.contains(&status.as_str()) {
        return Err(Error::OutOfRule {
            detail: format!(
                "goal {id} ({title}) has status {status:?}, which is none of {GOAL_STATUSES:?}; \
                 the lanes cannot tell whether it is open"
            ),
        });
    }
    Ok(Goal {
        workstream: row["open_meta"]["workstream"].as_str().map(str::to_owned),
        updated: string(row, "updated", call)?,
        id,
        title,
        status,
    })
}

fn task(row: &Value, call: &Call) -> Result<Task, Error> {
    let id = string(row, "id", call)?;
    let title = string(row, "title", call)?;
    let stage = row["managed_meta"]["temper-stage"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if !TASK_STAGES.contains(&stage.as_str()) {
        return Err(Error::OutOfRule {
            detail: format!(
                "task {id} ({title}) has stage {stage:?}, which is none of {TASK_STAGES:?}; \
                 the counts cannot tell whether it is finished"
            ),
        });
    }
    let open = &row["open_meta"];
    if !open.is_object() {
        return Err(Error::Shape {
            command: call.to_string(),
            detail: format!(
                "task {id} has no `open_meta`, so its claim and ready markers cannot be read"
            ),
        });
    }
    Ok(Task {
        has_claim_id: open.get("claim_id").is_some_and(|v| !v.is_null()),
        is_ready: open["state"] == "ready",
        goal_links: Vec::new(),
        updated: string(row, "updated", call)?,
        id,
        title,
        stage,
    })
}

/// The peers of a task's outgoing `advances` edges.
fn goal_links(temper: &mut dyn Temper, id: &str) -> Result<Vec<String>, Error> {
    let call = Call::ShowEdges { id: id.to_owned() };
    let shown = temper::json(temper, &call)?;
    let shape = |detail: String| Error::Shape {
        command: call.to_string(),
        detail,
    };
    if shown["id"] != id {
        return Err(shape(format!("it answered for {}, not {id}", shown["id"])));
    }
    let outgoing = shown["edges"]["outgoing"]
        .as_array()
        .ok_or_else(|| shape("it has no `edges.outgoing` list".to_owned()))?;
    let mut links = Vec::new();
    for edge in outgoing {
        if edge["label"] != "advances" || edge["direction"] != "outgoing" {
            continue;
        }
        let peer = edge["peer_id"]
            .as_str()
            .ok_or_else(|| shape("an edge has no `peer_id`".to_owned()))?;
        links.push(peer.to_owned());
    }
    links.sort();
    links.dedup();
    Ok(links)
}
