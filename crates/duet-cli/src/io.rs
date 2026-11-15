//! Reading and writing songs and drum grids. A `.duet` file is read by its envelope, so the kind
//! decides what it is, never the name. `.mid` (`.midi`, `.smf`) and `.abc` are the outside
//! formats, read by their names. notefmt text and a drum grid's text are views, so no file name
//! means them: they are read and written only when `--from` or `--to` says so, on standard input
//! and output, or, for a grid's text, on the grid side of a `duet drums` command.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use clap::builder::PossibleValue;
use duet_core::{EndRule, Song};
use duet_drums::DrumGrid;
use duet_midi::Midi;
use duet_schemas::{AnyDocument, Document, Tool};

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Format {
    /// A duet document (.duet), read by its envelope.
    Duet,
    /// A Standard MIDI File (.mid, .midi or .smf).
    Midi,
    /// ABC: the first tune of the file when reading (`duet abc render --tune` picks
    /// another), one tune `X:1` when writing.
    Abc,
    /// notefmt text, the song's text view. No file name means it.
    Text,
}

/// What a file's name says it is, when it is not a song's format.
enum Named {
    Song(Format),
    /// A `.duet.jsonl` stream.
    Stream,
    /// A name duet no longer reads: `.json`, `.toml`, and the `.drums.json` and `.drums.toml`
    /// of drum grids.
    Cut,
    /// A view's old name: `.notes`, `.txt` or `.drums`.
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
        Some("duet") => Named::Song(Format::Duet),
        Some("mid" | "midi" | "smf") => Named::Song(Format::Midi),
        Some("abc") => Named::Song(Format::Abc),
        Some("json" | "toml") => Named::Cut,
        Some("notes" | "txt" | "drums") => Named::View,
        _ => Named::Unknown,
    }
}

/// Why a name is refused as a song's, and what to do instead.
fn refusal(path: &Path, named: &Named, what: &str) -> String {
    let path = path.display();
    match named {
        Named::Stream => format!("{path} is a stream (.duet.jsonl), and {what} a song"),
        Named::Cut => format!(
            "{path} is not a duet document: duet's documents are .duet, and .json and .toml \
             are no longer read or written"
        ),
        Named::View => format!(
            "{path} is not a duet document or an outside format: duet reads .duet, .mid and \
             .abc by name. notefmt text is a view, read only when --from text says so"
        ),
        Named::Song(_) | Named::Unknown => format!(
            "{path}: duet reads and writes .duet, .mid (.midi, .smf) and .abc by name; name \
             any other with --from or --to"
        ),
    }
}

/// The format of a path a command reads or writes as a song: `given` when the user named one,
/// `default` for `-` (standard input or output), or else the format the path's name gives.
/// Any other name is refused, so a file is never misread: `.json` and `.toml` are no longer
/// read, and a view's text is read only when `--from text` says so.
pub fn song_format(
    path: &Path,
    given: Option<Format>,
    default: Format,
    what: &str,
) -> Result<Format> {
    if let Some(format) = given {
        return Ok(format);
    }
    if path == Path::new("-") {
        return Ok(default);
    }
    match named(path) {
        Named::Song(format) => Ok(format),
        other => bail!("{}", refusal(path, &other, what)),
    }
}

/// Every format `duet convert` goes between: a song's, or a drum grid's text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnyFormat {
    Song(Format),
    /// A drum grid's text, its `grid` view. No file name means it.
    Grid,
}

/// The song formats by their own names, then `drums`, a drum grid's text.
impl ValueEnum for AnyFormat {
    fn value_variants<'a>() -> &'a [Self] {
        static ALL: std::sync::LazyLock<Vec<AnyFormat>> = std::sync::LazyLock::new(|| {
            Format::value_variants()
                .iter()
                .copied()
                .map(AnyFormat::Song)
                .chain([AnyFormat::Grid])
                .collect()
        });
        &ALL
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            AnyFormat::Song(format) => format.to_possible_value(),
            AnyFormat::Grid => Some(PossibleValue::new("drums").help("A drum grid's text")),
        }
    }
}

/// How a drum grid is held on the grid side of a command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridForm {
    /// A drum grid's text, its `grid` view.
    Text,
    /// A drum-grid document (`.duet`), read by its envelope.
    Document,
}

