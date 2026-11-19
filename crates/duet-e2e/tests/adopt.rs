//! Adopting scratch work (the project design's §7, build task 5, ruled to stand as written in
//! temper `01a1f1b0`): `duet project adopt <dir>` prints a plan and writes nothing; `--song`
//! names the song its parts and comps go to; `--apply` copies what the plan said, byte for byte,
//! and never moves or rewrites anything.
//!
//! Every file here is one the test writes itself: nothing comes from `fixtures/`. The scratch
//! directory is made as duet's scratch work was: a take captured beside its recording, a reading
//! from before the take digest, a part from before the `part` rename, a loose part and its notes
//! view, a comp, a moments file, a review sheet, and a file duet has no use for.

#[path = "capture/audio.rs"]
#[expect(
    dead_code,
    reason = "the capture tests' shared generators: these tests use the tones alone"
)]
mod audio;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use duet_e2e::{duet, notes_doc, ok, s, scratch};
use duet_schemas::{Document, Project, Version, schema_url};
use serde_json::Value;

const NOTES: [(u8, f64); 3] = [(60, 0.6), (64, 0.6), (67, 0.9)];
const OTHER: [(u8, f64); 3] = [(62, 0.5), (65, 0.5), (69, 1.2)];

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every file under `dir`, by its path from `dir`, with its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_owned();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `doc` written back at `kind` `version`, its `$schema` to match.
fn restamped(mut doc: Value, kind: &str, version: Version) -> Value {
    doc["$schema"] = Value::String(schema_url(kind, version));
    doc["duet"]["kind"] = Value::String(kind.to_owned());
    doc["duet"]["version"] = Value::String(version.to_string());
    doc
}

