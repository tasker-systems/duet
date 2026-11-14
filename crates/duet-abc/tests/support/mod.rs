//! The corpus and its recordings of abc2midi's reading, shared by the round-trip and
//! oracle tests.
//!
//! A recording is the notefmt text of the MIDI abc2midi made from a tune, read through
//! duet-midi exactly as read. `manifest.toml` records the abc2midi version, the preamble
//! and the arguments that made them, and for each tune its file, `X`, provenance, the
//! constructs it exercises, and the differences from abc2midi it lists, each with its
//! reason.

#![expect(dead_code, reason = "each test file uses part of the support module")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use duet_abc::{Book, ReadOptions, WriteOptions};
use duet_core::Song;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// What `abc2midi -ver` prints, exactly.
    pub abc2midi_version: String,
    /// Lines inserted after the tune's `X:` line before abc2midi reads it.
    pub preamble: Vec<String>,
    /// Arguments after `abc2midi <file> <X> -o <out.mid>`.
    pub arguments: Vec<String>,
    pub tune: Vec<TuneEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuneEntry {
    /// The recordings are `<name>.abc2midi.txt`, `<name>.written.abc` and
    /// `<name>.written.abc2midi.txt`.
    pub name: String,
    pub file: String,
    pub x: u32,
    pub provenance: String,
    pub constructs: Vec<String>,
    /// Why our writer refuses this tune, when it does: it then has no written files, and
    /// the tests check the writer still refuses it.
    #[serde(default)]
    pub writer_refuses: Option<String>,
    #[serde(default)]
    pub difference: Vec<Difference>,
}

/// A difference between our reading and abc2midi's, listed with its reason. It is
/// subtracted exactly, and a listed difference that no longer occurs fails.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Difference {
    /// `velocity`, `start`, `length`, `channel`, `extra-note` or `missing-note`.
    pub what: String,
    /// Which of our notes: `all`, or a half-open range of our positions,
    /// `3.1.0..5.1.0`. Unused for `extra-note`.
    #[serde(default)]
    pub notes: String,
    /// Our value (ticks for `start` and `length`); for `missing-note`, our note as
    /// `tick length midi channel velocity`.
    #[serde(default)]
    pub ours: Option<toml::Value>,
    /// abc2midi's value; for `extra-note`, its note in the same form.
    #[serde(default)]
    pub theirs: Option<toml::Value>,
    pub reason: String,
}

/// A note as the comparison sees it: absolute tick, length in ticks, MIDI pitch,
/// channel and velocity. Track names, track order, titles and meta text are not compared.
pub type N = (u64, u64, u8, u8, u8);

pub fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

pub fn manifest() -> Manifest {
    let path = corpus().join("manifest.toml");
    let text = std::fs::read_to_string(&path).expect("corpus/manifest.toml");
    toml::from_str(&text).expect("the manifest parses")
}

pub fn read_file(name: &str) -> String {
    let path = corpus().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Our reading of a corpus tune.
pub fn our_song(entry: &TuneEntry) -> Song {
    let text = read_file(&entry.file);
    let book = Book::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
    let tune = book
        .tune(entry.x)
        .unwrap_or_else(|| panic!("{}: no X:{} in {}", entry.name, entry.x, entry.file));
    tune.to_song(&ReadOptions::default())
        .unwrap_or_else(|e| panic!("{}: {e}", entry.name))
        .song
}

/// Our writer's ABC for a corpus tune: `write(read(tune))`.
pub fn our_written(entry: &TuneEntry) -> String {
    duet_abc::write(&our_song(entry), &WriteOptions::default())
        .unwrap_or_else(|e| panic!("{}: writing: {e}", entry.name))
}

/// A recording, read back as a song.
pub fn recording(file: &str) -> Result<Song, String> {
    let path = corpus().join(file);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{} has no recording ({e}); record it with a local run",
            path.display()
        )
    })?;
    Song::from_text(&text).map_err(|e| format!("{file}: {e}"))
}

pub fn notes_of(song: &Song) -> Vec<N> {
    let map = song.time_map().expect("time map");
    let tpq = song.ticks_per_quarter;
    let mut out: Vec<N> = song
        .tracks
        .iter()
        .flat_map(|t| {
            t.notes.iter().map(|n| {
                (
                    map.tick(n.at).expect("tick"),
                    duet_core::time::ticks_from_beats(n.beats, tpq).expect("ticks"),
                    n.pitch.midi(),
                    t.channel,
                    n.vel,
                )
            })
        })
        .collect::<Vec<_>>();
    out.sort_unstable();
    out
}

/// abc2midi starts every note one tick late and ends it on time; this maps its notes
/// back. It is the harness's one fixed rule, applied to every tune.
pub fn mapped_back(theirs: &[N]) -> Result<Vec<N>, String> {
    let mut out = Vec::with_capacity(theirs.len());
    for &(start, len, midi, ch, vel) in theirs {
        if start == 0 {
            return Err(format!(
                "abc2midi's note {midi} on channel {ch} starts at tick 0, so it no longer \
                 starts notes one tick late; the harness's fixed rule needs revisiting"
            ));
        }
        out.push((start - 1, len + 1, midi, ch, vel));
    }
    out.sort_unstable();
    Ok(out)
}