/// The form of a path a command reads or writes as a drum grid: a `.duet` is a drum-grid
/// document, and any other name is a grid's text, as is `-` (standard input is read as a
/// document all the same when it starts with `{`). A name that is something else is refused
/// rather than misread: a song's outside format, a stream, or a name duet no longer reads.
pub fn grid_form(path: &Path, what: &str) -> Result<GridForm> {
    if path == Path::new("-") {
        return Ok(GridForm::Text);
    }
    let shown = path.display();
    match named(path) {
        Named::Song(Format::Duet) => Ok(GridForm::Document),
        Named::View | Named::Unknown => Ok(GridForm::Text),
        Named::Song(format) => bail!(
            "{shown} is a song's name ({}), and {what} a drum grid",
            format
                .to_possible_value()
                .map_or_else(String::new, |v| v.get_name().to_owned())
        ),
        other => bail!(
            "{}",
            refusal(path, &other, what).replace(" a song", " a grid")
        ),
    }
}

/// A grid as read: a drum-grid document's body, or a grid's text still to parse.
pub enum GridSource {
    Grid(Box<DrumGrid>),
    Text(String),
}

/// Reads a drum grid from a path in `form`: a document by its envelope, refused naming its
/// kind when it is not a drum grid, or a grid's text. Standard input is a document when it
/// starts with `{`. A document's grid is normalized, as a hand edit may leave it loose.
pub fn read_grid_source(path: &Path, form: GridForm, what: &str) -> Result<GridSource> {
    let bytes = bytes_of(path)?;
    if form == GridForm::Document || (path == Path::new("-") && is_document(&bytes)) {
        let text = String::from_utf8(bytes).context("a document is UTF-8 text")?;
        let doc = AnyDocument::read(&text).with_context(|| path.display().to_string())?;
        return match doc {
            AnyDocument::DrumGrid(doc) => {
                let mut grid = doc.body;
                grid.normalize()?;
                Ok(GridSource::Grid(Box::new(grid)))
            }
            other => bail!(
                "{} is a {} document, and {what} a drum grid",
                path.display(),
                other.kind()
            ),
        };
    }
    String::from_utf8(bytes)
        .context("input is not UTF-8 text")
        .map(GridSource::Text)
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

/// What `convert` read: a song, or a drum grid from a drum-grid document.
pub enum Loaded {
    Song(Song),
    Grid(DrumGrid),
}

/// Reads a song. `what` says what the command does with it, as in "duet play reads", for a
/// document of another kind. Standard input is read as a document when its first non-blank
/// byte is `{`, and as `format` otherwise.
pub fn read(path: &Path, format: Format, what: &str) -> Result<Song> {
    match read_any(path, format, what)? {
        Loaded::Song(song) => Ok(song),
        Loaded::Grid(_) => bail!(
            "{} is a drum-grid document, and {what} a song; duet drums render makes a song of \
             one",
            path.display()
        ),
    }
}

/// Reads a song, or a drum grid when the path holds a drum-grid document.
pub fn read_any(path: &Path, format: Format, what: &str) -> Result<Loaded> {
    let bytes = bytes_of(path)?;
    let format = if path == Path::new("-") && is_document(&bytes) {
        Format::Duet
    } else {
        format
    };
    let text = || String::from_utf8(bytes.clone()).context("input is not UTF-8 text");
    // MIDI, ABC and text come back normalized. A hand-edited document may hold lengths or
    // positions in a loose form, so it is normalized here.
    let song = match format {
        Format::Duet => {
            let doc = AnyDocument::read(&text()?).with_context(|| path.display().to_string())?;
            match doc {
                AnyDocument::Song(doc) => normalized(doc.body)?,
                AnyDocument::DrumGrid(doc) => {
                    let mut grid = doc.body;
                    grid.normalize()?;
                    return Ok(Loaded::Grid(grid));
                }
                other => bail!(
                    "{} is a {} document, and {what} a song",
                    path.display(),
                    other.kind()
                ),
            }
        }
        Format::Midi => Song::from_midi(&bytes)?,
        Format::Text => Song::from_text(&text()?)?,
        Format::Abc => {
            let book = duet_abc::Book::parse(&text()?)?;
            if book.tunes().len() > 1 {
                bail!(
                    "{} holds {} tunes; pick one with duet abc render --tune <X>",
                    path.display(),
                    book.tunes().len()
                );
            }
            crate::abc::reading(&book.tunes()[0])?
        }
    };
    Ok(Loaded::Song(song))
}

fn normalized(mut song: Song) -> duet_core::Result<Song> {
    song.normalize()?;
    Ok(song)
}

/// The song with its end stated: the one it names, or else the configured default's.
pub fn with_end(song: &Song, end: EndRule) -> Result<Song> {
    let mut song = song.clone();
    if song.end.is_none() {
        song.end = Some(song.default_end_by(end)?);
    }
    Ok(song)
}

/// What every document duet writes names as its tool.
pub fn tool() -> Tool {
    Tool::new("duet", env!("CARGO_PKG_VERSION"))
}

/// Writes the song in a format. Every file written says where the song ends, whatever
/// default computed it: text and MIDI state it themselves, and a document carries it as its
/// body's `end`.
pub fn write(song: &Song, path: &Path, format: Format, end: EndRule) -> Result<()> {
    let song = with_end(song, end)?;
    let bytes = match format {
        Format::Duet => Document::new(tool(), song).write()?.into_bytes(),
        Format::Midi => song.to_midi()?,
        Format::Text => song.to_text().into_bytes(),
        Format::Abc => duet_abc::write(&song, &duet_abc::WriteOptions::default())?.into_bytes(),
    };
    write_bytes(path, &bytes)
}

/// Writes to a file, or to standard output for `-`.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if path == Path::new("-") {
        io::stdout()
            .write_all(bytes)
            .context("writing standard output")
    } else {
        fs::write(path, bytes).with_context(|| path.display().to_string())
    }
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

    use duet_core::{Note, Position, Song, Track};

    use super::{Format, read, song_format, tool};
    use duet_schemas::Document;

    #[test]
    fn a_song_is_named_by_dot_duet_mid_or_abc_and_nothing_else() {
        let of = |name: &str| song_format(Path::new(name), None, Format::Text, "reads");
        assert_eq!(of("song.duet").unwrap(), Format::Duet);
        assert_eq!(of("Song.DUET").unwrap(), Format::Duet);
        assert_eq!(of("take.mid").unwrap(), Format::Midi);
        assert_eq!(of("take.smf").unwrap(), Format::Midi);
        assert_eq!(of("tune.abc").unwrap(), Format::Abc);
        assert_eq!(
            of("-").unwrap(),
            Format::Text,
            "standard streams take the default"
        );
        for (name, says) in [
            ("song.json", "no longer read"),
            ("song.toml", "no longer read"),
            ("beat.drums.json", "no longer read"),
            ("take.notes", "--from text"),
            ("take.txt", "--from text"),
            ("beat.drums", "--from text"),
            ("perf.duet.jsonl", "is a stream"),
            ("song", "name any other with --from or --to"),
        ] {
            let err = of(name).expect_err(name).to_string();
            assert!(err.contains(says), "{name}: {err}");
        }
        let given = song_format(
            Path::new("x.notes"),
            Some(Format::Text),
            Format::Midi,
            "reads",
        );
        assert_eq!(
            given.unwrap(),
            Format::Text,
            "--from says what a name does not"
        );
    }

    #[test]
    fn read_normalizes_a_document() {
        // A loose position and a length that is not whole ticks, as a hand edit might leave.
        let mut song = Song::default();
        let mut track = Track::new("P", 1);
        track.notes.push(Note::new(
            Position::new(1, 1, 960),
            "C4".parse().expect("C4 is a pitch"),
            0.3334,
            80,
        ));
        song.tracks.push(track);

        let dir = std::env::temp_dir().join(format!("duet-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("song.duet");
        let text = Document::new(tool(), song).write().expect("written");
        std::fs::write(&path, text).expect("write");
        let read = read(&path, Format::Duet, "reads").expect("read");
        let note = &read.tracks[0].notes[0];
        assert_eq!((note.at.to_string(), note.beats), ("1.3.0".into(), 0.333));
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