/// The scratch directory, as duet's scratch work was left before projects.
fn scratch_work(dir: &Path) -> PathBuf {
    let work = dir.join("wayfaring");
    std::fs::create_dir_all(&work).unwrap();

    // A take captured beside its recording: today's capture states the take's digest.
    let take = work.join("wayfaring.wav");
    audio::write_wav(&take, &audio::notes(0.5, &NOTES));
    ok(
        &duet(&work, &["capture", "melody", s(&take), "--tempo", "92"]),
        "capture melody",
    );

    // A take whose reading is from before the take digest: capture-analysis 1.12 and
    // performance 1.0, neither stating one.
    let old = work.join("old-take.wav");
    audio::write_wav(&old, &audio::notes(0.4, &OTHER));
    ok(
        &duet(&work, &["capture", "melody", s(&old), "--tempo", "100"]),
        "capture melody",
    );
    let analysis = work.join("old-take.capture.duet");
    let mut doc = json(&analysis);
    doc["body"].as_object_mut().unwrap().remove("take");
    doc["body"]["reading"]
        .as_object_mut()
        .unwrap()
        .remove("passes");
    let doc = restamped(doc, "capture-analysis", Version::new(1, 12));
    std::fs::write(&analysis, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let performance = work.join("old-take.performance.duet.jsonl");
    let text = std::fs::read_to_string(&performance).unwrap();
    let (first, rest) = text.split_once('\n').unwrap();
    let mut head: Value = serde_json::from_str(first).unwrap();
    head["head"].as_object_mut().unwrap().remove("take");
    let head = restamped(head, "performance", Version::new(1, 0));
    std::fs::write(
        &performance,
        format!("{}\n{rest}", serde_json::to_string(&head).unwrap()),
    )
    .unwrap();

    // A part made by hand, and the notes view it was applied from.
    notes_doc(&work, "bass.notes", "[track \"Bass\" ch2]\n1.1.0 G2 3 80\n");

    // A part from before the `part` rename: its kind was `song`, and it states no source.
    let mut doc = json(&work.join("bass.duet"));
    let body = doc["body"].as_object_mut().unwrap();
    body.remove("id");
    body.remove("source");
    let doc = restamped(doc, "song", Version::new(1, 0));
    std::fs::write(
        work.join("old-melody.duet"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    // A comp, picks across takes for the melody.
    let comp = serde_json::json!({
        "$schema": schema_url("comp", Version::new(1, 0)),
        "duet": {
            "kind": "comp",
            "version": "1.0",
            "tool": { "name": "duet", "version": "0.5.0" }
        },
        "body": { "picks": [] }
    });
    std::fs::write(
        work.join("melody.comp.duet"),
        serde_json::to_string_pretty(&comp).unwrap(),
    )
    .unwrap();

    // Moments, a review sheet, and a file duet has no use for.
    std::fs::write(work.join("wayfaring.moments"), "12.5 the turn\n").unwrap();
    std::fs::create_dir_all(work.join("wayfaring.review/clips")).unwrap();
    std::fs::write(work.join("wayfaring.review/index.html"), "<p>sheet</p>\n").unwrap();
    std::fs::write(work.join("wayfaring.review/clips/1.wav"), b"RIFF").unwrap();
    std::fs::write(work.join("todo.txt"), "sing it slower\n").unwrap();
    work
}

/// A fresh project at `dir/porch`.
fn project(dir: &Path) -> PathBuf {
    ok(&duet(dir, &["init", "porch"]), "init");
    dir.join("porch")
}

fn read_project(root: &Path) -> Project {
    let text = std::fs::read_to_string(root.join(".duet/project.duet")).unwrap();
    Document::<Project>::read(&text).unwrap().body
}

/// Every line of the project's journal segments.
fn journal(root: &Path) -> String {
    let mut out = String::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("journal.") && name != "journal.duet.jsonl" {
            out.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    out
}

#[test]
fn project_adopt_without_apply_prints_the_plan_and_writes_nothing() {
    let dir = scratch("adopt-plan");
    let work = scratch_work(&dir);
    let root = project(&dir);
    let (work_before, root_before) = (tree(&work), tree(&root));

    let out = duet(
        &root,
        &["project", "adopt", s(&work), "--song", "wayfaring-stranger"],
    );
    ok(&out, "project adopt");
    let plan = stdout(&out);

    assert_eq!(tree(&root), root_before, "the project is as it was\n{plan}");
    assert_eq!(
        tree(&work),
        work_before,
        "the scratch directory is as it was"
    );
    assert!(!root.join("songs").exists(), "no song is made by a plan");

    for said in [
        // Audio becomes a take, named from its stem.
        "take wayfaring: wayfaring.wav, copied to takes/wayfaring/wayfaring.wav",
        "take old-take: old-take.wav, copied to takes/old-take/old-take.wav",
        // Derived files go to the take whose digest they state.
        "wayfaring.capture.duet (capture-analysis 1.13) to takes/wayfaring/wayfaring.capture.duet",
        "wayfaring.performance.duet.jsonl (performance 1.1) to takes/wayfaring/",
        "wayfaring.duet (part 1.0) to takes/wayfaring/wayfaring.duet",
        "wayfaring.abc to takes/wayfaring/wayfaring.abc",
        // A reading from before the take digest is matched by rate and length.
        "old-take.capture.duet (capture-analysis 1.12) to takes/old-take/old-take.capture.duet",
        "states no take digest: matched to old-take by its stem, rate and length",
        "not a variant",
        // Parts, a comp, and a part from before the rename go to the song.
        "bass.duet (part 1.0) to songs/wayfaring-stranger/parts/bass.duet",
        "bass.notes to songs/wayfaring-stranger/parts/bass.notes",
        "old-melody.duet (song 1.0) to songs/wayfaring-stranger/parts/old-melody.duet",
        "a part from before the part rename: it states no source",
        "melody.comp.duet (comp 1.0) to songs/wayfaring-stranger/comps/melody.comp.duet",
        // Moments, and a review sheet.
        "wayfaring.moments to review/wayfaring.moments",
        "wayfaring.review/ to takes/wayfaring/review/",
        // The song, made by --apply.
        "song wayfaring-stranger: made, songs/wayfaring-stranger/song.duet",
        // Anything else is left behind.
        "left behind: todo.txt",
        "nothing written",
    ] {
        assert!(plan.contains(said), "{said:?} in\n{plan}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn project_adopt_apply_copies_every_document_byte_for_byte_and_leaves_the_scratch_directory() {
    let dir = scratch("adopt-apply");
    let work = scratch_work(&dir);
    let root = project(&dir);
    let work_before = tree(&work);

    let out = duet(
        &root,
        &[
            "project",
            "adopt",
            s(&work),
            "--song",
            "wayfaring-stranger",
            "--apply",
        ],
    );
    ok(&out, "project adopt --apply");
    let said = stdout(&out);

    assert_eq!(
        tree(&work),
        work_before,
        "the scratch directory is untouched: nothing is moved\n{said}"
    );
    let song = "songs/wayfaring-stranger";
    for (from, to) in [
        ("wayfaring.wav", "takes/wayfaring/wayfaring.wav"),
        (
            "wayfaring.capture.duet",
            "takes/wayfaring/wayfaring.capture.duet",
        ),
        (
            "wayfaring.performance.duet.jsonl",
            "takes/wayfaring/wayfaring.performance.duet.jsonl",
        ),
        ("wayfaring.duet", "takes/wayfaring/wayfaring.duet"),
        ("wayfaring.abc", "takes/wayfaring/wayfaring.abc"),
        ("old-take.wav", "takes/old-take/old-take.wav"),
        (
            "old-take.capture.duet",
            "takes/old-take/old-take.capture.duet",
        ),
        (
            "old-take.performance.duet.jsonl",
            "takes/old-take/old-take.performance.duet.jsonl",
        ),
        ("bass.duet", &format!("{song}/parts/bass.duet")),
        ("bass.notes", &format!("{song}/parts/bass.notes")),
        ("old-melody.duet", &format!("{song}/parts/old-melody.duet")),
        (
            "melody.comp.duet",
            &format!("{song}/comps/melody.comp.duet"),
        ),
        ("wayfaring.moments", "review/wayfaring.moments"),
        (
            "wayfaring.review/index.html",
            "takes/wayfaring/review/index.html",
        ),
        (
            "wayfaring.review/clips/1.wav",
            "takes/wayfaring/review/clips/1.wav",
        ),
    ] {
        let copied = std::fs::read(root.join(to)).unwrap_or_else(|e| panic!("{to}: {e}\n{said}"));
        assert_eq!(
            copied,
            work_before[Path::new(from)],
            "{from} is copied to {to} byte for byte"
        );
    }
    assert!(!root.join("todo.txt").exists(), "what is left behind stays");
    assert!(
        root.join(song).join("song.duet").is_file(),
        "--song made it"
    );

    let doc = read_project(&root);
    let mut names: Vec<&str> = doc.takes.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["old-take", "wayfaring"]);
    let journal = journal(&root);
    assert_eq!(
        journal.matches("\"take.added\"").count(),
        2,
        "one act per take\n{journal}"
    );
    for take in &doc.takes {
        assert!(journal.contains(&take.take.digest.to_string()), "{journal}");
    }

    // Again: every destination exists now, so it is refused, and nothing more is written.
    let after = tree(&root);
    let again = duet(
        &root,
        &[
            "project",
            "adopt",
            s(&work),
            "--song",
            "wayfaring-stranger",
            "--apply",
        ],
    );
    assert!(!again.status.success(), "{}", stdout(&again));
    assert!(
        stderr(&again).contains("exists"),
        "the refusal names what exists: {}",
        stderr(&again)
    );
    assert_eq!(tree(&root), after, "a refused apply writes nothing");
    assert_eq!(tree(&work), work_before);
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn without_a_song_every_part_and_comp_needs_one_and_apply_leaves_them() {
    let dir = scratch("adopt-no-song");
    let work = scratch_work(&dir);
    let root = project(&dir);

    let plan = duet(&root, &["project", "adopt", s(&work)]);
    ok(&plan, "project adopt");
    for said in [
        "bass.duet (part 1.0) needs a song: --song names one",
        "melody.comp.duet (comp 1.0) needs a song",
        "take wayfaring: wayfaring.wav, copied to takes/wayfaring/wayfaring.wav",
    ] {
        assert!(
            stdout(&plan).contains(said),
            "{said:?} in\n{}",
            stdout(&plan)
        );
    }

    ok(
        &duet(&root, &["project", "adopt", s(&work), "--apply"]),
        "project adopt --apply",
    );
    assert!(
        root.join("takes/wayfaring/wayfaring.capture.duet")
            .is_file()
    );
    assert!(
        !root.join("songs").exists(),
        "no song, so no part is copied"
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_named_take_is_adopted_where_it_is() {
    let dir = scratch("adopt-named");
    let work = scratch_work(&dir);
    let root = project(&dir);

    ok(
        &duet(&root, &["project", "adopt", s(&work), "--named", "--apply"]),
        "project adopt --named --apply",
    );
    assert!(
        !root.join("takes/wayfaring/wayfaring.wav").exists(),
        "not copied"
    );
    assert!(
        root.join("takes/wayfaring/wayfaring.capture.duet")
            .is_file()
    );
    let list = duet(&root, &["take", "list"]);
    ok(&list, "take list");
    assert!(
        stdout(&list).contains(&format!(
            "found at {}, named in .duet/takes.local.toml",
            work.join("wayfaring.wav").display()
        )),
        "{}",
        stdout(&list)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn a_reading_without_a_digest_that_does_not_fit_its_take_is_left_behind() {
    let dir = scratch("adopt-misfit");
    let work = scratch_work(&dir);
    let root = project(&dir);
    // A recording far shorter than the performance its stem names: never guessed at.
    audio::write_wav(&work.join("short.wav"), &audio::silence(0.5));
    std::fs::copy(
        work.join("old-take.performance.duet.jsonl"),
        work.join("short.performance.duet.jsonl"),
    )
    .unwrap();

    let plan = duet(&root, &["project", "adopt", s(&work)]);
    ok(&plan, "project adopt");
    let said = "left behind: short.performance.duet.jsonl (performance 1.0): states no take \
                digest, and its frames run to";
    assert!(
        stdout(&plan).contains(said),
        "{said:?} in\n{}",
        stdout(&plan)
    );
    std::fs::remove_dir_all(&dir).expect("clean up");
}
