use std::collections::VecDeque;

use crate::error::Result;
use crate::event::Event;
use crate::onset::Onsets;
use crate::pitch::{PitchFrame, PitchTrack};
use crate::stage::{Mono, Stage};

/// The shortest note: a voiced run shorter than this is dropped as a fragment.
const SHORTEST_NOTE_SECS: f64 = 0.05;
/// How far, in semitones, the pitch must move from a note's median to count as a change.
/// Above vibrato's usual swing of up to half a semitone, below a semitone step.
const CHANGE_SEMITONES: f32 = 0.7;
/// How long a changed pitch must hold to start a new note without an onset.
const CHANGE_HOLD_SECS: f64 = 0.05;
/// The least mean voicing probability of a note's frames. Noise is often decoded voiced, as in
/// the reference pYIN, since an unvoiced frame's likelihood is spread across every pitch bin
/// while a voiced one's sits in a few; its voicing probability stays near the tracker's 0.01 for
/// a frame with no clear trough, far below a sounding note's.
const LEAST_CONFIDENCE: f32 = 0.5;
/// How soon after an onset voicing must begin for the note to start at the onset.
const ONSET_GRACE_SECS: f64 = 0.05;

/// Notes from a take: pitch tracking and onset detection, segmented into notes.
///
/// It runs [`PitchTrack`] and [`Onsets`] over the same input and emits both every
/// [`Event::Onset`], as it settles, and every [`Event::Note`]. A note is a run of voiced frames,
/// split at onsets and at pitch changes held for 50 ms (`CHANGE_HOLD_SECS`). Its `midi` is the
/// median of its frames' pitch, rounded to the nearest semitone, which is the declared reading of
/// vibrato and slides. Its `confidence` is its frames' mean voicing probability, and its `strength` the
/// strength of the onset it starts at: a note split off by a pitch change keeps the strength of
/// the note it came from, and a note with no onset at all has 0.
///
/// Every frame is on the input [`Mono`] timeline. A pitch frame is segmented only once every
/// onset up to it has settled, and a note is emitted once it has ended, which
/// [`latency_frames`](Stage::latency_frames) bounds.
pub struct Melody {
    pitch: PitchTrack,
    onsets: Onsets,
    segmenter: Segmenter,
    /// The stream's first frame and the frames received since, for the onset horizon.
    origin: Option<u64>,
    received: u64,
    /// Pitch frames settled but not yet segmented, waiting for the onsets up to them.
    waiting: VecDeque<PitchFrame>,
}

impl std::fmt::Debug for Melody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Melody")
            .field("pitch", &self.pitch)
            .field("onsets", &self.onsets)
            .finish_non_exhaustive()
    }
}

impl Melody {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32) -> Melody {
        let pitch = PitchTrack::new(rate);
        let segmenter = Segmenter::new(rate, pitch.hop());
        Melody {
            pitch,
            onsets: Onsets::new(rate),
            segmenter,
            origin: None,
            received: 0,
            waiting: VecDeque::new(),
        }
    }

    /// Passes every onset on to `out` and to the segmenter.
    fn take_onsets(&mut self, events: Vec<Event>, out: &mut Vec<Event>) {
        for event in events {
            if let Event::Onset { frame, strength } = event {
                self.segmenter.onset(frame, strength);
            }
            out.push(event);
        }
    }
}

impl Stage for Melody {
    type In = Mono;
    type Out = Event;