fn parse_note(v: &toml::Value) -> Result<N, String> {
    let s = v
        .as_str()
        .ok_or("a note is a string 'tick length midi channel velocity'")?;
    let parts: Vec<u64> = s
        .split_whitespace()
        .map(|p| p.parse::<u64>().map_err(|_| format!("'{s}' is not a note")))
        .collect::<Result<_, _>>()?;
    let [t, l, m, c, v] = parts[..] else {
        return Err(format!("'{s}' is not 'tick length midi channel velocity'"));
    };
    Ok((t, l, m as u8, c as u8, v as u8))
}

fn as_u64(v: &Option<toml::Value>, what: &str) -> Result<u64, String> {
    v.as_ref()
        .and_then(toml::Value::as_integer)
        .and_then(|i| u64::try_from(i).ok())
        .ok_or_else(|| format!("a {what} difference needs integer ours and theirs"))
}

/// Applies each listed difference to our notes, so they read as abc2midi's would. Fails
/// when a difference no longer occurs: no note it selects has its `ours` value.
pub fn apply_differences(song: &Song, ours: &[N], diffs: &[Difference]) -> Result<Vec<N>, String> {
    let map = song.time_map().map_err(|e| e.to_string())?;
    let mut out = ours.to_vec();
    for d in diffs {
        let range = match d.notes.as_str() {
            "" | "all" => 0..u64::MAX,
            r => {
                let (a, b) = r
                    .split_once("..")
                    .ok_or_else(|| format!("'{r}' is not 'all' or a range like 3.1.0..5.1.0"))?;
                let tick = |p: &str| {
                    p.parse()
                        .map_err(|e: duet_core::Error| e.to_string())
                        .and_then(|p| map.tick(p).map_err(|e| e.to_string()))
                };
                tick(a)?..tick(b)?
            }
        };
        let field: fn(&mut N) -> &mut u64 = match d.what.as_str() {
            "extra-note" => {
                out.push(parse_note(
                    d.theirs.as_ref().ok_or("extra-note needs theirs")?,
                )?);
                continue;
            }
            "missing-note" => {
                let n = parse_note(d.ours.as_ref().ok_or("missing-note needs ours")?)?;
                let i = out
                    .iter()
                    .position(|o| *o == n)
                    .ok_or_else(|| format!("listed difference no longer occurs: {d:?}"))?;
                out.remove(i);
                continue;
            }
            "start" => |n| &mut n.0,
            "length" => |n| &mut n.1,
            "velocity" | "channel" => {
                let (ours_v, theirs_v) = (as_u64(&d.ours, &d.what)?, as_u64(&d.theirs, &d.what)?);
                let mut hit = false;
                for n in out.iter_mut().filter(|n| range.contains(&n.0)) {
                    let slot = if d.what == "velocity" {
                        &mut n.4
                    } else {
                        &mut n.3
                    };
                    if u64::from(*slot) == ours_v {
                        *slot = theirs_v as u8;
                        hit = true;
                    }
                }
                if !hit {
                    return Err(format!("listed difference no longer occurs: {d:?}"));
                }
                continue;
            }
            other => return Err(format!("'{other}' is not a kind of difference")),
        };
        let (ours_v, theirs_v) = (as_u64(&d.ours, &d.what)?, as_u64(&d.theirs, &d.what)?);
        let mut hit = false;
        for n in out.iter_mut().filter(|n| range.contains(&n.0)) {
            let slot = field(n);
            if *slot == ours_v {
                *slot = theirs_v;
                hit = true;
            }
        }
        if !hit {
            return Err(format!("listed difference no longer occurs: {d:?}"));
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// Compares our song with a recording of abc2midi's reading, under the fixed rule and
/// the listed differences. `Ok` holds the number of notes compared.
pub fn compare(ours_song: &Song, recording: &Song, diffs: &[Difference]) -> Result<usize, String> {
    let ours = apply_differences(ours_song, &notes_of(ours_song), diffs)?;
    let raw = notes_of(recording);
    let theirs = mapped_back(&raw)?;
    if ours == theirs {
        return Ok(ours.len());
    }
    let count = |v: &[N]| {
        let mut m: BTreeMap<N, i64> = BTreeMap::new();
        for n in v {
            *m.entry(*n).or_default() += 1;
        }
        m
    };
    let (a, b) = (count(&ours), count(&theirs));
    let mut lines = Vec::new();
    for (n, k) in &a {
        let other = b.get(n).copied().unwrap_or(0);
        if *k > other {
            lines.push(format!("  ours only:   {n:?} x{}", k - other));
        }
    }
    for (n, k) in &b {
        let other = a.get(n).copied().unwrap_or(0);
        if *k > other {
            lines.push(format!("  theirs only: {n:?} x{}", k - other));
        }
    }
    lines.truncate(24);
    Err(format!(
        "{} of ours, {} of theirs (tick, length, midi, channel, velocity):\n{}",
        ours.len(),
        theirs.len(),
        lines.join("\n")
    ))
}
