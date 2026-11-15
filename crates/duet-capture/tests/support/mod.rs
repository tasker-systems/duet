//! The reference corpus: loading `corpus/manifest.toml`, running each take through the
//! library pipeline, and comparing the result with its hand-checked transcription.
//!
//! The pipeline is the one `duet capture` runs (`FileSource` → `Prepare` → `Melody` or `Hits`
//! → `estimate` → `assemble`), then `Song::quantized`; the `duet` binary is not involved.

#![expect(dead_code, reason = "each test file uses part of the support module")]

use std::path::{Path, PathBuf};

use duet_capture::{
    ANALYSIS_RATE, Event, FileSource, Given, Hits, Kind, LaneMode, Melody, Mono, Prepare, Source,
    Stage, assemble, estimate,
};
use duet_core::{Grid, Song, TimeSig};
use duet_drums::LaneMap;
use serde::Deserialize;

/// A note of at least an eighth is held to its pitch and step; shorter ones are reported.
const EIGHTH_BEATS: f64 = 0.5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub take: Vec<Take>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    /// The recording, beside the manifest.
    pub file: String,
    pub kind: TakeKind,
    pub provenance: String,
    pub licence: String,
    /// The hand-checked transcription, a `.notes` file beside the recording.
    pub expected: String,
    /// A tapped take's asked lane, a name or alias in the `gm` lane map.
    #[serde(default)]
    pub lane: Option<String>,
    /// The given tempo, in quarter notes a minute.
    #[serde(default)]
    pub tempo: Option<f64>,
    /// The given meter, such as `4/4`.
    #[serde(default)]
    pub meter: Option<String>,
    /// The given downbeat, in seconds from the start of the recording.
    #[serde(default)]
    pub downbeat: Option<f64>,
    /// The given key, as ABC names it in `K:`.
    #[serde(default)]
    pub key: Option<String>,
    /// The step both songs are quantized to. Default `1/16`.
    #[serde(default = "default_grid")]
    pub grid: String,
}

fn default_grid() -> String {
    "1/16".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TakeKind {
    Melody,
    Tapped,
    Beatboxed,
}

impl TakeKind {
    pub const ALL: [TakeKind; 3] = [TakeKind::Melody, TakeKind::Tapped, TakeKind::Beatboxed];

    pub fn name(self) -> &'static str {
        match self {
            TakeKind::Melody => "melody",
            TakeKind::Tapped => "tapped",
            TakeKind::Beatboxed => "beatboxed",
        }
    }
}

impl Take {
    /// A tapped take must name its lane, and no other kind may.
    pub fn check(&self) -> Result<(), String> {
        match (self.kind, &self.lane) {
            (TakeKind::Tapped, None) => Err("a tapped take needs `lane`".to_owned()),
            (TakeKind::Melody | TakeKind::Beatboxed, Some(lane)) => Err(format!(
                "`lane = \"{lane}\"` is for a tapped take; a {} take has none",
                self.kind.name()
            )),
            _ => Ok(()),
        }
    }

    /// Synthetic takes witness the harness, not a goal clause.
    pub fn is_synthetic(&self) -> bool {
        self.provenance.starts_with("synthetic")
    }
}

pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

pub fn manifest() -> Manifest {
    let path = corpus_dir().join("manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let manifest: Manifest =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for take in &manifest.take {
        if let Err(e) = take.check() {
            panic!("manifest entry {}: {e}", take.file);
        }
    }
    manifest
}

/// Both of a take's songs, quantized to its grid.
pub struct Reading {
    pub grid: Grid,
    pub ticks_per_quarter: u16,
    pub found: Song,
    pub expected: Song,
}

