//! The action registry: every action duet has, and what each declares (the bus design's §1,
//! "The action registry").

use schemars::JsonSchema;
use serde_json::Value;

use crate::{abc, adopt, capture, convert, drums, part, play, project, record, song, take, view};

/// Where an action runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Runs {
    /// In the process that invokes it, joined to a server or not (decision 10). The default.
    Anywhere,
    /// In the server, for an action that needs what only the server owns. None does yet.
    Server,
}

/// What an action may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Writes {
    /// The files it may create, in words.
    pub what: &'static str,
    /// Whether it may write over a file that is there, which its `doc.written` then records
    /// as `replaced: true`. An action that never does refuses an output that exists.
    pub replaces: bool,
}

/// One registered action.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Action {
    /// Dotted and stable: `capture.melody`, `part.quantize`.
    pub name: &'static str,
    /// Its path in the CLI: `["capture", "melody"]`.
    pub cli: &'static [&'static str],
    /// The event kinds it may cause.
    pub emits: &'static [&'static str],
    pub runs: Runs,
    /// What it writes, if it writes files at all.
    pub writes: Option<Writes>,
    /// Its input's JSON schema, from the type its CLI arguments come from.
    pub input: fn() -> Value,
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("a schema is JSON")
}

const DOC_WRITTEN: &[&str] = &["doc.written"];

/// A take added to the project: its document's `doc.written`, and a `take.added` naming it.
const TAKE_ADDED: &[&str] = &["doc.written", "take.added"];

const fn writes(what: &'static str, replaces: bool) -> Option<Writes> {
    Some(Writes { what, replaces })
}

const fn reads(name: &'static str, cli: &'static [&'static str], input: fn() -> Value) -> Action {
    Action {
        name,
        cli,
        emits: &[],
        runs: Runs::Anywhere,
        writes: None,
        input,
    }
}

const fn writing(
    name: &'static str,
    cli: &'static [&'static str],
    emits: &'static [&'static str],
    writes: Option<Writes>,
    input: fn() -> Value,
) -> Action {
    Action {
        name,
        cli,
        emits,
        runs: Runs::Anywhere,
        writes,
        input,
    }
}

