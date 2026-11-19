//! What an action runs with: the project it runs in, if any, the configuration, and the record of
//! every document it writes.

use std::cell::RefCell;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use duet_schemas::journal::{By, DocDigest, DocWritten, Event, Id, RelPath};
use duet_schemas::read_envelope;

use crate::config::Config;

/// The environment variable that names a project's root, instead of finding one from the working
/// directory up.
pub const PROJECT_VAR: &str = "DUET_PROJECT";

/// The project's document, under its root.
pub const PROJECT_FILE: &str = ".duet/project.duet";

/// A project an action runs in: its root, its name (the root directory's), and how it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub found: Found,
}

/// How a project was found (the project design's §5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Found {
    /// `DUET_PROJECT` named it, and this is what it said.
    Named(std::ffi::OsString),
    /// It is the nearest directory, from this one up, whose `.duet/` holds `project.duet`.
    WalkedUp(PathBuf),
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Named(named) => {
                write!(f, "named by {PROJECT_VAR}={}", Path::new(named).display())
            }
            Self::WalkedUp(from) => write!(
                f,
                "found walking up from {}: the nearest .duet/ that holds project.duet",
                from.display()
            ),
        }
    }
}

impl Project {
    /// The project an action run from `cwd` runs in (the project design's §5, ruled): the one
    /// `DUET_PROJECT` names, or else the nearest directory from `cwd` up whose `.duet/` holds
    /// `project.duet`. Outside a project there is none, and no journal.
    ///
    /// # Errors
    ///
    /// When `DUET_PROJECT` names a directory that holds no `.duet/project.duet`.
    pub fn find(cwd: &Path) -> Result<Option<Self>> {
        if let Some(named) = std::env::var_os(PROJECT_VAR).filter(|v| !v.is_empty()) {
            let root = cwd.join(&named);
            if !root.join(PROJECT_FILE).is_file() {
                bail!(
                    "{PROJECT_VAR} names {}, which holds no {PROJECT_FILE}: it is no project",
                    root.display()
                );
            }
            return Self::at(&root, Found::Named(named)).map(Some);
        }
        Self::above(cwd)
            .map(|root| Self::at(root, Found::WalkedUp(cwd.to_path_buf())))
            .transpose()
    }

    /// The nearest directory, from `dir` up, whose `.duet/` holds `project.duet`: the walk
    /// [`Self::find`] makes when `DUET_PROJECT` is not set.
    #[must_use]
    pub fn above(dir: &Path) -> Option<&Path> {
        dir.ancestors().find(|dir| dir.join(PROJECT_FILE).is_file())
    }

    /// The project whose root is `root`, found as `found` says.
    ///
    /// # Errors
    ///
    /// When the root cannot be made absolute.
    pub fn at(root: &Path, found: Found) -> Result<Self> {
        let root = fs::canonicalize(root).with_context(|| root.display().to_string())?;
        let name = root.file_name().map_or_else(
            || "project".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        Ok(Self { root, name, found })
    }
}

/// Who is acting, through `face` (decision 8): `DUET_ACTOR` if it is set, else `claude` when
/// `CLAUDECODE=1`, else `owner`. A label, not authentication.
#[must_use]
pub fn actor(face: &str) -> By {
    let who = match std::env::var("DUET_ACTOR") {
        Ok(who) if !who.is_empty() => who,
        _ if std::env::var("CLAUDECODE").is_ok_and(|v| v == "1") => "claude".to_owned(),
        _ => "owner".to_owned(),
    };
    By::new(who, face)
}

/// A fresh UUIDv7: now, and 74 bits from the operating system's randomness.
///
/// # Errors
///
/// When the operating system gives no randomness.
pub fn fresh_id() -> Result<Id> {
    let mut random = [0u8; 10];
    getrandom::fill(&mut random).map_err(|e| anyhow::anyhow!("no randomness for an id: {e}"))?;
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Ok(Id::from_parts(ms, random))
}

/// What an action runs with: the project, the configuration, and every document it has written.
#[derive(Debug)]
pub struct Context {
    project: Option<Project>,
    config: Config,
    written: RefCell<Vec<DocWritten>>,
    emitted: RefCell<Vec<Event>>,
}

impl Context {
    /// A context in `project`, if any, with `config`.
    #[must_use]
    pub fn new(project: Option<Project>, config: Config) -> Self {
        Self {
            project,
            config,
            written: RefCell::new(Vec::new()),
            emitted: RefCell::new(Vec::new()),
        }
    }