/// Runs a take through the library pipeline and reads its `expected` file.
pub fn read(take: &Take) -> Result<Reading, String> {
    let dir = corpus_dir();
    let grid: Grid = take
        .grid
        .parse()
        .map_err(|e| format!("grid {}: {e}", take.grid))?;
    let found = capture(take, &dir.join(&take.file))?
        .quantized(grid)
        .map_err(|e| format!("quantizing the capture: {e}"))?;
    let path = dir.join(&take.expected);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let expected = Song::from_text(&text)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .quantized(grid)
        .map_err(|e| format!("quantizing {}: {e}", take.expected))?;
    Ok(Reading {
        grid,
        ticks_per_quarter: found.ticks_per_quarter,
        found,
        expected,
    })
}

fn capture(take: &Take, path: &Path) -> Result<Song, String> {
    let melody = take.kind == TakeKind::Melody;
    let mode = if take.kind == TakeKind::Tapped {
        LaneMode::Asked
    } else {
        LaneMode::Beatbox
    };
    let events = analyse(path, melody, mode)?;
    let given = Given {
        tempo_bpm: take.tempo,
        meter: take
            .meter
            .as_deref()
            .map(str::parse::<TimeSig>)
            .transpose()
            .map_err(|e| format!("meter: {e}"))?,
        downbeat_secs: take.downbeat,
    };
    let timing = estimate(&events, ANALYSIS_RATE, &given).map_err(|e| e.to_string())?;
    let key = take
        .key
        .as_deref()
        .map(duet_abc::key_from_name)
        .transpose()
        .map_err(|e| format!("key: {e}"))?;
    let kind = if melody {
        Kind::Melody
    } else {
        Kind::Rhythm {
            lanes: LaneMap::gm(),
            asked: take.lane.clone(),
        }
    };
    assemble(&events, ANALYSIS_RATE, &timing, key, &kind)
        .map(|captured| captured.song)
        .map_err(|e| e.to_string())
}

fn analyse(path: &Path, melody: bool, mode: LaneMode) -> Result<Vec<Event>, String> {
    let fail = |e: duet_capture::Error| e.to_string();
    let mut source = FileSource::open(path).map_err(fail)?;
    let mut prepare = Prepare::new(ANALYSIS_RATE);
    let mut stage: Box<dyn Stage<In = Mono, Out = Event>> = if melody {
        Box::new(Melody::new(ANALYSIS_RATE))
    } else {
        Box::new(Hits::new(ANALYSIS_RATE, mode))
    };
    let (mut mono, mut events) = (Vec::new(), Vec::new());
    while let Some(chunk) = source.next_chunk().map_err(fail)? {
        prepare.push(chunk, &mut mono).map_err(fail)?;
        for m in mono.drain(..) {
            stage.push(m, &mut events).map_err(fail)?;
        }
    }
    prepare.finish(&mut mono).map_err(fail)?;
    for m in mono.drain(..) {
        stage.push(m, &mut events).map_err(fail)?;
    }
    stage.finish(&mut events).map_err(fail)?;
    Ok(events)
}

/// A note or hit of a quantized song, where the comparison sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Played {
    /// The grid step the note starts on, counted from the start of the song.
    pub step: u64,
    /// The MIDI note number: a melody's pitch, or a drum's key.
    pub midi: u8,
    /// The length in quarter notes, in thousandths.
    pub milli_beats: u64,
}

impl Played {
    fn at(&self) -> String {
        format!("step {}", self.step)
    }
}

/// Every note of the song in time order (then pitch), as steps of the grid.
pub fn played(song: &Song, grid: Grid) -> Result<Vec<Played>, String> {
    let map = song.time_map().map_err(|e| e.to_string())?;
    let step = grid
        .step_ticks(song.ticks_per_quarter)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for track in &song.tracks {
        for note in &track.notes {
            let tick = map.tick(note.at).map_err(|e| e.to_string())?;
            out.push(Played {
                step: tick / step,
                midi: note.pitch.midi(),
                milli_beats: (note.beats * 1000.0).round() as u64,
            });
        }
    }
    out.sort();
    Ok(out)
}

/// What a comparison found: failures fail the take; notes are reported only.
#[derive(Debug, Default)]
pub struct Verdict {
    pub failures: Vec<String>,
    pub reported: Vec<String>,
}

