//! duet's configuration: `duet.toml` and `duet.local.toml`, in the user's config directory
//! and in a project's `.duet/`, laid over the built-in defaults.
//!
//! Files are read in this order, each over the ones before it:
//!
//! 1. the built-in defaults
//! 2. `<user config dir>/duet.toml`
//! 3. `<user config dir>/duet.local.toml`
//! 4. `<project>/.duet/duet.toml`, where the project is the nearest directory, from the
//!    working directory up, that holds a `.duet` directory
//! 5. `<project>/.duet/duet.local.toml`
//!
//! The user config directory is the platform's, from `etcetera` (XDG on macOS and Linux,
//! so `~/.config/duet`; Known Folders on Windows), or `DUET_CONFIG_DIR` when it is set.
//!
//! ```toml
//! [song]
//! end = "last-bar"            # or { round-up = 4 }
//!
//! [lanes.shaker-2]            # a lane of the gm map, added or changed
//! key = 82
//! aliases = ["sh2"]
//! ```
//!
//! A lane table changes only the fields it sets, and adds a lane when the name is new; no
//! file removes a built-in lane. Each file must leave a valid map. A file that breaks a
//! rule is refused, naming its path and the rule.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use duet_core::EndRule;
use duet_drums::{LaneMap, LanePatch};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::Deserialize;

/// The environment variable that names the user config directory, overriding the
/// platform's.
pub const CONFIG_DIR_VAR: &str = "DUET_CONFIG_DIR";

const BUILT_IN: &str = "built-in";

/// Where duet looks for its files.
pub struct Dirs {
    /// The user config directory, and what chose it.
    pub user: Option<(PathBuf, &'static str)>,
    /// The project's `.duet` directory, if one is found.
    pub project: Option<PathBuf>,
}

impl Dirs {
    /// The user config directory (`DUET_CONFIG_DIR`, or the platform's) and the nearest
    /// project `.duet` directory from the working directory up.
    pub fn find() -> Result<Dirs> {
        let user = match std::env::var_os(CONFIG_DIR_VAR).filter(|v| !v.is_empty()) {
            Some(dir) => Some((PathBuf::from(dir), CONFIG_DIR_VAR)),
            None => choose_app_strategy(AppStrategyArgs {
                top_level_domain: String::new(),
                author: String::new(),
                app_name: "duet".to_owned(),
            })
            .ok()
            .map(|s| (s.config_dir(), "the platform's config directory")),
        };
        let cwd = std::env::current_dir().context("finding the working directory")?;
        let project = cwd
            .ancestors()
            .map(|dir| dir.join(".duet"))
            .find(|dir| dir.is_dir());
        Ok(Dirs { user, project })
    }

    /// Every file duet reads, in the order it reads them, whether or not it exists.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for dir in self.user.iter().map(|(d, _)| d).chain(&self.project) {
            out.push(dir.join("duet.toml"));
            out.push(dir.join("duet.local.toml"));
        }
        out
    }
}

