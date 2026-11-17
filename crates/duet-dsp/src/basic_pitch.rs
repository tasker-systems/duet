//! The basic-pitch reader: Spotify's Basic Pitch, an outside note model, reads where a take's
//! notes start and end, and the take's own performance gives each note its key, centre and
//! residual, on the take's tuning, as it does for the rules and the note HMM.
//!
//! Basic Pitch runs through `basic-pitch-rs`, an unofficial Rust port that is not affiliated
//! with Spotify, on its pure-Rust `tract` backend. Its model, Spotify's `nmp.onnx` from
//! basic-pitch v0.4.0, is embedded from `models/basic-pitch/` (Apache-2.0; its licence and
//! Spotify's NOTICE are beside it), and its SHA-256 is pinned as [`MODEL_SHA256`].
//!
//! Basic Pitch is polyphonic and a reading is not: where its notes overlap, the louder one (by
//! Basic Pitch's own amplitude) is kept whole, and a quieter one is cut back to where it no
//! longer overlaps when that keeps at least half of it, or dropped when it does not. A note
//! with no voiced frame of the performance under it has no pitch to key it on, and is dropped.
//! [`BasicPitchRead`] counts each, for the caller to state.
//!
//! A note starts at the take's onset nearest its start, when one is within
//! `ONSET_GRACE_SECS` (50 ms) of it, as the note HMM starts its notes: Basic Pitch's own
//! frames fall up to a hop either side of the onset the performance hears, and the same-key
//! rejoin keeps a split only where an onset falls at it. Then the reading's interpretive passes
//! ([`interpret`](crate::interpret)), the breathy ornament, the same-key rejoin and the held
//! pitch, run over the notes, as they do over the other readers':
//! [`BasicPitch::read_as_decoded`] is the reading before them.
//!
//! Basic Pitch decodes its notes with the thresholds [`BasicPitch::with_thresholds`] sets,
//! [`Thresholds::BASIC_PITCH`] unless it is called, and the reading records them.

use basic_pitch::model::TractBackend;
use basic_pitch::{BasicPitch as Model, NoteEvent, NoteParams};

use crate::error::{Error, Result};
use crate::event::Event;
use crate::notes::{ONSET_GRACE_SECS, level_strength, median};
use crate::reading::{
    PerformanceFrames, ReadNote, Reader, Reading, Thresholds, interpret, tuning_of,
};
use crate::residual::residual;

/// Spotify's Basic Pitch note model, `nmp.onnx` from basic-pitch v0.4.0, unmodified.
const MODEL: &[u8] = include_bytes!("../models/basic-pitch/nmp.onnx");

/// The SHA-256 of the embedded model, in hex: the file as Spotify publishes it at v0.4.0.
pub const MODEL_SHA256: &str = "2c3c1d144bfa61ad236e92e169c13535c880469a12a047d4e73451f2c059a0ec";

/// The sample rate Basic Pitch reads at, which is also duet's analysis rate.
const RATE: u32 = basic_pitch::constants::AUDIO_SAMPLE_RATE;

/// Basic Pitch, loaded, ready to read takes.
#[derive(Debug)]
pub struct BasicPitch {
    model: Model,
    thresholds: Thresholds,
}

/// A basic-pitch reading, and what became of Basic Pitch's notes on the way to it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BasicPitchRead {
    pub reading: Reading,
    /// How many notes Basic Pitch found.
    pub found: usize,
    /// How many were cut back where a louder note overlapped them.
    pub trimmed: usize,
    /// How many were dropped: overlapped for more than half their length.
    pub overlapped: usize,
    /// How many were dropped for having no voiced frame of the performance under them.
    pub unvoiced: usize,
    /// How many of the notes read sit on a different key on the take's tuning than the one
    /// Basic Pitch gave them on A440.
    pub rekeyed: usize,
}

impl BasicPitch {
    /// Loads the embedded model.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if `tract` cannot prepare the model, which the embedded model, pinned
    /// by its hash, never gives.
    pub fn new() -> Result<BasicPitch> {
        let backend = TractBackend::from_onnx_bytes(MODEL)
            .map_err(|e| Error::Invalid(format!("basic-pitch: the model: {e}")))?;
        Ok(BasicPitch {
            model: Model::with_backend(backend),
            thresholds: Thresholds::BASIC_PITCH,
        })
    }

