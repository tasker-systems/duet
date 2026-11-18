//! Reading and writing parts and drum grids. A `.duet` file is read by its envelope, so the kind
//! decides what it is, never the name. `.mid` (`.midi`, `.smf`) and `.abc` are the outside
//! formats, read by their names. notefmt text and a drum grid's text are views: `duet view`
//! writes them and `duet apply` reads them, and no other command reads or writes them, by any
//! name or on standard input or output.
//!
//! Standard input is read by what it holds: a document when its first non-blank byte is `{`,
//! MIDI when it starts with `MThd`, and ABC only when `--from abc` says so. Anything else is
//! refused, pointing at `duet apply`. Standard output is a document unless `--to` says
//! otherwise, so one command's output pipes into the next: `duet apply take.notes -o - | duet
//! play -`.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use clap::ValueEnum;
use clap::builder::{EnumValueParser, PossibleValue, TypedValueParser};
use clap::error::ErrorKind;
use duet_core::{EndRule, Part};
use duet_drums::DrumGrid;
use duet_midi::Midi;
use duet_schemas::journal::{DocDigest, Id};
use duet_schemas::{AnyDocument, Document, From, PartBody, Source, Tool};
use schemars::JsonSchema;

use crate::Context;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// A duet document (.duet), read by its envelope.
    Duet,
    /// A Standard MIDI File (.mid, .midi or .smf).
    Midi,
    /// ABC: the first tune of the file when reading (`duet abc render --tune` picks
    /// another), one tune `X:1` when writing.
    Abc,
}

/// The parser of `--from` and `--to`: a [`Format`] by its name. The text formats these flags
/// once named (`text`, `drums`) are refused with where text went: `duet view` and
/// `duet apply`.
#[derive(Debug, Clone)]
pub struct FormatParser;

impl TypedValueParser for FormatParser {
    type Value = Format;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> std::result::Result<Format, clap::Error> {
        let given = value.to_string_lossy();
        if matches!(
            given.to_ascii_lowercase().as_str(),
            "text" | "drums" | "notes" | "notefmt" | "grid"
        ) {
            let flag = arg.map_or_else(
                || "--from or --to".to_owned(),
                |a| format!("--{}", a.get_long().unwrap_or(a.get_id().as_str())),
            );
            return Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                format!(
                    "{flag} {given} was removed: a part's or a drum grid's text is a view of a \
                     document. Write one with `duet view <doc.duet> --as notes|grid`, and read \
                     one with `duet apply <view> -o <new.duet>` (or --onto the document it came \
                     from)\n"
                ),
            )
            .with_cmd(cmd));
        }
        EnumValueParser::<Format>::new().parse_ref(cmd, arg, value)
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            Format::value_variants()
                .iter()
                .filter_map(ValueEnum::to_possible_value),
        ))
    }
}

/// What a file's name says it is, when it is not a part's format.
enum Named {
    Part(Format),
    /// A `.duet.jsonl` stream.
    Stream,
    /// A name duet no longer reads: `.json`, `.toml`, and the `.drums.json` and `.drums.toml`
    /// of drum grids.
    Cut,
    /// A view's name: `.notes`, `.txt` or `.drums`.
    View,
    Unknown,
}

fn named(path: &Path) -> Named {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if name.ends_with(".duet.jsonl") {
        return Named::Stream;
    }
    match name.rsplit_once('.').map(|(_, ext)| ext) {
        Some("duet") => Named::Part(Format::Duet),
        Some("mid" | "midi" | "smf") => Named::Part(Format::Midi),
        Some("abc") => Named::Part(Format::Abc),
        Some("json" | "toml") => Named::Cut,
        Some("notes" | "txt" | "drums") => Named::View,
        _ => Named::Unknown,
    }
}

/// Where a view's text goes instead, for a refusal.
fn to_apply(path: &Path) -> String {
    format!(
        "A view's text is read only by `duet apply {} -o <new.duet>` (or --onto the document \
         it came from), and written only by `duet view`",
        path.display()
    )
}

