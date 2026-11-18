//! `duet init` and project discovery (the project design's §3 and §5, build task 2). `init` writes
//! exactly `.duet/project.duet` and, when absent, a `.duet/duet.toml` of comments; it refuses a
//! project that exists, a project inside another without `--nested`, and the home directory; it
//! never writes a git file or prints a git line. A project is the nearest directory up whose
//! `.duet/` holds `project.duet`, or the one `DUET_PROJECT` names, and a `.duet/` without one is
//! config only, as it has always been.

use std::path::Path;
use std::process::Output;

use duet_e2e::{duet, duet_env, ok, s, scratch};
use duet_schemas::{Document, Project};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every file under `dir`, relative to it, sorted.
fn files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn read_project(root: &Path) -> Project {
    let text = std::fs::read_to_string(root.join(".duet/project.duet")).unwrap();
    Document::<Project>::read(&text).unwrap().body
}

#[test]
fn init_in_an_empty_directory_writes_exactly_its_two_files_and_init_again_is_refused() {
    let dir = scratch("init-empty");
    let root = dir.join("porch");
    let out = duet(&dir, &["init", "porch"]);
    ok(&out, "init");
    assert_eq!(
        files(&root),
        [".duet/duet.toml", ".duet/project.duet"],
        "init writes these and nothing else: no journal, no song, no other directory"
    );
    assert_eq!(
        stdout(&out),
        "wrote porch/.duet/project.duet\nwrote porch/.duet/duet.toml\n"
    );
    let project = read_project(&root);
    assert_eq!(project.title, "porch", "the title is the directory's name");
    assert!(project.takes.is_empty() && project.defaults.is_none());
    let config = std::fs::read_to_string(root.join(".duet/duet.toml")).unwrap();
    assert!(
        config
            .lines()
            .all(|line| line.is_empty() || line.starts_with('#')),
        "every line of the config is a comment:\n{config}"
    );
    for named in ["[part]", "[play]", "[lanes."] {
        assert!(config.contains(named), "it names {named}:\n{config}");
    }
    ok(
        &duet(&root, &["config", "show"]),
        "the commented config reads",
    );

    let before = std::fs::read(root.join(".duet/project.duet")).unwrap();
    let again = duet(&root, &["init", "--title", "Other"]);
    assert!(!again.status.success(), "init again is refused");
    assert!(
        stderr(&again).contains(".duet/project.duet exists"),
        "{}",
        stderr(&again)
    );
    assert_eq!(
        std::fs::read(root.join(".duet/project.duet")).unwrap(),
        before,
        "the project is left as it was"
    );

    let titled = dir.join("lift");
    ok(
        &duet(&dir, &["init", "lift", "--title", "Lift Off"]),
        "--title",
    );
    assert_eq!(read_project(&titled).title, "Lift Off");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn init_writes_no_git_file_and_prints_no_git_line() {
    let dir = scratch("init-git");
    let out = duet(&dir, &["init"]);
    ok(&out, "init");
    for file in files(&dir) {
        assert!(!file.contains(".git"), "init wrote {file}");
    }
    let said = format!("{}{}", stdout(&out), stderr(&out)).to_ascii_lowercase();
    assert!(!said.contains("git"), "init printed a git line:\n{said}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn init_adopts_a_config_only_duet_directory_keeping_its_config_byte_for_byte() {
    let dir = scratch("init-config");
    let config = b"# mine\n[part]\nend = { round-up = 2 }   # spaced as I like\n";
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    std::fs::write(dir.join(".duet/duet.toml"), config).unwrap();
    std::fs::write(dir.join(".duet/duet.local.toml"), b"[play]\n").unwrap();
    let out = duet(&dir, &["init"]);
    ok(&out, "init");
    assert_eq!(
        stdout(&out),
        "wrote .duet/project.duet\nkept .duet/duet.toml, as it was\n"
    );
    assert_eq!(std::fs::read(dir.join(".duet/duet.toml")).unwrap(), config);
    assert_eq!(
        std::fs::read(dir.join(".duet/duet.local.toml")).unwrap(),
        b"[play]\n"
    );
    assert_eq!(
        files(&dir),
        [
            ".duet/duet.local.toml",
            ".duet/duet.toml",
            ".duet/project.duet"
        ]
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_config_only_duet_directory_is_no_project() {
    let dir = scratch("config-only");
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    std::fs::write(dir.join(".duet/duet.toml"), "[part]\nend = \"last-bar\"\n").unwrap();
    let below = dir.join("sketches");
    std::fs::create_dir_all(&below).unwrap();
    let path = duet(&below, &["project", "path"]);
    ok(&path, "project path");
    assert!(
        stdout(&path).starts_with("project: none\n"),
        "{}",
        stdout(&path)
    );
    let show = duet(&below, &["project", "show"]);
    assert!(!show.status.success(), "no project to show");
    assert!(
        stderr(&show).contains("no project here") && stderr(&show).contains("duet init"),
        "{}",
        stderr(&show)
    );
    // A command that writes a document writes no journal: there is no project to keep one.
    ok(&duet(&dir, &["song", "new", "porch"]), "song new");
    assert_eq!(
        files(&dir),
        [".duet/duet.toml", "songs/porch/song.duet"],
        "no journal beside a config-only .duet/"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn project_path_names_the_project_found_walking_up_and_the_one_duet_project_names() {
    let dir = scratch("discovery");
    let root = dir.join("porch");
    ok(&duet(&dir, &["init", "porch"]), "init");
    let deep = root.join("songs/a/b");
    std::fs::create_dir_all(&deep).unwrap();
    let found = duet(&deep, &["project", "path"]);
    ok(&found, "project path");
    assert_eq!(
        stdout(&found),
        format!(
            "project: {}\nfound walking up from {}: the nearest .duet/ that holds project.duet\n",
            root.display(),
            deep.display()
        )
    );

    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let named = duet_env(
        &elsewhere,
        &elsewhere.join("no-config"),
        &["project", "path"],
        b"",
        &[("DUET_PROJECT", s(&root))],
    );
    ok(&named, "project path, named");
    assert_eq!(
        stdout(&named),
        format!(
            "project: {}\nnamed by DUET_PROJECT={}\n",
            root.display(),
            root.display()
        )
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn project_show_prints_the_document_each_take_and_where_it_was_found_and_every_song() {
    let dir = scratch("project-show");
    std::fs::create_dir_all(dir.join(".duet")).unwrap();
    let example = duet_e2e::root().join("crates/duet-schemas/schemas/project/examples/1.0.duet");
    std::fs::copy(example, dir.join(".duet/project.duet")).unwrap();
    // wayfaring is copied in; blood-sport is named where it is, and that file is gone.
    std::fs::create_dir_all(dir.join("takes/wayfaring")).unwrap();
    std::fs::write(dir.join("takes/wayfaring/wayfaring-stranger.flac"), b"fLaC").unwrap();
    let gone = dir.join("phone/Blood Sport 3.m4a");
    std::fs::write(
        dir.join(".duet/takes.local.toml"),
        format!("[blood-sport]\npath = {:?}\n", s(&gone)),
    )
    .unwrap();
    ok(
        &duet(
            &dir,
            &["song", "new", "stranger", "--title", "Wayfaring Stranger"],
        ),
        "song new",
    );
    std::fs::create_dir_all(dir.join("songs/empty")).unwrap();

    let out = duet(&dir, &["project", "show"]);
    ok(&out, "project show");
    let shown = stdout(&out);
    for want in [
        format!("project {}: Porch\n", dir.display()),
        "created 2026-11-17, by duet 0.3\n".to_owned(),
        "defaults tempo 92 bpm, meter 3/4, key Ab major, tuning +0c, grid 1/16\n".to_owned(),
        "  wayfaring (wayfaring-stranger.flac), sha256:9f2c41d07b3e8a65c1f0d92e4b7a3c58e06d1f9a2b4c7e8d3f5a6b0c9d1e2f34, 44100 Hz, 1 channel, 976080 frames, added 2026-11-17, given tempo 92 bpm; found at takes/wayfaring/wayfaring-stranger.flac\n".to_owned(),
        format!(
            "  blood-sport (Blood Sport 3.m4a), sha256:41ab07e3c9d25f18b6a4e0c7d3f92b5a8e1c6d4f7a0b3e9c2d5f8a1b4c7e0d36, 48000 Hz, 1 channel, 1203200 frames, added 2026-11-18; not found: looked at takes/blood-sport/Blood Sport 3.m4a and {}, named in .duet/takes.local.toml\n",
            gone.display()
        ),
        "songs\n  songs/empty/ holds no song.duet\n  stranger: Wayfaring Stranger (".to_owned(),
        "), at idea\n".to_owned(),
    ] {
        assert!(shown.contains(&want), "project show should hold\n{want}\n{shown}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_nested_init_without_nested_and_an_init_in_the_home_directory_are_refused_by_name() {
    let dir = scratch("init-refused");
    let root = dir.join("porch");
    ok(&duet(&dir, &["init", "porch"]), "init");
    let inner = root.join("old");
    std::fs::create_dir_all(&inner).unwrap();
    let nested = duet(&inner, &["init"]);
    assert!(!nested.status.success(), "a nested init is refused");
    assert!(
        stderr(&nested).contains(&format!("inside the project {}", root.display()))
            && stderr(&nested).contains("--nested"),
        "{}",
        stderr(&nested)
    );
    assert_eq!(files(&inner), Vec::<String>::new(), "nothing is written");
    ok(&duet(&inner, &["init", "--nested"]), "init --nested");
    let path = duet(&inner, &["project", "path"]);
    assert!(
        stdout(&path).starts_with(&format!("project: {}\n", inner.display())),
        "the nearest project wins: {}",
        stdout(&path)
    );

    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let refused = duet_env(
        &home,
        &dir.join("no-config"),
        &["init"],
        b"",
        &[("HOME", s(&home))],
    );
    assert!(!refused.status.success(), "init in the home directory");
    assert!(
        stderr(&refused).contains(&format!("{} is the home directory", home.display())),
        "{}",
        stderr(&refused)
    );
    assert!(!home.join(".duet").exists(), "no .duet/ is made there");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