    fn push(&mut self, input: Mono, out: &mut Vec<Event>) -> Result<()> {
        let mut onsets = Vec::new();
        self.onsets.push(input.clone(), &mut onsets)?;
        let origin = *self.origin.get_or_insert(input.start_frame);
        self.received += input.samples.len() as u64;
        let mut frames = Vec::new();
        self.pitch.push(input, &mut frames)?;
        self.take_onsets(onsets, out);
        self.waiting.extend(frames);
        // Every onset at or before this frame has been emitted.
        let horizon = (origin + self.received).saturating_sub(self.onsets.latency_frames());
        while let Some(frame) = self.waiting.pop_front_if(|p| p.frame <= horizon) {
            self.segmenter.pitch(frame, out);
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<Event>) -> Result<()> {
        let mut onsets = Vec::new();
        self.onsets.finish(&mut onsets)?;
        self.take_onsets(onsets, out);
        let mut frames = Vec::new();
        self.pitch.finish(&mut frames)?;
        self.waiting.extend(frames);
        for frame in self.waiting.drain(..) {
            self.segmenter.pitch(frame, out);
        }
        self.segmenter.finish(out);
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        self.pitch
            .latency_frames()
            .max(self.onsets.latency_frames())
            + self.segmenter.latency()
    }
}

/// A note being built: where it starts, and its voiced frames so far.
struct Run {
    start: u64,
    strength: f32,
    /// The last voiced frame taken into the run.
    last: u64,
    /// Each frame's pitch, in fractional MIDI key numbers.
    pitches: Vec<f32>,
    voicing: f32,
}

impl Run {
    fn new(start: u64, strength: f32) -> Run {
        Run {
            start,
            strength,
            last: start,
            pitches: Vec::new(),
            voicing: 0.0,
        }
    }

    fn take(&mut self, frame: u64, pitch: f32, voiced: f32) {
        self.last = frame;
        self.pitches.push(pitch);
        self.voicing += voiced;
    }

    fn median(&self) -> f32 {
        let mut sorted = self.pitches.clone();
        sorted.sort_by(f32::total_cmp);
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 1 {
            sorted[mid]
        } else {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        }
    }
}

/// Splits settled pitch frames into notes, at onsets and at held pitch changes.
///
/// It must be given every onset up to a pitch frame before that frame.
struct Segmenter {
    hop: u64,
    shortest: u64,
    hold: usize,
    grace: u64,
    /// Onsets not yet reached by the pitch frames: `(frame, strength)`.
    onsets: VecDeque<(u64, f32)>,
    /// The latest onset reached while no note was sounding.
    pending: Option<(u64, f32)>,
    run: Option<Run>,
    /// Frames of the run that have moved off its pitch: `(frame, pitch, voicing)`.
    departed: Vec<(u64, f32, f32)>,
}

impl Segmenter {
    fn new(rate: u32, hop: u64) -> Segmenter {
        let frames = |secs: f64| (secs * f64::from(rate)).round() as u64;
        Segmenter {
            hop,
            shortest: frames(SHORTEST_NOTE_SECS),
            hold: (frames(CHANGE_HOLD_SECS) as f64 / hop as f64).ceil() as usize,
            grace: frames(ONSET_GRACE_SECS),
            onsets: VecDeque::new(),
            pending: None,
            run: None,
            departed: Vec::new(),
        }
    }

    /// The most frames after a note's end before it is emitted, once its pitch frames arrive: a
    /// held change is confirmed `hold` frames after the note ends, and an end at an onset or at
    /// unvoicing is seen within a frame of it.
    fn latency(&self) -> u64 {
        (self.hold as u64 + 1) * self.hop
    }

    fn onset(&mut self, frame: u64, strength: f32) {
        self.onsets.push_back((frame, strength));
    }

    fn pitch(&mut self, p: PitchFrame, out: &mut Vec<Event>) {
        while let Some((at, strength)) = self.onsets.pop_front_if(|o| o.0 <= p.frame) {
            self.departed.clear();
            if let Some(run) = self.run.take() {
                emit(run, at, self.shortest, out);
            }
            self.pending = Some((at, strength));
        }
        let Some(hz) = p.hz else {
            self.close(out);
            return;
        };
        let pitch = 69.0 + 12.0 * (hz / 440.0).log2();
        let Some(run) = &mut self.run else {
            let (start, strength) = match self.pending.take() {
                Some((at, strength)) if p.frame - at <= self.grace => (at, strength),
                _ => (p.frame.saturating_sub(self.hop / 2), 0.0),
            };
            let mut run = Run::new(start, strength);
            run.take(p.frame, pitch, p.voiced);
            self.run = Some(run);
            return;
        };
        if (pitch - run.median()).abs() <= CHANGE_SEMITONES {
            // Back on pitch: a short excursion belongs to the note.
            for (frame, pitch, voiced) in self.departed.drain(..) {
                run.take(frame, pitch, voiced);
            }
            run.take(p.frame, pitch, p.voiced);
            return;
        }
        self.departed.push((p.frame, pitch, p.voiced));
        if self.departed.len() >= self.hold {
            let at = self.departed[0].0.saturating_sub(self.hop / 2);
            let mut next = Run::new(at, run.strength);
            for (frame, pitch, voiced) in self.departed.drain(..) {
                next.take(frame, pitch, voiced);
            }
            if let Some(run) = self.run.replace(next) {
                emit(run, at, self.shortest, out);
            }
        }
    }