/// Why a name is refused as a part's, and what to do instead.
fn refusal(path: &Path, named: &Named, what: &str) -> String {
    let shown = path.display();
    match named {
        Named::Stream => format!("{shown} is a stream (.duet.jsonl), and {what} a part"),
        Named::Cut => format!(
            "{shown} is not a duet document: duet's documents are .duet, and .json and .toml \
             are no longer read or written"
        ),
        Named::View => format!(
            "{shown} is not a duet document or an outside format: duet reads .duet, .mid and \
             .abc by name. {}",
            to_apply(path)
        ),
        Named::Part(_) | Named::Unknown => format!(
            "{shown}: duet reads and writes .duet, .mid (.midi, .smf) and .abc by name; name \
             any other with --from or --to (duet, midi or abc). {}",
            to_apply(path)
        ),
    }
}

/// The format of a path a command reads or writes as a part: `given` when the user named one,
/// a document for `-` (standard input is read by what it holds, as [`read`] says), or else the
/// format the path's name gives. Any other name is refused, so a file is never misread:
/// `.json` and `.toml` are no longer read, and a view's text is read only by `duet apply`.
pub fn part_format(path: &Path, given: Option<Format>, what: &str) -> Result<Format> {
    if let Some(format) = given {
        return Ok(format);
    }
    if path == Path::new("-") {
        return Ok(Format::Duet);
    }
    match named(path) {
        Named::Part(format) => Ok(format),
        other => bail!("{}", refusal(path, &other, what)),
    }
}

/// Checks a path a command reads or writes as a drum grid: a `.duet` drum-grid document, or
/// `-` for one on standard input or output. Anything else is refused by name: a part's
/// outside format, a stream, a name duet no longer reads, or a grid's text, which is a view.
pub fn grid_path(path: &Path, what: &str) -> Result<()> {
    if path == Path::new("-") {
        return Ok(());
    }
    let shown = path.display();
    match named(path) {
        Named::Part(Format::Duet) => Ok(()),
        Named::Part(format) => bail!(
            "{shown} is a part's name ({}), and {what} a drum grid, a drum-grid document \
             (.duet)",
            format
                .to_possible_value()
                .map_or_else(String::new, |v| v.get_name().to_owned())
        ),
        Named::View | Named::Unknown => bail!(
            "{shown}: {what} a drum grid as a drum-grid document (.duet). {}",
            to_apply(path)
        ),
        other => bail!(
            "{}",
            refusal(path, &other, what).replace(" a part", " a drum grid")
        ),
    }
}

/// Reads a drum-grid document from a path ([`grid_path`]), refused naming its kind when it is
/// not a drum grid. Its grid is normalized, as a hand edit may leave it loose.
pub fn read_grid_document(path: &Path, what: &str) -> Result<DrumGrid> {
    read_grid_with_digest(path, what).map(|(grid, _)| grid)
}

/// Reads a drum-grid document as [`read_grid_document`] does, with its body's digest as read.
pub fn read_grid_with_digest(path: &Path, what: &str) -> Result<(DrumGrid, DocDigest)> {
    grid_path(path, what)?;
    let bytes = bytes_of(path)?;
    if path == Path::new("-") && !is_document(&bytes) {
        bail!("{}", not_a_document(what));
    }
    let text = String::from_utf8(bytes).context("a document is UTF-8 text")?;
    match AnyDocument::read(&text).with_context(|| path.display().to_string())? {
        AnyDocument::DrumGrid(doc) => {
            let digest = body_digest(&doc.body)?;
            let mut grid = doc.body;
            grid.normalize()?;
            Ok((grid, digest))
        }
        other => bail!(
            "{} is a {} document, and {what} a drum grid",
            path.display(),
            other.kind()
        ),
    }
}

/// The refusal of standard input that is not a document, nor MIDI where a part is read.
fn not_a_document(what: &str) -> String {
    format!(
        "standard input is not a duet document (one starts with {{), and {what} one; if it is \
         a view's text, read it with `duet apply - -o -` and pipe that in, and name an ABC tune \
         with --from abc"
    )
}

/// A drum grid as a document, with what every document duet writes names as its tool.
pub fn grid_document(grid: &DrumGrid) -> Result<String> {
    Ok(Document::new(tool(), grid.clone()).write()?)
}

fn bytes_of(path: &Path) -> Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .context("reading standard input")?;
        Ok(buf)
    } else {
        fs::read(path).with_context(|| path.display().to_string())
    }
}

