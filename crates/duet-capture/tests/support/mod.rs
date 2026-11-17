//! The reference corpus: loading `corpus/manifest.toml`, running each take through the
//! library pipeline, and comparing the result with its hand-checked transcription.
//!
//! The pipeline is the one `duet capture` runs (`FileSource` → `Prepare` → `Performance` and its
//! reading on the take's own tuning, or `Hits` → `estimate` → `assemble`), then
//! `Part::quantized`; the `duet` binary is not involved.

#![expect(dead_code, reason = "each test file uses part of the support module")]

use std::path::{Path, PathBuf};

use duet_capture::{
    ANALYSIS_RATE, Event, FileSource, Given, Hits, Kind, LaneMode, Layer1, Mono, NoteModel,
    Performance, PerformanceFrames, Prepare, ReaderName, Source, Stage, assemble, estimate,
};
use duet_core::{Grid, Part, TimeSig};
use duet_drums::LaneMap;
use serde::Deserialize;

/// A note of at least an eighth is held to its pitch and step; shorter ones are reported.
const EIGHTH_BEATS: f64 = 0.5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub take: Vec<Take>,
    /// Takes the owner has listened to and judged, but has not transcribed note by note.
    #[serde(default)]
    pub listened: Vec<Listened>,
}

/// A recorded melody whose reading the owner judged by ear: only what they agreed is held,
/// each within its stated tolerance. It has no transcription, so it witnesses no goal clause
/// and is not a [`Take`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listened {
    /// The recording, beside the manifest.
    pub file: String,
    pub provenance: String,
    pub licence: String,
    /// The licence's text or notice, beside the recording.
    pub licence_file: String,
    /// The take's digest: the SHA-256 of its decoded samples, as `duet capture` states it in
    /// `take.digest`, so a file that is not this take fails by name.
    pub digest: String,
    /// The key the HMM reading must estimate, as ABC names it in `K:`.
    pub key: String,
    /// The estimated tuning: A4's offset from 440 Hz, in cents, and how far it may move.
    pub tuning_cents: f64,
    pub tuning_within: f64,
    /// How many notes the HMM reads, and how far that may move.
    pub notes: usize,
    pub notes_within: usize,
    /// How many excursions the HMM's notes hold in all, and how far that may move.
    pub excursions: usize,
    pub excursions_within: usize,
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
    /// The step both parts are quantized to. Default `1/16`.
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

/// Both of a take's parts, quantized to its grid.
pub struct Reading {
    pub grid: Grid,
    pub ticks_per_quarter: u16,
    pub found: Part,
    pub expected: Part,
}

/// Runs a take through the library pipeline and reads its `expected` file.
pub fn read(take: &Take) -> Result<Reading, String> {
    read_by(take, ReaderName::Rules).map(|(reading, _)| reading)
}

/// [`read`], with a melody's notes chosen by `reader`; with the melody's reading too.
pub fn read_by(
    take: &Take,
    reader: ReaderName,
) -> Result<(Reading, Option<duet_capture::Reading>), String> {
    let dir = corpus_dir();
    let grid: Grid = take
        .grid
        .parse()
        .map_err(|e| format!("grid {}: {e}", take.grid))?;
    let (found, read) = capture(take, &dir.join(&take.file), reader)?;
    let found = found
        .quantized(grid)
        .map_err(|e| format!("quantizing the capture: {e}"))?;
    let path = dir.join(&take.expected);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let expected = Part::from_text(&text)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .quantized(grid)
        .map_err(|e| format!("quantizing {}: {e}", take.expected))?;
    Ok((
        Reading {
            grid,
            ticks_per_quarter: found.ticks_per_quarter,
            found,
            expected,
        },
        read,
    ))
}