    /// Decodes notes with `thresholds` in place of Basic Pitch's defaults.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if Basic Pitch refuses them: a threshold that is not a number, a
    /// negative frame threshold, or a minimum length that is negative or not finite.
    pub fn with_thresholds(self, thresholds: Thresholds) -> Result<BasicPitch> {
        params(thresholds)
            .validate()
            .map_err(|e| Error::Invalid(format!("basic-pitch: {e}")))?;
        Ok(BasicPitch { thresholds, ..self })
    }

    /// The thresholds it decodes notes with.
    #[must_use]
    pub fn thresholds(&self) -> Thresholds {
        self.thresholds
    }

    /// Reads `samples`, the take's mono audio from its first frame at duet's analysis rate, over
    /// `frames`, its performance, on `given`, A4 in absolute cents, or on the tuning estimated
    /// from the frames. Then the reading's interpretive passes ([`interpret`](crate::interpret)),
    /// the breathy ornament, the same-key rejoin and the held pitch, run over the notes, with
    /// `onsets`, the take's (other events are ignored).
    ///
    /// # Errors
    ///
    /// As [`BasicPitch::read_as_decoded`].
    pub fn read(
        &self,
        samples: &[f32],
        frames: &PerformanceFrames,
        onsets: &[Event],
        given: Option<f64>,
    ) -> Result<BasicPitchRead> {
        let read = self.read_as_decoded(samples, frames, onsets, given)?;
        Ok(BasicPitchRead {
            reading: interpret(read.reading, frames, onsets),
            ..read
        })
    }

    /// [`BasicPitch::read`], before its interpretive passes: the notes as Basic Pitch decodes them,
    /// one at a time, each started at an onset within `ONSET_GRACE_SECS` (50 ms) of its
    /// start, and keyed on the take's tuning.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the frames are not at Basic Pitch's rate, or Basic Pitch refuses
    /// the samples (a sample that is not finite).
    pub fn read_as_decoded(
        &self,
        samples: &[f32],
        frames: &PerformanceFrames,
        onsets: &[Event],
        given: Option<f64>,
    ) -> Result<BasicPitchRead> {
        if frames.rate != RATE {
            return Err(Error::Invalid(format!(
                "basic-pitch reads at {RATE} Hz; the performance is at {} Hz",
                frames.rate
            )));
        }
        let transcription = self
            .model
            .transcribe_with(samples, &params(self.thresholds))
            .map_err(|e| Error::Invalid(format!("basic-pitch: {e}")))?;
        let found = transcription.notes.len();
        let spans = spans_of(&transcription.notes, frames.rate);
        let (spans, trimmed, overlapped) = monophonic(spans);
        let onsets: Vec<u64> = onsets
            .iter()
            .filter_map(|e| match *e {
                Event::Onset { frame, .. } => Some(frame),
                _ => None,
            })
            .collect();
        let grace = (ONSET_GRACE_SECS * f64::from(frames.rate)).round() as u64;
        let spans = at_onsets(spans, &onsets, grace);

        let tuning = tuning_of(frames, given);
        let offset = tuning.offset_cents();
        let (mut notes, mut unvoiced, mut rekeyed) = (Vec::new(), 0, 0);
        for span in spans {
            let under: Vec<usize> = (0..frames.len())
                .filter(|&k| (span.start..span.end).contains(&frames.frame(k)))
                .collect();
            let held: Vec<f32> = under
                .iter()
                .map(|&k| f64::from(frames.cents[k]))
                .filter(|c| !c.is_nan())
                .map(|c| ((c - offset) / 100.0) as f32)
                .collect();
            if held.is_empty() {
                unvoiced += 1;
                continue;
            }
            let count = under.len() as f32;
            let confidence = under.iter().map(|&k| frames.voicing[k]).sum::<f32>() / count;
            let level = under.iter().map(|&k| frames.level[k]).sum::<f32>() / count;
            // An outside model says nothing of where a note settles, so the whole note is
            // taken as held: a scoop into it is read as its drift and rest, not as a scoop.
            let note = ReadNote::held_at(median(&held), span.start, span.end, span.start)
                .with(confidence, level_strength(level));
            if note.key != span.pitch {
                rekeyed += 1;
            }
            notes.push(note);
        }
        let (parts, take) = residual(frames, &tuning, &notes);
        Ok(BasicPitchRead {
            reading: Reading::of(
                tuning,
                Reader::basic_pitch(self.thresholds),
                notes,
                parts,
                take,
            ),
            found,
            trimmed,
            overlapped,
            unvoiced,
            rekeyed,
        })
    }
}

