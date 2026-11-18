//! End-to-end tests of the `duet` binary, and what they share: the binary itself, built for
//! the run, and a way to run it with no config but what a test gives it.
//!
//! # The binary under test is the working tree's
//!
//! This crate cannot depend on `duet-cli`, which has no library, so cargo does not build the
//! `duet` binary for it. Running whatever `target/debug/duet` is on disk would test some
//! earlier build, so [`bin`] runs `cargo build -p duet-cli --bin duet` once per test
//! process and runs the executable that build reports. Set `DUET_E2E_BIN` to run another
//! binary instead, such as one built from `main` to show that a new test fails there.
//!
//! nextest runs every test as its own process, so concurrent builds would relink the binary
//! while another test runs it. Its setup script, `scripts/nextest-duet-bin.sh`, makes the same
//! build once before the tests start and sets `DUET_E2E_BIN` to what it reports.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The environment variable naming a `duet` binary to run instead of building one.
pub const BIN_VAR: &str = "DUET_E2E_BIN";

/// The workspace root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `golden/`: what `duet` wrote from the samples before a part could name an end.
pub fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("golden")
}

/// A file in [`golden_dir`].
pub fn golden(name: &str) -> PathBuf {
    golden_dir().join(name)
}

/// The `duet` binary to run: `DUET_E2E_BIN` when it is set, or else the one a fresh
/// `cargo build -p duet-cli --bin duet` reports, built once per process.
///
/// # Panics
///
/// When `DUET_E2E_BIN` is not a file, or the build fails or reports no executable.
pub fn bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(path) = std::env::var_os(BIN_VAR) {
            let path = PathBuf::from(path);
            assert!(path.is_file(), "{BIN_VAR}={} is not a file", path.display());
            return path;
        }
        build()
    })
}

fn build() -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "build",
            "--locked",
            "-p",
            "duet-cli",
            "--bin",
            "duet",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root())
        .stderr(Stdio::inherit())
        .output()
        .expect("run cargo build");
    assert!(
        out.status.success(),
        "cargo build -p duet-cli --bin duet failed"
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "duet")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .expect("cargo build reported the duet executable")
}

/// A fresh scratch directory, unique to this process and call.
///
/// # Panics
///
/// When the directory cannot be made.
pub fn scratch(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "duet-e2e-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    // The path duet sees: its working directory comes back with symlinks resolved, and
    // macOS's temp dir is a symlink (/var is /private/var).
    dir.canonicalize().expect("scratch dir resolves")
}

/// Runs `duet` in `cwd`, with `user` as its user config directory (and `user/data` as its
/// data directory) and `stdin` on standard input. With `cwd` a scratch directory, no project
/// `.duet` is found above it. Neither the developer's `DUET_PROJECT` nor who they are
/// (`DUET_ACTOR`, `CLAUDECODE`) reaches it.
///
/// # Panics
///
/// When the binary cannot be started or its input written.
pub fn duet_with<S: AsRef<std::ffi::OsStr>>(
    cwd: &Path,
    user: &Path,
    args: &[S],
    stdin: &[u8],
) -> Output {
    duet_env(cwd, user, args, stdin, &[])
}

/// Runs `duet` as [`duet_with`] does, with `env` set too, such as `DUET_ACTOR`.
///
/// # Panics
///
/// When the binary cannot be started or its input written.
pub fn duet_env<S: AsRef<std::ffi::OsStr>>(
    cwd: &Path,
    user: &Path,
    args: &[S],
    stdin: &[u8],
    env: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(cwd)
        .env("DUET_CONFIG_DIR", user)
        // Nor does a SoundFont the developer has installed: duet play looks here.
        .env("DUET_DATA_DIR", user.join("data"))
        .env_remove("DUET_PROJECT")
        .env_remove("DUET_ACTOR")
        .env_remove("CLAUDECODE");
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run duet");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("duet output")
}

/// Runs `duet` in `cwd` with no config at all: an empty user config directory under `cwd`.
pub fn duet<S: AsRef<std::ffi::OsStr>>(cwd: &Path, args: &[S]) -> Output {
    duet_with(cwd, &cwd.join("no-config"), args, b"")
}

/// Asserts that `duet` succeeded, showing its standard error when it did not.
///
/// # Panics
///
/// When it did not.
pub fn ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A path as `&str`, for argument lists.
///
/// # Panics
///
/// When the path is not UTF-8.
pub fn s(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// Writes `text`, a part's notes view, to `dir/name` and reads it into a part document beside
/// it with `duet apply --as notes`, the one way duet reads a view's text: `take.notes` becomes
/// `take.duet`. Returns the document's path.
///
/// # Panics
///
/// When the text cannot be written or `duet apply` refuses it.
pub fn notes_doc(dir: &Path, name: &str, text: &str) -> PathBuf {
    applied(dir, name, text, "notes")
}

/// Writes `text`, a drum grid's grid view, to `dir/name` and reads it into a drum-grid document
/// beside it with `duet apply --as grid`: `beat.drums` becomes `beat.duet`. Returns the
/// document's path.
///
/// # Panics
///
/// When the text cannot be written or `duet apply` refuses it.
pub fn grid_doc(dir: &Path, name: &str, text: &str) -> PathBuf {
    applied(dir, name, text, "grid")
}

fn applied(dir: &Path, name: &str, text: &str, view: &str) -> PathBuf {
    let from = dir.join(name);
    std::fs::write(&from, text).expect("write the view");
    let doc = from.with_extension("duet");
    let out = duet(dir, &["apply", s(&from), "--as", view, "-o", s(&doc)]);
    ok(&out, &format!("duet apply {name}"));
    doc
}

/// The text of a document's view, `duet view <doc> --as <view>`, with its header line, the
/// first, left out: what `--to text` or `--to drums` once wrote. `args` are more flags, such
/// as `--snap-levels`.
///
/// # Panics
///
/// When `duet view` refuses the document.
pub fn view_text(dir: &Path, doc: &Path, view: &str, args: &[&str]) -> String {
    let mut all = vec!["view", s(doc), "--as", view];
    all.extend_from_slice(args);
    let out = duet(dir, &all);
    ok(&out, &format!("duet view {}", doc.display()));
    let text = String::from_utf8(out.stdout).expect("a view is UTF-8");
    text.split_once('\n')
        .map_or(String::new(), |(_, rest)| rest.to_owned())
}

/// A part document's text with its body's `id` made all zeros, so two documents of one part,
/// each with an id of its own, compare byte for byte in everything else. The text is the
/// document, unchanged, when it holds no id.
#[must_use]
pub fn without_id(document: &[u8]) -> Vec<u8> {
    const KEY: &[u8] = b"\n    \"id\": \"";
    let mut out = document.to_vec();
    if let Some(at) = out.windows(KEY.len()).position(|w| w == KEY) {
        let start = at + KEY.len();
        for b in out.iter_mut().skip(start).take(36) {
            if *b != b'-' {
                *b = b'0';
            }
        }
    }
    out
}

/// A part document's body as JSON, with its `id` left out: what two documents of one part, each
/// with an id of its own, hold alike.
#[must_use]
pub fn body_without_id(document: &serde_json::Value) -> serde_json::Value {
    let mut body = document["body"].clone();
    if let Some(fields) = body.as_object_mut() {
        fields.remove("id");
    }
    body
}