/// Whether standard input holds a document: its first non-blank byte is `{`. No MIDI file,
/// ABC tune or notefmt text starts with one.
fn is_document(bytes: &[u8]) -> bool {
    bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

/// What `convert` read: a part with where it came from, or a drum grid from a drum-grid
/// document, with that document's digest.
#[derive(Debug)]
pub enum Loaded {
    Part(Part, Origin),
    Grid(DrumGrid, DocDigest),
}

/// What a command knows of where a part came from, beside its notes: the envelope's `from`, and
/// the part document it was read from, if it was read from one.
#[derive(Debug, Clone, Default)]
pub struct Origin {
    pub from: Option<From>,
    pub document: Option<ReadDocument>,
}

/// A part document as it was read: where, its id and source, and its body's digest.
#[derive(Debug, Clone)]
pub struct ReadDocument {
    pub path: std::path::PathBuf,
    pub id: Id,
    pub source: Source,
    pub digest: DocDigest,
}

impl Origin {
    /// An import's: where it came from, and no document.
    pub fn imported(from: From) -> Self {
        Self {
            from: Some(from),
            document: None,
        }
    }

    /// **The rule for `source`: it follows the part.** A part written from a part document
    /// keeps that document's source; one read from MIDI or ABC was written by hand, in another
    /// program (the envelope's `from` says which).
    pub fn source(&self) -> Source {
        self.document
            .as_ref()
            .map_or_else(Source::hand, |doc| doc.source.clone())
    }

    /// The source of a part an operation made of this one: `generated`, naming the operation
    /// and the document it read, when it read one.
    pub fn generated(&self, operation: &str) -> Source {
        generated(operation, self.document.iter().map(|doc| doc.digest))
    }
}

/// A `generated` source: the operation, and the digest of each document it read.
pub fn generated(operation: &str, inputs: impl IntoIterator<Item = DocDigest>) -> Source {
    serde_json::from_value(serde_json::json!({
        "by": "generated",
        "operation": operation,
        "inputs": inputs.into_iter().collect::<Vec<_>>(),
    }))
    .expect("a generated source is a source")
}

/// The digest a document's `doc.written` names: the FNV-1a of its body as compact JSON.
pub fn body_digest<T: serde::Serialize>(body: &T) -> Result<DocDigest> {
    Ok(DocDigest::of(&serde_json::to_vec(body)?))
}

pub use crate::context::fresh_id;

/// Reads a part. `what` says what the command does with it, as in "duet play reads", for a
/// document of another kind. Standard input is read by what it holds: a document when its
/// first non-blank byte is `{`, ABC when `format` (`--from abc`) says so, MIDI when it starts
/// with `MThd`, and anything else is refused, pointing at `duet apply` for a view's text.
pub fn read(path: &Path, format: Format, what: &str) -> Result<Part> {
    read_with_from(path, format, what).map(|(part, _)| part)
}

/// Reads a part and where it came from, the provenance a `.duet` written from it records:
///
/// - **a `.duet`**: its own `from`, kept as it is, or none;
/// - **MIDI**: `{"format": "midi"}`, since MIDI drops nothing duet's model holds that a
///   notice would name;
/// - **ABC**: `{"format": "abc"}`, the tune's `X:` number, and every notation-only construct
///   the reader read but did not carry, each at its line and column.
///
/// A view's text is not read here: `duet apply` reads it, and a document it writes has no
/// `from`, since a view is not where a part comes from.
pub fn read_with_from(path: &Path, format: Format, what: &str) -> Result<(Part, Origin)> {
    match read_any(path, format, what)? {
        Loaded::Part(part, origin) => Ok((part, origin)),
        Loaded::Grid(..) => bail!(
            "{} is a drum-grid document, and {what} a part; duet drums render makes a part of \
             one",
            path.display()
        ),
    }
}

/// Reads a part, or a drum grid when the path holds a drum-grid document.
pub fn read_any(path: &Path, format: Format, what: &str) -> Result<Loaded> {
    let bytes = bytes_of(path)?;
    let format = if path != Path::new("-") {
        format
    } else if is_document(&bytes) {
        Format::Duet
    } else if format == Format::Abc {
        Format::Abc
    } else if bytes.starts_with(b"MThd") {
        Format::Midi
    } else {
        bail!("{}", not_a_document(what));
    };
    let text = || String::from_utf8(bytes.clone()).context("input is not UTF-8 text");
    // MIDI and ABC come back normalized. A hand-edited document may hold lengths or
    // positions in a loose form, so it is normalized here.
    let (part, origin) = match format {
        Format::Duet => {
            let doc = AnyDocument::read(&text()?).with_context(|| path.display().to_string())?;
            match doc {
                AnyDocument::Part(doc) => {
                    let document = ReadDocument {
                        path: path.to_path_buf(),
                        id: doc.body.id.clone(),
                        source: doc.body.source.clone(),
                        digest: body_digest(&doc.body)?,
                    };
                    let origin = Origin {
                        from: doc.from,
                        document: Some(document),
                    };
                    (normalized(doc.body.part)?, origin)
                }
                AnyDocument::DrumGrid(doc) => {
                    let digest = body_digest(&doc.body)?;
                    let mut grid = doc.body;
                    grid.normalize()?;
                    return Ok(Loaded::Grid(grid, digest));
                }
                other => bail!(
                    "{} is a {} document, and {what} a part",
                    path.display(),
                    other.kind()
                ),
            }
        }
        Format::Midi => (
            Part::from_midi(&bytes)?,
            Origin::imported(From::new("midi")),
        ),
        Format::Abc => {
            let book = duet_abc::Book::parse(&text()?)?;
            if book.tunes().len() > 1 {
                bail!(
                    "{} holds {} tunes; pick one with duet abc render --tune <X>",
                    path.display(),
                    book.tunes().len()
                );
            }
            let (part, from) = crate::abc::reading(&book.tunes()[0])?;
            (part, Origin::imported(from))
        }
    };
    Ok(Loaded::Part(part, origin))
}

fn normalized(mut part: Part) -> duet_core::Result<Part> {
    part.normalize()?;
    Ok(part)
}

/// The part with its end stated: the one it names, or else the configured default's.
pub fn with_end(part: &Part, end: EndRule) -> Result<Part> {
    let mut part = part.clone();
    if part.end.is_none() {
        part.end = Some(part.default_end_by(end)?);
    }
    Ok(part)
}

/// What every document duet writes names as its tool.
pub fn tool() -> Tool {
    Tool::new("duet", env!("CARGO_PKG_VERSION"))
}

/// Writes a part made from no document (a drum grid, a view) in a format, with `source`.
pub fn write(
    ctx: &Context,
    part: &Part,
    path: &Path,
    format: Format,
    source: Source,
) -> Result<()> {
    write_from(ctx, part, path, format, &Origin::default(), source)
}

/// Writes the part in a format. Every file written says where the part ends, whatever
/// default computed it: MIDI states it itself, and a document carries it as its
/// body's `end`. A document also records `from`, where the part came from
/// ([`read_with_from`]), the part's `id` and its `source`; the other formats have nowhere to
/// keep them.
///
/// **The rule for `from`:** it follows the part. A command that reads a part and writes a
/// document of it (`convert`, `part quantize`, `part show --to duet`, `abc render`) records
/// what it read: a `.duet`'s own `from`, unchanged, or a new one for an import from ABC or
/// MIDI. Going out to ABC or MIDI and in again replaces it, since that import is the part's new
/// provenance. Quantizing keeps it: the notices still name what the import did not carry.
/// A view carries none, so a part `duet apply` reads from one has none. A drum grid carries
/// none yet.
///
/// **The rule for `id`: it follows the file** (ruling 13: minted once when the document is
/// made, never changed). A document written over the one it was read from keeps its id; every
/// other document written is a new one, a copy at most, and has a new id.
///
/// **The source is the caller's**, by [`Origin::source`] or [`Origin::generated`].
pub fn write_from(
    ctx: &Context,
    part: &Part,
    path: &Path,
    format: Format,
    origin: &Origin,
    source: Source,
) -> Result<()> {
    let part = with_end(part, ctx.config().end)?;
    let bytes = match format {
        Format::Duet => {
            let id = match &origin.document {
                Some(doc) if path != Path::new("-") && same_file(&doc.path, path) => doc.id.clone(),
                _ => fresh_id()?,
            };
            let mut doc = Document::new(tool(), PartBody::new(id, source, part));
            doc.from.clone_from(&origin.from);
            doc.write()?.into_bytes()
        }
        Format::Midi => part.to_midi()?,
        Format::Abc => duet_abc::write(&part, &duet_abc::WriteOptions::default())?.into_bytes(),
    };
    write_bytes(ctx, path, &bytes)
}

/// A part's document with a new id, by hand: what a test writes as a part `duet` reads.
///
/// # Panics
///
/// When the operating system gives no randomness for the id.
#[must_use]
pub fn part_document(part: Part) -> String {
    Document::new(
        tool(),
        PartBody::new(fresh_id().expect("an id"), Source::hand(), part),
    )
    .write()
    .expect("written")
}

/// Writes to a file, or to standard output for `-`, through the action's context, which
/// records a document it writes ([`Context::write`]).
pub fn write_bytes(ctx: &Context, path: &Path, bytes: &[u8]) -> Result<()> {
    ctx.write(path, bytes)
}

/// Writes text to standard output.
pub fn stdout(text: &str) -> Result<()> {
    io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| anyhow::anyhow!("writing standard output: {e}"))
}

