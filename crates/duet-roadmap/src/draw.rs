//! Placing and counting: the recipe, as code. `render::RECIPE` says the same in words.

use std::collections::BTreeMap;

use crate::WORKSTREAMS;
use crate::read::{Goal, Records};

/// A goal's lane (ROAD-2), in the order the snapshot lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// Some of its unfinished work is claimed, or ready and unclaimed.
    Now,
    /// It has unfinished work, none of it claimed or ready.
    Next,
    /// It has no unfinished work.
    Later,
}

impl Lane {
    /// The lane's name.
    pub fn name(self) -> &'static str {
        match self {
            Lane::Now => "Now",
            Lane::Next => "Next",
            Lane::Later => "Later",
        }
    }
}

/// A goal's four counts. An item counts in `open` and, if it is claimed or ready, in one of
/// `claimed` and `ready_unclaimed`; a done item only in `done`; a cancelled one nowhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Unfinished and carrying a `claim_id`.
    pub claimed: usize,
    /// Unfinished, `state = "ready"`, and carrying no `claim_id`.
    pub ready_unclaimed: usize,
    /// Unfinished: filed, and not `done` or `cancelled`.
    pub open: usize,
    /// `done`.
    pub done: usize,
}

impl Counts {
    /// The lane these counts put a goal in.
    pub fn lane(&self) -> Lane {
        if self.claimed + self.ready_unclaimed > 0 {
            Lane::Now
        } else if self.open > 0 {
            Lane::Next
        } else {
            Lane::Later
        }
    }
}

/// One open goal's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLine {
    /// The goal's id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// `active` or `paused`.
    pub status: String,
    /// Its workstream as recorded, which for an ungrouped goal may be absent or outside the
    /// vocabulary.
    pub workstream: Option<String>,
    /// Its lane.
    pub lane: Lane,
    /// Its counts.
    pub counts: Counts,
}

/// The open goals of one workstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The workstream, one of [`WORKSTREAMS`].
    pub workstream: String,
    /// Its open goals, by lane, then title, then id. Empty when it has none.
    pub goals: Vec<GoalLine>,
}

/// A task named in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRef {
    /// Its id.
    pub id: String,
    /// Its title.
    pub title: String,
}

/// What a snapshot computes. Everything here is a function of the records alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// One group per workstream, in [`WORKSTREAMS`] order.
    pub groups: Vec<Group>,
    /// Open goals with no workstream, or one outside the vocabulary (ROAD-5).
    pub ungrouped: Vec<GoalLine>,
    /// Unfinished tasks that advance no open goal.
    pub unlinked_open: usize,
    /// Done tasks that advance no open goal.
    pub unlinked_done: usize,
    /// Tasks, not cancelled, with more than one `advances` edge, counted for no goal.
    pub multiple_goals: Vec<TaskRef>,
    /// The goal statuses left out, each with how many goals hold it.
    pub omitted: Vec<(String, usize)>,
    /// How many goals were read, of every status.
    pub goals_read: usize,
    /// How many tasks were read.
    pub tasks_read: usize,
}

fn line(goal: &Goal, counts: Counts) -> GoalLine {
    GoalLine {
        id: goal.id.clone(),
        title: goal.title.clone(),
        status: goal.status.clone(),
        workstream: goal.workstream.clone(),
        lane: counts.lane(),
        counts,
    }
}

fn order(lines: &mut [GoalLine]) {
    lines.sort_by(|a, b| (a.lane, &a.title, &a.id).cmp(&(b.lane, &b.title, &b.id)));
}

pub(crate) fn draw(records: &Records) -> Snapshot {
    let mut counts: BTreeMap<&str, Counts> = records
        .goals
        .iter()
        .filter(|g| g.is_open())
        .map(|g| (g.id.as_str(), Counts::default()))
        .collect();

    let (mut unlinked_open, mut unlinked_done) = (0, 0);
    let mut multiple_goals = Vec::new();
    for task in &records.tasks {
        if task.goal_links.len() > 1 {
            if task.stage != "cancelled" {
                multiple_goals.push(TaskRef {
                    id: task.id.clone(),
                    title: task.title.clone(),
                });
            }
            continue;
        }
        let goal = task
            .goal_links
            .first()
            .and_then(|id| counts.get_mut(id.as_str()));
        match goal {
            Some(c) if task.is_unfinished() => {
                c.open += 1;
                if task.has_claim_id {
                    c.claimed += 1;
                } else if task.is_ready {
                    c.ready_unclaimed += 1;
                }
            }
            Some(c) if task.stage == "done" => c.done += 1,
            Some(_) => {}
            None if task.is_unfinished() => unlinked_open += 1,
            None if task.stage == "done" => unlinked_done += 1,
            None => {}
        }
    }

    let mut groups: Vec<Group> = WORKSTREAMS
        .iter()
        .map(|w| Group {
            workstream: (*w).to_owned(),
            goals: Vec::new(),
        })
        .collect();
    let mut ungrouped = Vec::new();
    for goal in records.goals.iter().filter(|g| g.is_open()) {
        let l = line(goal, counts[goal.id.as_str()]);
        match groups
            .iter_mut()
            .find(|g| Some(&g.workstream) == goal.workstream.as_ref())
        {
            Some(group) => group.goals.push(l),
            None => ungrouped.push(l),
        }
    }
    groups.iter_mut().for_each(|g| order(&mut g.goals));
    order(&mut ungrouped);

    let omitted = ["completed", "cancelled"]
        .iter()
        .map(|s| {
            (
                (*s).to_owned(),
                records.goals.iter().filter(|g| g.status == *s).count(),
            )
        })
        .collect();

    Snapshot {
        groups,
        ungrouped,
        unlinked_open,
        unlinked_done,
        multiple_goals,
        omitted,
        goals_read: records.goals.len(),
        tasks_read: records.tasks.len(),
    }
}