    #[must_use]
    pub const fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Every document written so far, in the order written, as `doc.written` bodies.
    #[must_use]
    pub fn written(&self) -> Vec<DocWritten> {
        self.written.borrow().clone()
    }

    /// Records an event the action caused besides its `doc.written`s, such as `take.added`.
    /// Inside a project the face appends it to the journal after them; outside one there is no
    /// journal, and nothing is recorded.
    pub fn emit(&self, event: impl Into<Event>) {
        if self.project.is_some() {
            self.emitted.borrow_mut().push(event.into());
        }
    }

    /// Every event [`Self::emit`] recorded, in order.
    #[must_use]
    pub fn emitted(&self) -> Vec<Event> {
        self.emitted.borrow().clone()
    }

    /// Writes `bytes` to `path`, or to standard output for `-`, and records it
    /// ([`Self::wrote`]), replacing whatever file was there.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if path == Path::new("-") {
            return std::io::stdout()
                .write_all(bytes)
                .context("writing standard output");
        }
        let replaced = path.exists();
        fs::write(path, bytes).with_context(|| path.display().to_string())?;
        self.wrote(path, replaced)
    }

    /// Records a file an action has written, however it wrote it, and whether it replaced one
    /// that was there. Inside a project, a document (a `.duet`, or a `.duet.jsonl` stream) under
    /// the project's root is recorded as a `doc.written`: its path from the root, its kind and
    /// version, and its body's digest. Anything else, and anything outside a project, is not.
    ///
    /// A document's digest is the FNV-1a of its body as compact JSON, as a view's `from` names
    /// one. A stream's is of its head as compact JSON followed by each later line, each ended by
    /// a newline: everything but its envelope.
    ///
    /// # Errors
    ///
    /// When a document just written cannot be read back.
    pub fn wrote(&self, path: &Path, replaced: bool) -> Result<()> {
        let Some(project) = &self.project else {
            return Ok(());
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let stream = name.ends_with(".duet.jsonl");
        if !stream && !name.ends_with(".duet") {
            return Ok(());
        }
        let Some(relative) = relative_to(&project.root, path)? else {
            return Ok(());
        };
        let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
        let (first, rest) = if stream {
            text.split_once('\n').unwrap_or((&text, ""))
        } else {
            (text.as_str(), "")
        };
        let envelope = read_envelope(first).with_context(|| path.display().to_string())?;
        let framed: Framed<'_> =
            serde_json::from_str(first).with_context(|| path.display().to_string())?;
        let digest = if stream {
            let mut bytes = minified(framed.head.map_or("", |h| h.get())).into_bytes();
            bytes.push(b'\n');
            bytes.extend_from_slice(rest.as_bytes());
            DocDigest::of(&bytes)
        } else {
            DocDigest::of(minified(framed.body.map_or("", |b| b.get())).as_bytes())
        };
        self.written.borrow_mut().push(DocWritten::new(
            relative,
            envelope.kind,
            envelope.version,
            digest,
            replaced,
        ));
        Ok(())
    }
}

/// A document's body, or a stream's head, as written.
#[derive(serde::Deserialize)]
struct Framed<'a> {
    #[serde(borrow)]
    body: Option<&'a serde_json::value::RawValue>,
    #[serde(borrow)]
    head: Option<&'a serde_json::value::RawValue>,
}

/// JSON with the whitespace between its tokens taken out: what serde_json's compact writer
/// writes for the value its pretty writer wrote.
fn minified(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let (mut in_string, mut escaped) = (false, false);
    for c in json.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !c.is_whitespace() {
            out.push(c);
        }
    }
    out
}

/// `path` relative to `root`, when it lies under it.
fn relative_to(root: &Path, path: &Path) -> Result<Option<RelPath>> {
    let full = fs::canonicalize(path).with_context(|| path.display().to_string())?;
    let Ok(rest) = full.strip_prefix(root) else {
        return Ok(None);
    };
    let parts: Vec<String> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    Ok(parts.join("/").parse().ok())
}

#[cfg(test)]
mod tests {
    use super::minified;

    #[test]
    fn minifying_pretty_json_gives_the_compact_writers_bytes() {
        let value = serde_json::json!({
            "a": [1, 2.5, {"b": "x y\n\"z\" \\"}], "c": null, "d": " spaced "
        });
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        assert_eq!(minified(&pretty), serde_json::to_string(&value).unwrap());
    }
}