/// Whether two paths name the same existing file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use duet_core::{Note, Part, Position, Track};

    use super::{Format, grid_path, part_format, read};

    #[test]
    fn a_part_is_named_by_dot_duet_mid_or_abc_and_nothing_else() {
        let of = |name: &str| part_format(Path::new(name), None, "reads");
        assert_eq!(of("part.duet").unwrap(), Format::Duet);
        assert_eq!(of("Part.DUET").unwrap(), Format::Duet);
        assert_eq!(of("take.mid").unwrap(), Format::Midi);
        assert_eq!(of("take.smf").unwrap(), Format::Midi);
        assert_eq!(of("tune.abc").unwrap(), Format::Abc);
        assert_eq!(
            of("-").unwrap(),
            Format::Duet,
            "standard streams are documents"
        );
        for (name, says) in [
            ("part.json", "no longer read"),
            ("part.toml", "no longer read"),
            ("beat.drums.json", "no longer read"),
            ("take.notes", "`duet apply take.notes -o <new.duet>`"),
            ("take.txt", "`duet apply take.txt -o <new.duet>`"),
            ("beat.drums", "`duet apply beat.drums -o <new.duet>`"),
            ("perf.duet.jsonl", "is a stream"),
            ("part", "name any other with --from or --to"),
        ] {
            let err = of(name).expect_err(name).to_string();
            assert!(err.contains(says), "{name}: {err}");
        }
        let given = part_format(Path::new("x.bin"), Some(Format::Midi), "reads");
        assert_eq!(
            given.unwrap(),
            Format::Midi,
            "--from says what a name does not"
        );
    }

    #[test]
    fn a_grid_is_a_document_by_name_and_a_view_is_refused() {
        assert!(grid_path(Path::new("beat.duet"), "reads").is_ok());
        assert!(grid_path(Path::new("-"), "reads").is_ok());
        for (name, says) in [
            ("beat.drums", "`duet apply beat.drums -o <new.duet>`"),
            ("beat", "`duet apply beat -o <new.duet>`"),
            ("beat.mid", "is a part's name (midi)"),
            ("beat.drums.json", "no longer read"),
        ] {
            let err = grid_path(Path::new(name), "reads")
                .expect_err(name)
                .to_string();
            assert!(err.contains(says), "{name}: {err}");
        }
    }

    #[test]
    fn read_normalizes_a_document() {
        // A loose position and a length that is not whole ticks, as a hand edit might leave.
        let mut part = Part::default();
        let mut track = Track::new("P", 1);
        track.notes.push(Note::new(
            Position::new(1, 1, 960),
            "C4".parse().expect("C4 is a pitch"),
            0.3334,
            80,
        ));
        part.tracks.push(track);

        let dir = std::env::temp_dir().join(format!("duet-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("part.duet");
        let text = super::part_document(part);
        std::fs::write(&path, text).expect("write");
        let read = read(&path, Format::Duet, "reads").expect("read");
        let note = &read.tracks[0].notes[0];
        assert_eq!((note.at.to_string(), note.beats), ("1.3.0".into(), 0.333));
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