/// Every action, in the CLI's order.
pub const ACTIONS: &[Action] = &[
    // `init` journals nothing: it makes the project, and the project's account begins with the
    // first action run in it (the project design's §3 lists all `init` writes).
    writing(
        "init",
        &["init"],
        &[],
        writes(
            ".duet/project.duet, new, and .duet/duet.toml when absent",
            false,
        ),
        schema::<project::InitInput>,
    ),
    reads(
        "project.path",
        &["project", "path"],
        schema::<project::PathInput>,
    ),
    reads(
        "project.show",
        &["project", "show"],
        schema::<project::ShowInput>,
    ),
    // A plan writes nothing. --apply copies each file new, makes the song --song names when it
    // is missing, and rewrites the project's document; each take added is a `take.added`.
    writing(
        "project.adopt",
        &["project", "adopt"],
        TAKE_ADDED,
        writes(
            "with --apply: takes/<take>/, songs/<song>/parts/ and comps/, and review/, each file \
             new; songs/<song>/song.duet when missing; .duet/project.duet, over itself, and with \
             --named .duet/takes.local.toml",
            true,
        ),
        schema::<adopt::AdoptInput>,
    ),
    writing(
        "take.add",
        &["take", "add"],
        TAKE_ADDED,
        writes(
            "takes/<take>/<file>, new, or with --named .duet/takes.local.toml; and \
             .duet/project.duet, over itself",
            true,
        ),
        schema::<take::AddInput>,
    ),
    writing(
        "take.locate",
        &["take", "locate"],
        &[],
        writes(".duet/takes.local.toml", true),
        schema::<take::LocateInput>,
    ),
    reads("take.list", &["take", "list"], schema::<take::ListInput>),
    writing(
        "take.forget",
        &["take", "forget"],
        DOC_WRITTEN,
        writes(
            ".duet/project.duet, over itself, and .duet/takes.local.toml; never a take's files",
            true,
        ),
        schema::<take::ForgetInput>,
    ),
    writing(
        "convert",
        &["convert"],
        DOC_WRITTEN,
        writes("a part or drum-grid document, MIDI or ABC", true),
        schema::<convert::ConvertInput>,
    ),
    writing(
        "view",
        &["view"],
        &[],
        writes("a view's text, with -o", true),
        schema::<view::ViewArgs>,
    ),
    writing(
        "apply",
        &["apply"],
        DOC_WRITTEN,
        writes("the document --onto names, or a new one with -o", true),
        schema::<view::ApplyArgs>,
    ),
    reads("part.show", &["part", "show"], schema::<part::ShowInput>),
    writing(
        "part.quantize",
        &["part", "quantize"],
        DOC_WRITTEN,
        writes("a quantized copy of a part", true),
        schema::<part::QuantizeInput>,
    ),
    writing(
        "song.new",
        &["song", "new"],
        DOC_WRITTEN,
        writes("songs/<song>/song.duet, and its parts/", false),
        schema::<song::NewInput>,
    ),
    reads("song.show", &["song", "show"], schema::<song::ShowInput>),
    writing(
        "song.status",
        &["song", "status"],
        &["doc.written", "song.status-set"],
        writes("the song's song.duet, over itself", true),
        schema::<song::StatusInput>,
    ),
    writing(
        "song.link",
        &["song", "link"],
        DOC_WRITTEN,
        writes("the song's song.duet, over itself", true),
        schema::<song::LinkInput>,
    ),
    writing(
        "song.place",
        &["song", "place"],
        DOC_WRITTEN,
        writes("the song's song.duet, over itself", true),
        schema::<song::PlaceInput>,
    ),
    writing(
        "song.snapshot",
        &["song", "snapshot"],
        &["doc.written", "snapshot.taken"],
        writes(
            "songs/<song>/snapshots/<date>-<name>/: copies of song.duet and its placed parts, and \
             snapshot.duet",
            false,
        ),
        schema::<song::SnapshotInput>,
    ),
    writing(
        "song.restore",
        &["song", "restore"],
        &["doc.written", "snapshot.taken"],
        writes(
            "a snapshot of the working song, then the snapshot's song.duet and parts over the \
             working ones",
            true,
        ),
        schema::<song::RestoreInput>,
    ),
    writing(
        "drums.render",
        &["drums", "render"],
        DOC_WRITTEN,
        writes("a part rendered from a grid: a document, MIDI or ABC", true),
        schema::<drums::RenderInput>,
    ),
    writing(
        "drums.read",
        &["drums", "read"],
        DOC_WRITTEN,
        writes("a drum-grid document", true),
        schema::<drums::ReadInput>,
    ),
    writing(
        "drums.snap",
        &["drums", "snap"],
        DOC_WRITTEN,
        writes("a drum-grid document, its velocities snapped", true),
        schema::<drums::SnapInput>,
    ),
    reads(
        "drums.lanes",
        &["drums", "lanes"],
        schema::<drums::LanesInput>,
    ),
    reads("abc.check", &["abc", "check"], schema::<abc::CheckInput>),
    writing(
        "abc.render",
        &["abc", "render"],
        DOC_WRITTEN,
        writes("a part rendered from a tune: a document or MIDI", true),
        schema::<abc::RenderInput>,
    ),
    writing(
        "abc.write",
        &["abc", "write"],
        &[],
        writes("an ABC file", true),
        schema::<abc::WriteInput>,
    ),
    writing(
        "capture.melody",
        &["capture", "melody"],
        DOC_WRITTEN,
        writes(
            "STEM.duet, STEM.abc, STEM.capture.duet and STEM.performance.duet.jsonl, all new; in \
             a project, the song.duet of each song that links the take and is placed by it, over \
             itself",
            true,
        ),
        schema::<capture::TakeArgs>,
    ),
    writing(
        "capture.rhythm",
        &["capture", "rhythm"],
        DOC_WRITTEN,
        writes(
            "STEM.duet, STEM.grid.duet and STEM.capture.duet, all new; in a project, the \
             song.duet of each song that links the take and is placed by it, over itself",
            true,
        ),
        schema::<capture::TakeArgs>,
    ),
    writing(
        "capture.compare",
        &["capture", "compare"],
        &[],
        writes("STEM.compare.svg and STEM.compare.wav, both new", false),
        schema::<capture::compare::CompareArgs>,
    ),
    writing(
        "capture.review",
        &["capture", "review"],
        &[],
        writes(
            "STEM.review/, a new directory of a page and its clips",
            false,
        ),
        schema::<capture::review::ReviewArgs>,
    ),
    writing(
        "capture.disagree",
        &["capture", "disagree"],
        &[],
        writes("STEM.disagree.moments, new, when a place is found", false),
        schema::<capture::disagree::DisagreeArgs>,
    ),
    reads("play", &["play"], schema::<play::PlayArgs>),
    // In a project, `duet record <take>` records into takes/<take>/<take>.flac and adds the
    // take, which rewrites the project's document.
    writing(
        "record",
        &["record"],
        TAKE_ADDED,
        writes(
            "a new FLAC take; in a project, under takes/<take>/, and .duet/project.duet over \
             itself",
            true,
        ),
        schema::<record::RecordArgs>,
    ),
];

/// The action named `name`.
#[must_use]
pub fn action(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.name == name)
}

#[cfg(test)]
mod tests {
    use super::ACTIONS;

    #[test]
    fn every_action_has_a_unique_name_and_path_and_an_input_schema() {
        for (i, action) in ACTIONS.iter().enumerate() {
            assert!(
                ACTIONS[..i]
                    .iter()
                    .all(|a| a.name != action.name && a.cli != action.cli),
                "{} is registered twice",
                action.name
            );
            assert_eq!(action.name, action.cli.join("."), "named by its path");
            let schema = (action.input)();
            assert_eq!(schema["type"], "object", "{}: {schema}", action.name);
            assert_eq!(
                action.emits.contains(&"doc.written"),
                action
                    .writes
                    .is_some_and(|_| action.emits.contains(&"doc.written")),
                "{}",
                action.name
            );
        }
    }
}