/// A melody against its transcription, aligned by the step each note starts on, over notes of
/// at least an eighth on either side. An expected long note fails when no long note found
/// starts on its step with its pitch; a long note found on a step where the transcription has
/// no note fails as invented or split. Shorter notes, on either side, are reported only.
pub fn compare_melody(take: &str, expected: &[Played], found: &[Played]) -> Verdict {
    let is_long = |p: &Played| p.milli_beats as f64 / 1000.0 >= EIGHTH_BEATS;
    let mut verdict = Verdict::default();
    for want in expected {
        let hit = found
            .iter()
            .any(|got| is_long(got) && got.step == want.step && got.midi == want.midi);
        if is_long(want) && !hit {
            verdict.failures.push(format!(
                "{take}: expected note: pitch {} at {}, found {}",
                want.midi,
                want.at(),
                near(found, want.step)
            ));
        } else if !is_long(want)
            && !found
                .iter()
                .any(|g| g.step == want.step && g.midi == want.midi)
        {
            verdict.reported.push(format!(
                "{take}: short expected note: pitch {} at {}, found {}",
                want.midi,
                want.at(),
                near(found, want.step)
            ));
        }
    }
    for got in found {
        if expected.iter().any(|want| want.step == got.step) {
            continue;
        }
        let message = format!(
            "{take}: extra note: found pitch {} at {}, expected {}",
            got.midi,
            got.at(),
            near(expected, got.step)
        );
        if is_long(got) {
            verdict.failures.push(message);
        } else {
            verdict
                .reported
                .push(format!("{message} (shorter than an eighth)"));
        }
    }
    verdict
}

/// The notes within two steps of `step`, for a failure to say what was found nearby.
fn near(notes: &[Played], step: u64) -> String {
    let nearby: Vec<String> = notes
        .iter()
        .filter(|p| p.step.abs_diff(step) <= 2)
        .map(|p| format!("pitch {} at {}", p.midi, p.at()))
        .collect();
    if nearby.is_empty() {
        "no note near".to_owned()
    } else {
        nearby.join(", ")
    }
}

/// A rhythm against its transcription as sets of (step, key): any missing or extra hit
/// fails. With `asked`, every hit found must also carry that key.
pub fn compare_rhythm(
    take: &str,
    expected: &[Played],
    found: &[Played],
    asked: Option<u8>,
) -> Verdict {
    let mut verdict = Verdict::default();
    let hit = |p: &Played| (p.step, p.midi);
    for want in expected {
        if !found.iter().any(|got| hit(got) == hit(want)) {
            verdict.failures.push(format!(
                "{take}: hit missing: expected key {} at {}, found {}",
                want.midi,
                want.at(),
                at_step(found, want.step)
            ));
        }
    }
    for got in found {
        if !expected.iter().any(|want| hit(want) == hit(got)) {
            verdict.failures.push(format!(
                "{take}: extra hit: found key {} at {}, expected {}",
                got.midi,
                got.at(),
                at_step(expected, got.step)
            ));
        }
        if let Some(key) = asked
            && got.midi != key
        {
            verdict.failures.push(format!(
                "{take}: hit at {} is key {}, but a tapped take's hits are all key {key}, the asked lane",
                got.at(),
                got.midi
            ));
        }
    }
    verdict
}

/// What a list of hits holds at `step`, for a failure to say.
fn at_step(hits: &[Played], step: u64) -> String {
    let keys: Vec<String> = hits
        .iter()
        .filter(|p| p.step == step)
        .map(|p| format!("key {}", p.midi))
        .collect();
    if keys.is_empty() {
        "no hit there".to_owned()
    } else {
        keys.join(" and ")
    }
}

/// The key of the lane a tapped take asks for.
pub fn asked_key(take: &Take) -> Result<Option<u8>, String> {
    let Some(name) = take.lane.as_deref() else {
        return Ok(None);
    };
    LaneMap::gm()
        .lookup(name)
        .map(|lane| Some(lane.key))
        .ok_or_else(|| format!("lane {name}: not in the gm lane map"))
}