fn capture(
    take: &Take,
    path: &Path,
    reader: ReaderName,
) -> Result<(Part, Option<duet_capture::Reading>), String> {
    let melody = take.kind == TakeKind::Melody;
    let mode = if take.kind == TakeKind::Tapped {
        LaneMode::Asked
    } else {
        LaneMode::Beatbox
    };
    let (events, read) = analyse(path, melody, mode, reader)?;
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
        .map(|captured| (captured.part, read))
        .map_err(|e| e.to_string())
}

/// A melody recording as `duet capture melody` reads it with nothing given: its notes read by
/// `reader`, the timing and key estimated. The capture, with the melody's reading.
pub fn capture_melody(
    path: &Path,
    reader: ReaderName,
) -> Result<(duet_capture::Captured, duet_capture::Reading), String> {
    let (events, read) = analyse(path, true, LaneMode::Beatbox, reader)?;
    let read = read.ok_or("a melody has a reading")?;
    let timing = estimate(&events, ANALYSIS_RATE, &Given::default()).map_err(|e| e.to_string())?;
    let captured = assemble(&events, ANALYSIS_RATE, &timing, None, &Kind::Melody)
        .map_err(|e| e.to_string())?;
    Ok((captured, read))
}

/// The take's events, as `duet capture` finds them: a rhythm's from [`Hits`]; a melody's onsets
/// from its [`Performance`], and its notes read from the performance on its own estimated
/// tuning by `reader` ([`duet_capture::read`] or [`NoteModel`]), with that reading.
fn analyse(
    path: &Path,
    melody: bool,
    mode: LaneMode,
    reader: ReaderName,
) -> Result<(Vec<Event>, Option<duet_capture::Reading>), String> {
    // The source's errors are duet-capture's, the stages' duet-dsp's: both become their message.
    let fail = |e: duet_capture::Error| e.to_string();
    let mut source = FileSource::open(path).map_err(fail)?;
    let mut prepare = Prepare::new(ANALYSIS_RATE);
    let mut performance = Performance::new(ANALYSIS_RATE);
    let mut hits = Hits::new(ANALYSIS_RATE, mode);
    let (mut mono, mut events, mut layers) = (Vec::new(), Vec::new(), Vec::new());
    // Basic Pitch reads the samples themselves, as `duet capture` keeps them for it.
    let mut samples: Vec<f32> = Vec::new();
    let keep = melody && reader == ReaderName::BasicPitch;
    let mut push = |m: Mono, events: &mut Vec<Event>, layers: &mut Vec<Layer1>| {
        if melody {
            performance.push(m, layers)
        } else {
            hits.push(m, events)
        }
    };
    while let Some(chunk) = source.next_chunk().map_err(fail)? {
        prepare.push(chunk, &mut mono).map_err(|e| fail(e.into()))?;
        for m in mono.drain(..) {
            if keep {
                samples.extend_from_slice(&m.samples);
            }
            push(m, &mut events, &mut layers).map_err(|e| fail(e.into()))?;
        }
    }
    prepare.finish(&mut mono).map_err(|e| fail(e.into()))?;
    for m in mono.drain(..) {
        if keep {
            samples.extend_from_slice(&m.samples);
        }
        push(m, &mut events, &mut layers).map_err(|e| fail(e.into()))?;
    }
    if !melody {
        hits.finish(&mut events).map_err(|e| fail(e.into()))?;
        return Ok((events, None));
    }
    performance
        .finish(&mut layers)
        .map_err(|e| fail(e.into()))?;
    let mut frames = None;
    for layer in layers {
        match layer {
            Layer1::Head(head) => frames = Some(PerformanceFrames::new(&head)),
            Layer1::Frames(block) => frames
                .as_mut()
                .ok_or("frames before the head")?
                .push(&block)
                .map_err(|e| fail(e.into()))?,
            Layer1::Onset { frame, strength } => events.push(Event::Onset { frame, strength }),
            _ => {}
        }
    }
    let frames = frames.ok_or("the take has no frames")?;
    let reading = match reader {
        ReaderName::Rules => duet_capture::read(&frames, &events, None),
        ReaderName::Hmm => NoteModel::default().read(&frames, &events, None),
        #[cfg(feature = "basic-pitch")]
        ReaderName::BasicPitch => {
            duet_capture::BasicPitch::new()
                .and_then(|bp| bp.read(&samples, &frames, &events, None))
                .map_err(|e| fail(e.into()))?
                .reading
        }
        #[cfg(not(feature = "basic-pitch"))]
        ReaderName::BasicPitch => return Err("basic-pitch is not built here".to_owned()),
    };
    events.extend(reading.notes.iter().map(|n| n.event()));
    events.sort_by_key(|e| match *e {
        Event::Onset { frame, .. } | Event::Hit { frame, .. } => frame,
        Event::Note { start, .. } => start,
        _ => u64::MAX,
    });
    Ok((events, Some(reading)))
}