/// Basic Pitch's own parameters, with `thresholds` and its defaults for the rest.
fn params(thresholds: Thresholds) -> NoteParams {
    NoteParams {
        onset_threshold: thresholds.onset_threshold,
        frame_threshold: thresholds.frame_threshold,
        minimum_note_length_ms: thresholds.minimum_note_length_ms,
        ..NoteParams::default()
    }
}

/// A Basic Pitch note on the take's timeline, in frames at `rate`: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    start: u64,
    end: u64,
    pitch: u8,
    amplitude: f32,
}

impl Span {
    fn len(&self) -> u64 {
        self.end - self.start
    }
}

fn spans_of(notes: &[NoteEvent], rate: u32) -> Vec<Span> {
    let at = |secs: f64| (secs * f64::from(rate)).round().max(0.0) as u64;
    notes
        .iter()
        .map(|n| Span {
            start: at(n.start_s),
            end: at(n.end_s),
            pitch: n.pitch,
            amplitude: n.amplitude,
        })
        .filter(|s| s.end > s.start)
        .collect()
}

/// `spans`, in order of time and apart, each started at the onset nearest its start when one is
/// within `grace` frames of it and inside the span, and the note before ended there if it ran
/// past it. A start never moves to before the note before starts.
fn at_onsets(mut spans: Vec<Span>, onsets: &[u64], grace: u64) -> Vec<Span> {
    for i in 0..spans.len() {
        let span = spans[i];
        let floor = if i > 0 { spans[i - 1].start + 1 } else { 0 };
        let nearest = onsets
            .iter()
            .copied()
            .filter(|&f| f.abs_diff(span.start) <= grace && f < span.end && f >= floor)
            .min_by_key(|&f| f.abs_diff(span.start));
        if let Some(onset) = nearest {
            spans[i].start = onset;
            if i > 0 && spans[i - 1].end > onset {
                spans[i - 1].end = onset;
            }
        }
    }
    spans
}