    /// Ends the sounding note after its last voiced frame.
    fn close(&mut self, out: &mut Vec<Event>) {
        if let Some(mut run) = self.run.take() {
            for (frame, pitch, voiced) in self.departed.drain(..) {
                run.take(frame, pitch, voiced);
            }
            let end = run.last + self.hop / 2;
            emit(run, end, self.shortest, out);
        }
    }

    fn finish(&mut self, out: &mut Vec<Event>) {
        self.close(out);
    }
}

/// Emits `run` as a note ending at `end`, unless it is shorter than `shortest` or too weakly
/// voiced to be a note.
fn emit(run: Run, end: u64, shortest: u64, out: &mut Vec<Event>) {
    if run.pitches.is_empty() || end < run.start + shortest {
        return;
    }
    let confidence = run.voicing / run.pitches.len() as f32;
    if confidence < LEAST_CONFIDENCE {
        return;
    }
    out.push(Event::Note {
        start: run.start,
        end,
        midi: run.median().round().clamp(0.0, 127.0) as u8,
        confidence,
        strength: run.strength,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    const RATE: u32 = 22_050;

    fn run(signal: &[f32], chunk: usize) -> Vec<Event> {
        let mut stage = Melody::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(chunk).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: (i * chunk) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        out
    }

    /// `(start, end, midi)` of each note, in seconds.
    fn notes(events: &[Event]) -> Vec<(f64, f64, u8)> {
        let secs = |frame: u64| frame as f64 / f64::from(RATE);
        events
            .iter()
            .filter_map(|e| match *e {
                Event::Note {
                    start, end, midi, ..
                } => Some((secs(start), secs(end), midi)),
                _ => None,
            })
            .collect()
    }

    fn padded(signal: Vec<f32>) -> Vec<f32> {
        let mut out = synth::silence(RATE, 0.2);
        out.extend(signal);
        out.extend(synth::silence(RATE, 0.3));
        out
    }

    const SCALE: [f64; 5] = [261.63, 293.66, 329.63, 349.23, 392.0];

    fn scale() -> Vec<f32> {
        let notes: Vec<(f64, f64)> = SCALE.iter().map(|&f| (f, 0.25)).collect();
        padded(synth::note_sequence(RATE, &notes, 4, 0.5))
    }

    #[test]
    fn a_held_tone_is_one_note_at_its_pitch() {
        let found = notes(&run(&padded(synth::tone(RATE, 261.63, 0.5, 4, 0.5)), 1000));
        assert_eq!(found.len(), 1, "{found:?}");
        let (start, end, midi) = found[0];
        assert_eq!(midi, 60);
        assert!((start - 0.2).abs() <= 0.02, "{found:?}");
        assert!((end - 0.7).abs() <= 0.03, "{found:?}");
    }

    #[test]
    fn a_scale_gives_its_notes_in_order() {
        let found = notes(&run(&scale(), 1000));
        let midis: Vec<u8> = found.iter().map(|n| n.2).collect();
        assert_eq!(midis, vec![60, 62, 64, 65, 67], "{found:?}");
        for (i, &(start, _, _)) in found.iter().enumerate() {
            let wanted = 0.2 + 0.25 * i as f64;
            assert!((start - wanted).abs() <= 0.02, "note {i}: {found:?}");
        }
    }

    #[test]
    fn vibrato_reads_as_its_median() {
        let signal = padded(synth::vibrato(RATE, 220.0, 1.0, 40.0, 6.0, 4, 0.5));
        let found = notes(&run(&signal, 1000));
        let midis: Vec<u8> = found.iter().map(|n| n.2).collect();
        assert_eq!(midis, vec![57], "{found:?}");
    }

    #[test]
    fn a_repeated_pitch_splits_at_its_onsets() {
        let signal = padded(synth::note_sequence(RATE, &[(220.0, 0.3); 3], 4, 0.5));
        let found = notes(&run(&signal, 1000));
        let midis: Vec<u8> = found.iter().map(|n| n.2).collect();
        assert_eq!(midis, vec![57, 57, 57], "{found:?}");
        for (i, &(start, _, _)) in found.iter().enumerate() {
            let wanted = 0.2 + 0.3 * i as f64;
            assert!((start - wanted).abs() <= 0.02, "note {i}: {found:?}");
        }
    }

    #[test]
    fn unvoiced_noise_gives_no_notes() {
        let signal = synth::noise_bursts(RATE, 2.0, &[(0.2, 1.5)], 100.0, 8000.0, 0.5);
        let events = run(&signal, 1000);
        assert_eq!(notes(&events), vec![], "{events:?}");
    }

    #[test]
    fn onsets_pass_through() {
        let events = run(&scale(), 1000);
        let onsets = events
            .iter()
            .filter(|e| matches!(e, Event::Onset { .. }))
            .count();
        assert!(onsets >= SCALE.len(), "{events:?}");
    }

    #[test]
    fn notes_settle_within_the_reported_latency() {
        let mut signal = scale();
        signal.extend(synth::silence(RATE, 1.0));
        let mut stage = Melody::new(RATE);
        let mut out = Vec::new();
        let mut fed = 0u64;
        let chunk = 64;
        for piece in signal.chunks(chunk) {
            let mono = Mono {
                rate: RATE,
                start_frame: fed,
                samples: piece.to_vec(),
            };
            fed += piece.len() as u64;
            let before = out.len();
            stage.push(mono, &mut out).unwrap();
            for event in &out[before..] {
                if let Event::Note { end, .. } = *event {
                    assert!(
                        fed <= end + stage.latency_frames() + chunk as u64,
                        "a note ending at {end} settled at {fed}"
                    );
                }
            }
        }
        // A second of silence follows the scale, so every note settles before the stream ends.
        assert_eq!(notes(&out).len(), SCALE.len(), "{out:?}");
    }

    /// Feeds the segmenter frames one hop apart from frame 0, each `(midi, frames)` a stretch at
    /// that pitch, fully voiced; `None` is unvoiced.
    fn segment(stretches: &[(Option<f32>, usize)]) -> Vec<(u64, u64, u8)> {
        let hop = 256;
        let mut segmenter = Segmenter::new(RATE, hop);
        let mut out = Vec::new();
        let mut frame = 0;
        for &(midi, count) in stretches {
            for _ in 0..count {
                let hz = midi.map(|m| 440.0 * ((m - 69.0) / 12.0).exp2());
                let voiced = if hz.is_some() { 1.0 } else { 0.0 };
                segmenter.pitch(PitchFrame { frame, hz, voiced }, &mut out);
                frame += hop;
            }
        }
        segmenter.finish(&mut out);
        out.iter()
            .filter_map(|e| match *e {
                Event::Note {
                    start, end, midi, ..
                } => Some((start, end, midi)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_held_pitch_change_splits_without_an_onset() {
        let found = segment(&[(Some(60.0), 30), (Some(62.0), 30), (None, 5)]);
        let midis: Vec<u8> = found.iter().map(|n| n.2).collect();
        assert_eq!(midis, vec![60, 62], "{found:?}");
        // The second note starts where its pitch did, half a hop before its first frame.
        assert_eq!(found[1].0, 30 * 256 - 128);
        assert_eq!(found[0].1, found[1].0);
    }

    #[test]
    fn a_brief_excursion_stays_in_the_note() {
        let found = segment(&[
            (Some(60.0), 20),
            (Some(62.0), 3),
            (Some(60.0), 20),
            (None, 5),
        ]);
        assert_eq!(found.iter().map(|n| n.2).collect::<Vec<_>>(), vec![60]);
    }

    #[test]
    fn a_run_shorter_than_the_shortest_note_is_dropped() {
        let found = segment(&[(None, 5), (Some(60.0), 3), (None, 5)]);
        assert_eq!(found, vec![]);
    }
}