/// The merged configuration, and where each value came from.
pub struct Config {
    /// How a song that names no end gets one when a file is written.
    pub end: EndRule,
    end_from: String,
    /// The lanes `map gm` stands for: the built-in map with every file's lanes laid over.
    pub lanes: LaneMap,
    /// (lane, field, source) for every lane field a file set.
    lane_from: Vec<(String, &'static str, String)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    song: Option<RawSong>,
    lanes: Option<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSong {
    end: Option<toml::Value>,
}

fn end_rule(value: &toml::Value) -> Result<EndRule> {
    let bad = || anyhow!("[song] end is \"last-bar\" or {{ round-up = <bars> }}");
    match value {
        toml::Value::String(s) if s == "last-bar" => Ok(EndRule::LastBar),
        toml::Value::Table(t) if t.len() == 1 => {
            let n = t
                .get("round-up")
                .and_then(toml::Value::as_integer)
                .ok_or_else(bad)?;
            match u32::try_from(n) {
                Ok(n) if n > 0 => Ok(EndRule::RoundUp(n)),
                _ => bail!("[song] end: round-up is a whole number of bars, 1 or more, not {n}"),
            }
        }
        _ => Err(bad()),
    }
}

impl Config {
    /// The built-in defaults, with no files read.
    pub fn built_in() -> Config {
        Config {
            end: EndRule::LastBar,
            end_from: BUILT_IN.to_owned(),
            lanes: LaneMap::gm(),
            lane_from: Vec::new(),
        }
    }

    /// Reads every file there is, in order, over the built-in defaults.
    pub fn load() -> Result<Config> {
        Config::load_from(&Dirs::find()?.files())
    }

    /// Reads these files, those that exist, in order, over the built-in defaults.
    pub fn load_from(files: &[PathBuf]) -> Result<Config> {
        let mut config = Config::built_in();
        for path in files {
            if path.is_file() {
                config
                    .apply(path)
                    .with_context(|| path.display().to_string())?;
            }
        }
        Ok(config)
    }

    fn apply(&mut self, path: &Path) -> Result<()> {
        let text = fs::read_to_string(path)?;
        let raw: RawFile = toml::from_str(&text)?;
        let from = path.display().to_string();
        if let Some(end) = raw.song.and_then(|s| s.end) {
            self.end = end_rule(&end)?;
            self.end_from.clone_from(&from);
        }
        let mut patches = Vec::new();
        for (name, value) in raw.lanes.unwrap_or_default() {
            let patch: LanePatch = value
                .try_into()
                .map_err(|e: toml::de::Error| anyhow!("[lanes.{name}]: {}", e.message()))?;
            for (field, set) in [
                ("key", patch.key.is_some()),
                ("group", patch.group.is_some()),
                ("aliases", patch.aliases.is_some()),
            ] {
                if set {
                    self.lane_from
                        .retain(|(l, f, _)| !(l == &name && *f == field));
                    self.lane_from.push((name.clone(), field, from.clone()));
                }
            }
            patches.push((name, patch));
        }
        self.lanes = self.lanes.patched(&patches)?;
        Ok(())
    }

    fn lane_source(&self, lane: &str, field: &str) -> &str {
        self.lane_from
            .iter()
            .find(|(l, f, _)| l == lane && *f == field)
            .map_or(BUILT_IN, |(_, _, s)| s)
    }

    /// The merged configuration as a `duet.toml`, each value followed by the file it came
    /// from, or `built-in`.
    pub fn show(&self) -> String {
        let mut out = String::from("# duet config: every value, and where it came from\n\n");
        let end = match self.end {
            EndRule::RoundUp(n) => format!("{{ round-up = {n} }}"),
            _ => "\"last-bar\"".to_owned(),
        };
        // Writing to a String cannot fail.
        let _ = writeln!(out, "[song]\nend = {end}  # {}", self.end_from);
        for lane in self.lanes.lanes() {
            let src = |field| self.lane_source(&lane.name, field);
            let aliases: Vec<String> = lane.aliases.iter().map(|a| format!("\"{a}\"")).collect();
            let _ = write!(
                out,
                "\n[lanes.{}]\nkey = {}  # {}\ngroup = \"{}\"  # {}\naliases = [{}]  # {}\n",
                lane.name,
                lane.key,
                src("key"),
                lane.group,
                src("group"),
                aliases.join(", "),
                src("aliases"),
            );
        }
        out
    }
}

/// What `duet config path` prints: the directories, and each file with whether it
/// exists, in the order they are read.
pub fn describe_paths(dirs: &Dirs) -> String {
    let mut out = String::new();
    match &dirs.user {
        Some((dir, from)) => {
            let _ = writeln!(out, "user config directory: {} ({from})", dir.display());
        }
        None => out.push_str("user config directory: none (no home directory found)\n"),
    }
    match &dirs.project {
        Some(dir) => {
            let _ = writeln!(out, "project config directory: {}", dir.display());
        }
        None => out.push_str("project config directory: none (no .duet directory found)\n"),
    }
    out.push_str("files, read in this order over the built-in defaults:\n");
    for file in dirs.files() {
        let state = if file.is_file() { "found" } else { "not found" };
        let _ = writeln!(out, "  {}  ({state})", file.display());
    }
    out
}