/// One note at a time, from Basic Pitch's overlapping notes, in order of time, with how many
/// were cut back and how many dropped. Louder notes are placed first and kept whole; a quieter
/// note keeps the longest part of it outside every louder one, if that is at least half of it.
fn monophonic(mut spans: Vec<Span>) -> (Vec<Span>, usize, usize) {
    spans.sort_by(|a, b| {
        b.amplitude
            .total_cmp(&a.amplitude)
            .then(a.start.cmp(&b.start))
    });
    let (mut kept, mut trimmed, mut dropped): (Vec<Span>, usize, usize) = (Vec::new(), 0, 0);
    for span in spans {
        let mut free = vec![(span.start, span.end)];
        for k in &kept {
            free = free
                .into_iter()
                .flat_map(|(s, e)| {
                    if k.end <= s || k.start >= e {
                        vec![(s, e)]
                    } else {
                        [(s, k.start.max(s)), (k.end.min(e), e)]
                            .into_iter()
                            .filter(|(a, b)| b > a)
                            .collect()
                    }
                })
                .collect();
        }
        let longest = free.into_iter().max_by_key(|(s, e)| e - s);
        match longest {
            Some((s, e)) if 2 * (e - s) >= span.len() => {
                if (s, e) != (span.start, span.end) {
                    trimmed += 1;
                }
                kept.push(Span {
                    start: s,
                    end: e,
                    ..span
                });
            }
            _ => dropped += 1,
        }
    }
    kept.sort_by_key(|s| s.start);
    (kept, trimmed, dropped)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::performance::{Layer1, Performance};
    use crate::reading::{A440_CENTS, ReaderName};
    use crate::stage::{Mono, Stage};
    use crate::synth;

    /// A melody in C major, one key number per note, each `secs` long.
    const TUNE: [(u8, f64); 9] = [
        (60, 0.4),
        (62, 0.3),
        (64, 0.5),
        (65, 0.3),
        (67, 0.6),
        (69, 0.3),
        (67, 0.4),
        (64, 0.4),
        (60, 0.7),
    ];

    fn hz(key: u8, cents: f64) -> f64 {
        440.0 * ((f64::from(key) - 69.0 + cents / 100.0) / 12.0).exp2()
    }

    /// The tune `cents` off A440, each note `scatter` cents off that, with a quarter second of
    /// silence either side.
    fn tune_scattered(cents: f64, scatter: [f64; 9]) -> Vec<f32> {
        let tones: Vec<(f64, f64)> = TUNE
            .iter()
            .zip(scatter)
            .map(|(&(k, s), off)| (hz(k, cents + off), s))
            .collect();
        [
            synth::silence(RATE, 0.25),
            synth::note_sequence(RATE, &tones, 4, 0.3),
            synth::silence(RATE, 0.25),
        ]
        .concat()
    }

    fn tune(cents: f64) -> Vec<f32> {
        tune_scattered(cents, [0.0; 9])
    }

    /// The take's performance, as `duet capture` keeps it, and its onsets.
    fn performance(signal: &[f32]) -> (PerformanceFrames, Vec<Event>) {
        let mut stage = Performance::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(4096).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: (i * 4096) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        let (mut frames, mut onsets) = (None, Vec::new());
        for layer in out {
            match layer {
                Layer1::Head(head) => frames = Some(PerformanceFrames::new(&head)),
                Layer1::Frames(block) => frames.as_mut().unwrap().push(&block).unwrap(),
                Layer1::Onset { frame, strength } => onsets.push(Event::Onset { frame, strength }),
            }
        }
        (frames.unwrap(), onsets)
    }

    fn keys(reading: &Reading) -> Vec<u8> {
        reading.notes.iter().map(|n| n.key).collect()
    }

    fn span(start: u64, end: u64, amplitude: f32) -> Span {
        Span {
            start,
            end,
            pitch: 60,
            amplitude,
        }
    }

    #[test]
    fn the_embedded_model_is_spotifys_v0_4_0_file() {
        let digest = Sha256::digest(MODEL);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, MODEL_SHA256);
        assert_eq!(MODEL.len(), 230_444);
    }

    #[test]
    fn the_default_thresholds_are_basic_pitchs_own() {
        let own = NoteParams::default();
        assert_eq!(
            Thresholds::BASIC_PITCH,
            Thresholds {
                onset_threshold: own.onset_threshold,
                frame_threshold: own.frame_threshold,
                minimum_note_length_ms: own.minimum_note_length_ms,
            }
        );
        assert_eq!(params(Thresholds::BASIC_PITCH), own);
    }

    #[test]
    fn notes_are_decoded_with_the_thresholds_given_and_the_reading_records_them() {
        // Longer than 450 ms: only the tune's 0.5, 0.6 and 0.7 s notes.
        let signal = tune(0.0);
        let (frames, onsets) = performance(&signal);
        let long = Thresholds {
            minimum_note_length_ms: 450.0,
            ..Thresholds::BASIC_PITCH
        };
        let model = BasicPitch::new().unwrap().with_thresholds(long).unwrap();
        let read = model.read(&signal, &frames, &onsets, None).unwrap();
        assert_eq!(keys(&read.reading), [64, 67, 60]);
        assert_eq!(read.reading.reader.and_then(|r| r.thresholds), Some(long));
        let read = BasicPitch::new()
            .unwrap()
            .read(&signal, &frames, &onsets, None)
            .unwrap();
        assert_eq!(
            read.reading.reader.and_then(|r| r.thresholds),
            Some(Thresholds::BASIC_PITCH)
        );
    }

    #[test]
    fn thresholds_basic_pitch_refuses_are_refused() {
        let negative = Thresholds {
            frame_threshold: -0.1,
            ..Thresholds::BASIC_PITCH
        };
        let err = BasicPitch::new()
            .unwrap()
            .with_thresholds(negative)
            .unwrap_err();
        assert!(err.to_string().contains("frame_threshold"), "{err}");
        let nan = Thresholds {
            onset_threshold: f64::NAN,
            ..Thresholds::BASIC_PITCH
        };
        assert!(BasicPitch::new().unwrap().with_thresholds(nan).is_err());
    }

    #[test]
    fn a_synthetic_melody_reads_its_keys_one_note_at_a_time() {
        let signal = tune(0.0);
        let (frames, onsets) = performance(&signal);
        let read = BasicPitch::new()
            .unwrap()
            .read(&signal, &frames, &onsets, None)
            .unwrap();
        assert_eq!(keys(&read.reading), TUNE.map(|t| t.0));
        assert_eq!(
            read.reading.reader.map(|r| r.name),
            Some(ReaderName::BasicPitch)
        );
        for pair in read.reading.notes.windows(2) {
            assert!(pair[0].end <= pair[1].start, "{pair:?} overlap");
        }
        assert!(read.reading.notes.iter().all(|n| n.residual.is_some()));
        assert_eq!(read.unvoiced, 0);
    }

    #[test]
    fn a_melody_sung_sharp_is_keyed_on_the_takes_tuning_not_on_a440() {
        // 44 cents sharp, as the owner's takes sit, and each note up to 12 cents off that, as a
        // singer is: on A440, a note 44 + 12 cents sharp is keyed a key up. The take's tuning,
        // estimated, keys it as the in-tune melody.
        let signal = tune_scattered(44.0, [0.0, 12.0, -12.0, 12.0, 0.0, -12.0, 12.0, 0.0, -12.0]);
        let (frames, onsets) = performance(&signal);
        let model = BasicPitch::new().unwrap();
        let on_tuning = model.read(&signal, &frames, &onsets, None).unwrap();
        assert_eq!(keys(&on_tuning.reading), TUNE.map(|t| t.0));
        let on_a440 = model
            .read(&signal, &frames, &onsets, Some(A440_CENTS))
            .unwrap();
        assert_ne!(keys(&on_a440.reading), TUNE.map(|t| t.0));
    }

    #[test]
    fn a_louder_note_is_kept_whole_and_a_quieter_one_cut_back_or_dropped() {
        // Overlapped for a fifth of its length: cut back to the rest of it.
        let (kept, trimmed, dropped) = monophonic(vec![span(0, 100, 0.9), span(80, 180, 0.5)]);
        assert_eq!(kept, vec![span(0, 100, 0.9), span(100, 180, 0.5)]);
        assert_eq!((trimmed, dropped), (1, 0));
        // Overlapped for most of its length, as a ghost an octave away is: dropped.
        let (kept, trimmed, dropped) = monophonic(vec![span(0, 100, 0.9), span(10, 90, 0.4)]);
        assert_eq!(kept, vec![span(0, 100, 0.9)]);
        assert_eq!((trimmed, dropped), (0, 1));
        // Apart: both kept as they are, in order of time.
        let (kept, trimmed, dropped) = monophonic(vec![span(200, 300, 0.2), span(0, 100, 0.9)]);
        assert_eq!(kept, vec![span(0, 100, 0.9), span(200, 300, 0.2)]);
        assert_eq!((trimmed, dropped), (0, 0));
    }

    #[test]
    fn a_note_starts_at_an_onset_within_the_grace_and_never_overlaps_the_note_before() {
        // An onset 300 frames after the second note's start (14 ms): it starts there, and the
        // first keeps the end Basic Pitch gave it. One 3000 frames off (136 ms) is too far.
        let spans = vec![span(0, 5000, 0.9), span(5000, 9000, 0.9)];
        let moved = at_onsets(spans.clone(), &[5300], 1103);
        assert_eq!(moved, vec![span(0, 5000, 0.9), span(5300, 9000, 0.9)]);
        assert_eq!(at_onsets(spans.clone(), &[8000], 1103), spans);
        // An onset before the note's start moves it earlier, cutting the note before short.
        let moved = at_onsets(spans, &[4700], 1103);
        assert_eq!(moved, vec![span(0, 4700, 0.9), span(4700, 9000, 0.9)]);
    }

    #[test]
    fn frames_at_another_rate_are_refused() {
        let (mut frames, _) = performance(&tune(0.0));
        frames.rate = 44_100;
        let err = BasicPitch::new()
            .unwrap()
            .read_as_decoded(&[], &frames, &[], None)
            .unwrap_err();
        assert!(err.to_string().contains("22050 Hz"), "{err}");
    }
}
