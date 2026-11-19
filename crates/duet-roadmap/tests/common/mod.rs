//! A `temper` that answers from `tests/fixture/records.json` and records every call it was asked
//! to make, so a test can see both what the tool drew and what it tried to write.

#![expect(
    dead_code,
    reason = "each test binary uses a different part of the fake"
)]

use std::path::Path;
use std::time::{Duration, SystemTime};

use duet_roadmap::{Call, Error, Temper};
use serde_json::{Value, json};

/// The id the fake gives the snapshot it creates.
pub const CREATED_ID: &str = "50000000-0000-7000-8000-000000000001";

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/records.json");
    let text = std::fs::read_to_string(&path).expect("the fixture is readable");
    serde_json::from_str(&text).expect("the fixture is JSON")
}

/// How the fake departs from the fixture.
#[derive(Debug, Clone, Default)]
pub enum Fault {
    #[default]
    None,
    /// The goal listing says it was truncated.
    TruncatedGoals,
    /// The task listing returns fewer rows than its total.
    ShortTasks,
    /// Reading this task's edges fails, as an unreachable service would.
    FailShow(String),
    /// The goal listing is not JSON.
    Garbage,
}

#[derive(Debug, Default)]
pub struct Fake {
    pub records: Value,
    /// Serve every listing and edge list in reverse order, as another server might.
    pub reverse: bool,
    pub fault: Fault,
    pub calls: Vec<Call>,
}

impl Fake {
    pub fn new() -> Self {
        Fake {
            records: fixture(),
            ..Fake::default()
        }
    }

    pub fn with_fault(fault: Fault) -> Self {
        Fake {
            fault,
            ..Fake::new()
        }
    }

    pub fn creates(&self) -> Vec<&Call> {
        self.calls
            .iter()
            .filter(|c| matches!(c, Call::CreateSnapshot { .. }))
            .collect()
    }

    fn page(&self, key: &str) -> Value {
        let mut page = self.records[key].clone();
        if self.reverse {
            page["rows"].as_array_mut().expect("rows").reverse();
        }
        page
    }
}

impl Temper for Fake {
    fn run(&mut self, call: &Call) -> Result<String, Error> {
        self.calls.push(call.clone());
        let answer = match call {
            Call::ListGoals => {
                if let Fault::Garbage = self.fault {
                    return Ok("not json at all".to_owned());
                }
                let mut page = self.page("goals");
                if let Fault::TruncatedGoals = self.fault {
                    page["truncated"] = json!(true);
                }
                page
            }
            Call::ListTasks => {
                let mut page = self.page("tasks");
                if let Fault::ShortTasks = self.fault {
                    page["rows"].as_array_mut().expect("rows").pop();
                    let returned = page["rows"].as_array().expect("rows").len();
                    page["returned"] = json!(returned);
                }
                page
            }
            Call::ShowEdges { id } => {
                if let Fault::FailShow(failing) = &self.fault
                    && failing == id
                {
                    return Err(Error::Unavailable {
                        command: call.to_string(),
                        detail: "connection refused".to_owned(),
                    });
                }
                let mut show = self.records["shows"][id.as_str()].clone();
                if self.reverse {
                    show["edges"]["outgoing"]
                        .as_array_mut()
                        .expect("outgoing")
                        .reverse();
                }
                show
            }
            Call::CreateSnapshot { title, .. } => json!({ "id": CREATED_ID, "title": title }),
        };
        // `temper` may print more after its JSON document; the reader must take only the first.
        Ok(format!("{answer}\nwarning: a newer temper is available\n"))
    }
}

/// A clock that starts at `start` and moves one second per reading.
pub fn clock(start: &str) -> impl FnMut() -> SystemTime {
    let mut at = duet_roadmap::parse_utc(start).expect("a valid start time");
    move || {
        let now = at;
        at += Duration::from_secs(1);
        now
    }
}
