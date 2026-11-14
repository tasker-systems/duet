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

/// `golden/`: what `duet` wrote from the samples before a song could name an end.
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
    dir
}

/// Runs `duet` in `cwd`, with `user` as its user config directory and `stdin` on standard
/// input. With `cwd` a scratch directory, no project `.duet` is found above it.
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
    let mut child = Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("DUET_CONFIG_DIR", user)
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
