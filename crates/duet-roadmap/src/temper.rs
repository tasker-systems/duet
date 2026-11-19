//! The `temper` CLI calls the tool makes: a closed set, three reads and one create.

use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::{Error, GOALS_CONTEXT, SNAPSHOT_CONTEXT, TASKS_CONTEXT};

/// Every call the tool makes to `temper`. The set is closed: there is no update, meta write, edge
/// write or delete, so no snapshot can be rewritten and no work record changed. It is exhaustive
/// deliberately: a new call is a change every [`Temper`] must answer, and a test must see.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// Every goal in [`GOALS_CONTEXT`], whatever its status.
    ListGoals,
    /// Every task in [`TASKS_CONTEXT`], with its open metadata.
    ListTasks,
    /// One task's edges.
    ShowEdges {
        /// The task.
        id: String,
    },
    /// The one write: a new snapshot resource. Its body goes on standard input.
    CreateSnapshot {
        /// The title.
        title: String,
        /// The body, markdown.
        body: String,
        /// The open metadata, a JSON object.
        open_meta: Value,
    },
}

impl Call {
    /// The arguments to `temper`, without the program name.
    pub fn argv(&self) -> Vec<String> {
        let args: Vec<&str> = match self {
            Call::ListGoals => vec![
                "resource",
                "list",
                "--type",
                "goal",
                "--context",
                GOALS_CONTEXT,
                "--all",
                "--with",
                "open-meta",
            ],
            Call::ListTasks => vec![
                "resource",
                "list",
                "--type",
                "task",
                "--context",
                TASKS_CONTEXT,
                "--all",
                "--with",
                "open-meta",
            ],
            Call::ShowEdges { id } => vec![
                "resource",
                "show",
                id,
                "--without",
                "body",
                "--without",
                "open-meta",
                "--with",
                "edges",
            ],
            Call::CreateSnapshot {
                title, open_meta, ..
            } => {
                let mut argv: Vec<String> = [
                    "resource",
                    "create",
                    "--type",
                    "research",
                    "--context",
                    SNAPSHOT_CONTEXT,
                    "--title",
                ]
                .map(str::to_owned)
                .into();
                argv.push(title.clone());
                argv.push("--open-meta".to_owned());
                argv.push(open_meta.to_string());
                argv.extend(["--body", "-", "--format", "json"].map(str::to_owned));
                return argv;
            }
        };
        let mut argv: Vec<String> = args.into_iter().map(str::to_owned).collect();
        argv.extend(["--format", "json"].map(str::to_owned));
        argv
    }

    /// What the call writes to `temper`'s standard input.
    pub fn stdin(&self) -> Option<&str> {
        match self {
            Call::CreateSnapshot { body, .. } => Some(body),
            _ => None,
        }
    }
}

impl fmt::Display for Call {
    /// The command as it would be typed, for error messages. The create's body is not shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "temper")?;
        for arg in self.argv() {
            if arg.contains([' ', '"', '{']) {
                write!(f, " '{arg}'")?;
            } else {
                write!(f, " {arg}")?;
            }
        }
        Ok(())
    }
}

/// Runs a [`Call`] and returns what `temper` printed on standard output.
///
/// An implementation returns [`Error::Unavailable`] when the call could not be made or failed.
/// Parsing and checking the answer is the caller's.
pub trait Temper {
    /// Run the call.
    fn run(&mut self, call: &Call) -> Result<String, Error>;
}

/// The `temper` CLI, as a child process.
#[derive(Debug, Clone)]
pub struct TemperCli {
    program: PathBuf,
}

impl TemperCli {
    /// Run `program` as `temper`.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        TemperCli {
            program: program.into(),
        }
    }
}

impl Default for TemperCli {
    /// `temper` from `PATH`.
    fn default() -> Self {
        TemperCli::new("temper")
    }
}

impl Temper for TemperCli {
    fn run(&mut self, call: &Call) -> Result<String, Error> {
        let unavailable = |detail: String| Error::Unavailable {
            command: call.to_string(),
            detail,
        };
        let mut child = Command::new(&self.program)
            .args(call.argv())
            .stdin(if call.stdin().is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| unavailable(format!("{}: {e}", self.program.display())))?;
        if let Some(input) = call.stdin() {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| unavailable("no stdin pipe".to_owned()))?;
            pipe.write_all(input.as_bytes())
                .map_err(|e| unavailable(format!("writing stdin: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| unavailable(e.to_string()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(unavailable(format!("{}: {}", out.status, stderr.trim())));
        }
        String::from_utf8(out.stdout).map_err(|e| unavailable(format!("output is not UTF-8: {e}")))
    }
}

/// Run a read and decode its answer: the first JSON value printed, since `temper` may print more
/// after it.
pub(crate) fn json(temper: &mut dyn Temper, call: &Call) -> Result<Value, Error> {
    let stdout = temper.run(call)?;
    serde_json::Deserializer::from_str(&stdout)
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| Error::Shape {
            command: call.to_string(),
            detail: "it printed nothing".to_owned(),
        })?
        .map_err(|e| Error::Shape {
            command: call.to_string(),
            detail: format!("its output is not JSON: {e}"),
        })
}

/// Make the create and return the new resource's id. Any failure here is
/// [`Error::WriteUncertain`]: the create was attempted, so it may have landed.
pub(crate) fn create(temper: &mut dyn Temper, call: Call) -> Result<String, Error> {
    let uncertain = |detail: String| Error::WriteUncertain {
        command: call.to_string(),
        detail,
    };
    let answer = json(temper, &call).map_err(|e| uncertain(e.to_string()))?;
    answer["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| uncertain("its answer names no id".to_owned()))
}