/// A note or hit of a quantized part, where the comparison sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Played {
    /// The grid step the note starts on, counted from the start of the part.
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

/// Every note of the part in time order (then pitch), as steps of the grid.
pub fn played(part: &Part, grid: Grid) -> Result<Vec<Played>, String> {
    let map = part.time_map().map_err(|e| e.to_string())?;
    let step = grid
        .step_ticks(part.ticks_per_quarter)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for track in &part.tracks {
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
/// at least an eighth on either side, each note matched to at most one on the other side by its
/// step and pitch:
/// - every expected long note needs a long note found of its own on its step with its pitch, or
///   it fails, so two expected on one step need two found;
/// - every long note found must then match an expected note on its step with its pitch, or it
///   fails as invented, split or wrong, whatever else the transcription has on that step;
/// - shorter notes, on either side, are reported only: an expected short note with no note
///   found to match, and a short note found on a step where nothing is expected.
pub fn compare_melody(take: &str, expected: &[Played], found: &[Played]) -> Verdict {
    let is_long = |p: &Played| p.milli_beats as f64 / 1000.0 >= EIGHTH_BEATS;
    let same = |a: &Played, b: &Played| a.step == b.step && a.midi == b.midi;
    let mut verdict = Verdict::default();
    let (mut want_used, mut got_used) = (vec![false; expected.len()], vec![false; found.len()]);
    // Takes the first note of `notes` not yet used, that `fits`, and marks it used.
    let take_first = |notes: &[Played], used: &mut [bool], fits: &dyn Fn(&Played) -> bool| {
        let i = (0..notes.len()).find(|&i| !used[i] && fits(&notes[i]))?;
        used[i] = true;
        Some(i)
    };

    for (w, want) in expected.iter().enumerate() {
        if is_long(want) {
            want_used[w] = true;
            if take_first(found, &mut got_used, &|got| is_long(got) && same(got, want)).is_none() {
                verdict.failures.push(format!(
                    "{take}: expected note: pitch {} at {}, found {}",
                    want.midi,
                    want.at(),
                    near(found, want.step)
                ));
            }
        }
    }
    for (g, got) in found.iter().enumerate() {
        if got_used[g] || !is_long(got) {
            continue;
        }
        got_used[g] = true;
        if take_first(expected, &mut want_used, &|want| same(want, got)).is_none() {
            verdict.failures.push(format!(
                "{take}: extra note: found pitch {} at {}, expected {}",
                got.midi,
                got.at(),
                near(expected, got.step)
            ));
        }
    }
    for (w, want) in expected.iter().enumerate() {
        if want_used[w] {
            continue;
        }
        if take_first(found, &mut got_used, &|got| same(got, want)).is_none() {
            verdict.reported.push(format!(
                "{take}: short expected note: pitch {} at {}, found {}",
                want.midi,
                want.at(),
                near(found, want.step)
            ));
        }
    }
    for (g, got) in found.iter().enumerate() {
        if got_used[g] || expected.iter().any(|want| want.step == got.step) {
            continue;
        }
        verdict.reported.push(format!(
            "{take}: extra note: found pitch {} at {}, expected {} (shorter than an eighth)",
            got.midi,
            got.at(),
            near(expected, got.step)
        ));
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
