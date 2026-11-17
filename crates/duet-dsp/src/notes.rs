use std::collections::VecDeque;

use crate::error::Result;
use crate::event::Event;
use crate::performance::{Layer1, Performance};
use crate::pitch::PitchFrame;
use crate::stage::{Mono, Stage};

/// The shortest note: a voiced run shorter than this is dropped as a fragment.
pub(crate) const SHORTEST_NOTE_SECS: f64 = 0.05;
/// How far, in semitones, the pitch must move from where a note is held to count as leaving it.
/// Above vibrato's usual swing of up to half a semitone, below a semitone step.
pub(crate) const CHANGE_SEMITONES: f32 = 0.7;
/// How widely, in semitones, a slide's frames over [`CHANGE_HOLD_SECS`] may spread for its
/// pitch to have arrived. A vibrato of 40 cents either side at 7 a second spreads about 0.2
/// over that span at each turn of its swing; a glide of 7.5 semitones a second spreads this
/// much, and the turn of a V-shaped dip of about 16 a second does, so anything moving faster is
/// still on its way.
pub(crate) const ARRIVED_SEMITONES: f32 = 0.35;
/// How long the pitch must be away from a note to end it without an onset, and how long a new
/// pitch must hold to start the next.
pub(crate) const CHANGE_HOLD_SECS: f64 = 0.05;
/// How far back where a note is held is measured: the mean of its latest frames over this span,
/// so it follows a slow drift. A mean over a whole swing of a vibrato cancels it; over 0.2 s, a
/// vibrato of 4.5 to 7 a second is cancelled to within about a quarter of its swing, where a
/// median of the same frames reads it off centre by up to half its swing.
const HELD_SECS: f64 = 0.2;
/// The least mean voicing probability of a note's frames.
///
/// Noise is often decoded voiced, as in the reference pYIN, since an unvoiced frame's likelihood
/// is spread across every pitch bin while a voiced one's sits in a few. Its voicing probability
/// stays near the tracker's 0.01 for a frame with no clear trough. A frame whose one clear trough
/// is `d` deep has a voicing probability of the Beta(2, 18) prior's mass above `d`: 0.15 is a
/// trough about 0.17 deep. Breathy or reverberant voice, with troughs of 0.10 to 0.15, reads
/// about 0.43 to 0.21 and is kept. Task 9 retunes this on real takes.
pub(crate) const LEAST_CONFIDENCE: f32 = 0.15;
/// How soon after an onset voicing must begin for the note to start at the onset.
pub(crate) const ONSET_GRACE_SECS: f64 = 0.05;
/// How long before voicing begins an onset may be and still give the note its strength: an "s",
/// "sh" or "f" that opens a word, or a pick's noise, comes before the vowel or the pitch. The
/// note still starts at the voicing, unless the onset is within [`ONSET_GRACE_SECS`].
pub(crate) const STRENGTH_WINDOW_SECS: f64 = 0.2;
/// Strength per square root of a note's level, for a note no onset gives a strength to.
///
/// Fitted to the onset strength a four-partial tone has when it is attacked from silence: it
/// grows as about the square root of the level, and `0.06 * sqrt(rms)` is within 10% of it from
/// peak 0.05 to 0.5. A sine's attack reads about half this, and an eight-partial tone's about
/// half again more, since onset strength also counts how many bins rise. Task 9 retunes it on
/// real takes.
const STRENGTH_PER_ROOT_LEVEL: f32 = 0.06;

/// Notes from a take: its [`Performance`] (pitch tracking and onset detection), segmented into
/// notes.
///
/// It runs [`PitchTrack`](crate::PitchTrack) and [`Onsets`](crate::Onsets) over the same input,
/// as [`Performance`] does, and emits both every
/// [`Event::Onset`], as it settles, and every [`Event::Note`]. A note is held pitch in a run of
/// voiced frames, split at onsets and where the pitch moves away and holds somewhere new; pitch
/// still moving between two notes is a slide, never a note of its own. Its `midi` is the median
/// of its held frames' pitch, rounded to the nearest semitone, which is the declared reading of
/// vibrato and drift. Its `confidence` is its frames' mean voicing probability, and its
/// `strength` the strength of the latest onset since the previous note ended, within 0.2 s
/// (`STRENGTH_WINDOW_SECS`) before its voicing begins. A note split off by a pitch change keeps
/// the strength of the note it came from. A note with no such onset takes its strength from its
/// own level, so a note's strength is never 0.
///
/// Every frame is on the input [`Mono`] timeline. A pitch frame is segmented only once every
/// onset up to it has settled, and a note is emitted once it has ended, which
/// [`latency_frames`](Stage::latency_frames) bounds. The onsets and the notes do not depend on how
/// the stream is cut into chunks, but how they interleave in `out` does.
///
/// [`push_keeping`](Melody::push_keeping) and [`finish_keeping`](Melody::finish_keeping) also
/// keep the performance the notes are segmented from: its head and frame blocks, exactly as
/// [`Performance`] emits them. Notes are segmented from each frame's
/// [`hz`](PitchFrame::hz), the performance is written from its
/// [`fine_hz`](PitchFrame::fine_hz).
pub struct Melody {
    performance: Performance,
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
            .field("performance", &self.performance)
            .finish_non_exhaustive()
    }
}

impl Melody {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32) -> Melody {
        let performance = Performance::new(rate);
        let segmenter = Segmenter::new(rate, performance.hop());
        Melody {
            performance,
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

impl Melody {
    /// [`Stage::push`], keeping the performance: its head and every block of frames that
    /// settles go to `performance` (never [`Layer1::Onset`]: the onsets are `out`'s
    /// [`Event::Onset`]s).
    ///
    /// # Errors
    ///
    /// As [`Stage::push`].
    pub fn push_keeping(
        &mut self,
        input: Mono,
        out: &mut Vec<Event>,
        performance: &mut Vec<Layer1>,
    ) -> Result<()> {
        self.push_inner(input, out, Some(performance))
    }

    /// [`Stage::finish`], keeping the rest of the performance, as
    /// [`push_keeping`](Melody::push_keeping) does.
    ///
    /// # Errors
    ///
    /// As [`Stage::finish`].
    pub fn finish_keeping(
        &mut self,
        out: &mut Vec<Event>,
        performance: &mut Vec<Layer1>,
    ) -> Result<()> {
        self.finish_inner(out, Some(performance))
    }

    fn push_inner(
        &mut self,
        input: Mono,
        out: &mut Vec<Event>,
        performance: Option<&mut Vec<Layer1>>,
    ) -> Result<()> {
        let origin = *self.origin.get_or_insert(input.start_frame);
        let samples = input.samples.len() as u64;
        let (mut frames, mut onsets) = (Vec::new(), Vec::new());
        self.performance.push_raw(input, &mut frames, &mut onsets)?;
        self.received += samples;
        if let Some(kept) = performance {
            self.performance.keep(&frames, kept);
        }
        self.take_onsets(onsets, out);
        self.waiting.extend(frames);
        // Every onset at or before this frame has been emitted.
        let horizon = (origin + self.received).saturating_sub(self.performance.onset_latency());
        while let Some(frame) = self.waiting.pop_front_if(|p| p.frame <= horizon) {
            self.segmenter.pitch(frame, out);
        }
        Ok(())
    }

    fn finish_inner(
        &mut self,
        out: &mut Vec<Event>,
        performance: Option<&mut Vec<Layer1>>,
    ) -> Result<()> {
        let (mut frames, mut onsets) = (Vec::new(), Vec::new());
        self.performance.finish_raw(&mut frames, &mut onsets)?;
        if let Some(kept) = performance {
            self.performance.keep(&frames, kept);
            self.performance.keep_last(kept);
        }
        self.take_onsets(onsets, out);
        self.waiting.extend(frames);
        for frame in self.waiting.drain(..) {
            self.segmenter.pitch(frame, out);
        }
        self.segmenter.finish(out);
        Ok(())
    }
}

impl Stage for Melody {
    type In = Mono;
    type Out = Event;

    fn push(&mut self, input: Mono, out: &mut Vec<Event>) -> Result<()> {
        self.push_inner(input, out, None)
    }

    fn finish(&mut self, out: &mut Vec<Event>) -> Result<()> {
        self.finish_inner(out, None)
    }

    fn latency_frames(&self) -> u64 {
        self.performance
            .pitch_latency()
            .max(self.performance.onset_latency())
            + self.segmenter.latency()
    }
}

/// A note being built: where it starts, and its voiced frames so far.
struct Run {
    start: u64,
    /// The onset it was attributed to, kept so that a run dropped as a fragment hands it on.
    onset: Option<(u64, f32)>,
    /// Its strength, when it has one from an onset or from the note it was split from; otherwise
    /// it is taken from its level.
    strength: Option<f32>,
    /// The last voiced frame taken into the run.
    last: u64,
    /// The first frame taken on pitch: where the slide or scoop into the note ends and its held
    /// pitch begins.
    held_from: Option<u64>,
    /// Each frame's pitch, in fractional MIDI key numbers.
    pitches: Vec<f32>,
    /// Where the note is held: the mean of its latest `held` pitches.
    reference: f32,
    /// Where the note was first held: the mean of its first `held` pitches, once it has
    /// them. The reference follows a slow glide; this does not, so a glide far from it has
    /// left the note.
    anchor: Option<f32>,
    held: usize,
    /// Whether its pitch has ever held: stayed within [`CHANGE_SEMITONES`] for `hold` frames,
    /// as vibrato and a tracker's jitter do. A run that leaves before it has held was a scoop,
    /// not a note.
    has_held: bool,
    hold: usize,
    /// Every frame taken, on pitch or off it: what its voicing and level are averaged over.
    frames: usize,
    voicing: f32,
    level: f32,
}

impl Run {
    fn new(
        start: u64,
        onset: Option<(u64, f32)>,
        strength: Option<f32>,
        (held, hold): (usize, usize),
    ) -> Run {
        Run {
            start,
            onset,
            strength,
            last: start,
            held_from: None,
            pitches: Vec::new(),
            reference: 0.0,
            anchor: None,
            held,
            has_held: false,
            hold,
            frames: 0,
            voicing: 0.0,
            level: 0.0,
        }
    }

    fn take(&mut self, frame: u64, pitch: f32, voiced: f32, level: f32) {
        self.take_off_pitch(frame, voiced, level);
        self.held_from.get_or_insert(frame);
        self.pitches.push(pitch);
        if !self.has_held && self.pitches.len() >= self.hold {
            let recent = &self.pitches[self.pitches.len() - self.hold..];
            self.has_held = spread(recent) <= CHANGE_SEMITONES;
        }
        let from = self.pitches.len().saturating_sub(self.held);
        let recent = &self.pitches[from..];
        self.reference = recent.iter().sum::<f32>() / recent.len() as f32;
        if self.pitches.len() == self.held {
            self.anchor = Some(self.reference);
        }
    }

    /// Takes a frame that belongs to the note's time but not to its pitch: a short excursion
    /// that came back, which must not move where the note is held.
    fn take_off_pitch(&mut self, frame: u64, voiced: f32, level: f32) {
        self.last = frame;
        self.frames += 1;
        self.voicing += voiced;
        self.level += level;
    }

    /// Whether the pitch, following the note slowly, has moved far from where it was held.
    fn drifted_away(&self) -> bool {
        self.anchor
            .is_some_and(|anchor| (self.reference - anchor).abs() > CHANGE_SEMITONES)
    }

    fn median(&self) -> f32 {
        median(&self.pitches)
    }

    fn strength(&self) -> f32 {
        self.strength
            .unwrap_or_else(|| level_strength(self.level / self.frames.max(1) as f32))
    }
}

/// How widely `pitches` spread, in semitones.
fn spread(pitches: &[f32]) -> f32 {
    let (low, high) = pitches
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), &p| {
            (low.min(p), high.max(p))
        });
    high - low
}

pub(crate) fn median(pitches: &[f32]) -> f32 {
    let mut sorted = pitches.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// A note's strength from its mean level (root mean square), on the scale of onset strength
/// ([`STRENGTH_PER_ROOT_LEVEL`]), clamped to `0..=1` and never 0. Louder is stronger.
pub(crate) fn level_strength(level: f32) -> f32 {
    (STRENGTH_PER_ROOT_LEVEL * level.sqrt()).clamp(f32::MIN_POSITIVE, 1.0)
}

/// A frame of a run: `(frame, pitch, voicing, level)`.
type Taken = (u64, f32, f32, f32);

/// A note as the segmenter finds it, before it is given a key: its time, where its held pitch
/// begins, and the median of its held frames' pitch in fractional key numbers, on whatever
/// reference the pitch was given in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub start: u64,
    pub end: u64,
    pub held_from: u64,
    pub median: f32,
    pub confidence: f32,
    pub strength: f32,
}

impl Segment {
    /// The note as [`Melody`] emits it: its key the median rounded to the nearest.
    fn event(&self) -> Event {
        Event::Note {
            start: self.start,
            end: self.end,
            midi: self.median.round().clamp(0.0, 127.0) as u8,
            confidence: self.confidence,
            strength: self.strength,
        }
    }
}

/// Pitch on its way from one note to the next: it has left where the note was held, and has
/// not yet arrived anywhere and stayed.
struct Slide {
    /// Where the next note starts, if the slide arrives: where the slide began.
    start: u64,
    /// The strength the next note inherits from the one the slide left.
    strength: Option<f32>,
    /// The onset of a fragment the slide left, handed on if the slide never arrives.
    onset: Option<(u64, f32)>,
    frames: Vec<Taken>,
}

/// Splits settled pitch frames into notes, at onsets and at held pitch changes.
///
/// A note is held pitch. When the pitch leaves where a note is held, by more than
/// [`CHANGE_SEMITONES`] for [`CHANGE_HOLD_SECS`], the note ends where it left, and the pitch is
/// a slide until it holds again: until its frames over [`CHANGE_HOLD_SECS`] spread no wider
/// than [`ARRIVED_SEMITONES`]. The next note then starts where the slide began, so a slide into
/// a note is part of the note it arrives at, but only the held frames give the note its pitch.
/// A slide is never a note of its own: one that ends at an onset or in silence before it holds
/// is dropped. A voice that comes in moving, after silence or an onset, has not held either:
/// until its frames over [`CHANGE_HOLD_SECS`] spread no wider than [`CHANGE_SEMITONES`], as a
/// vibrato's or a tracker's jitter does, it is a scoop into the note it reaches, which starts
/// where the voice came in. A scoop that leaves where its pitch has been so far, as pitch
/// leaves a note, before it has held is a slide from then on, so it must arrive as a slide
/// does, within [`ARRIVED_SEMITONES`]. A pitch that dips
/// away and comes back to where it was is a slide too, so it
/// splits one pitch into two notes, as a voiced consonant between two syllables does. The
/// reading's same-key rejoin ([`rejoin`](crate::rejoin)) then reads such a split as one note
/// where no onset marks it and the dip stays within a whole tone.
///
/// Where a note is held is the mean of its latest [`HELD_SECS`], so vibrato and a slow drift
/// stay in it. A glide slow enough to be followed that way has still left the note once that
/// mean is more than [`CHANGE_SEMITONES`] from where the note was first held, so two notes a
/// glide joins are never read as one between them.
///
/// Stated limits:
/// - A glide slower than about 7.5 semitones a second spreads no wider than
///   [`ARRIVED_SEMITONES`] over [`CHANGE_HOLD_SECS`], so it arrives on its way and can still
///   give a note every semitone or two it passes, sometimes two at its last pitch.
/// - The turn of a dip slower than about 16 semitones a second can arrive in the same way and
///   be a note.
/// - A vibrato of a full half semitone either side at 7 swings a second, on a note that also
///   drifts half a semitone, can leave it at some phases of its swing and split it in two.
/// - The bottom of a scoop that wobbles in place for [`CHANGE_HOLD_SECS`] or more, within
///   [`CHANGE_SEMITONES`], has held, so it is a note of its own, as a short note is.
/// - An onset ends whatever note is sounding, so a scoop an onset cuts can be a note.
/// - A held approach, or a turn inside a note, that holds for [`CHANGE_HOLD_SECS`] on another
///   key is a note of its own: a G#3 turning up to A#3 for 0.14 s reads G#3, A#3, G#3, which
///   is no same-key pair, so the same-key rejoin leaves it. The note HMM takes either in.
/// - A slide that keeps wobbling, so that its frames over every [`CHANGE_HOLD_SECS`] spread
///   wider than [`ARRIVED_SEMITONES`], never arrives, and is dropped as no note when the voice
///   stops, however long it is sung. A glide or a scoop into a fast or uneven wobble does this:
///   an even one of 50 cents either side at 9 swings a second, or 60 at 8. An even vibrato
///   at 7 a second still arrives at the turns of its swing, even at 60 cents either side. The
///   note HMM ([`NoteModel`](crate::NoteModel)) reads such a note.
///
/// It must be given every onset up to a pitch frame before that frame.
pub(crate) struct Segmenter {
    hop: u64,
    shortest: u64,
    hold: usize,
    grace: u64,
    window: u64,
    /// Onsets not yet reached by the pitch frames: `(frame, strength)`.
    onsets: VecDeque<(u64, f32)>,
    /// The latest onset since the previous note ended, not yet given to a note.
    pending: Option<(u64, f32)>,
    run: Option<Run>,
    /// Frames of the run that have moved off its pitch.
    departed: Vec<Taken>,
    /// The pitch between the note it left and the next, when it is moving.
    slide: Option<Slide>,
    /// How many of a note's latest frames say where it is held.
    held: usize,
}

impl Segmenter {
    pub(crate) fn new(rate: u32, hop: u64) -> Segmenter {
        let frames = |secs: f64| (secs * f64::from(rate)).round() as u64;
        Segmenter {
            hop,
            shortest: frames(SHORTEST_NOTE_SECS),
            hold: (frames(CHANGE_HOLD_SECS) as f64 / hop as f64).ceil() as usize,
            grace: frames(ONSET_GRACE_SECS),
            window: frames(STRENGTH_WINDOW_SECS),
            onsets: VecDeque::new(),
            pending: None,
            run: None,
            departed: Vec::new(),
            slide: None,
            held: (frames(HELD_SECS) as f64 / hop as f64).ceil() as usize,
        }
    }

    /// The most frames after a note's end before it is emitted, once its pitch frames arrive: a
    /// held change is confirmed `hold` frames after the note ends, and an end at an onset or at
    /// unvoicing is seen within a frame of it.
    fn latency(&self) -> u64 {
        (self.hold as u64 + 1) * self.hop
    }

    pub(crate) fn onset(&mut self, frame: u64, strength: f32) {
        self.onsets.push_back((frame, strength));
    }

    /// Takes a pitch frame, its pitch in fractional key numbers on A440 from its
    /// [`hz`](PitchFrame::hz), and emits every note it ends as an [`Event::Note`].
    fn pitch(&mut self, p: PitchFrame, out: &mut Vec<Event>) {
        let mut segments = Vec::new();
        let pitch = p.hz.map(|hz| 69.0 + 12.0 * (hz / 440.0).log2());
        self.take_frame(p.frame, pitch, p.voiced, p.level, &mut segments);
        out.extend(segments.iter().map(Segment::event));
    }

    /// Takes one frame: `pitch` in fractional key numbers on any one reference, or `None` when
    /// unvoiced. Every note it ends goes to `out`.
    pub(crate) fn take_frame(
        &mut self,
        frame: u64,
        pitch: Option<f32>,
        voiced: f32,
        level: f32,
        out: &mut Vec<Segment>,
    ) {
        let p = (frame, voiced, level);
        while let Some(onset) = self.onsets.pop_front_if(|o| o.0 <= p.0) {
            self.departed.clear();
            self.slide = None;
            if let Some(run) = self.run.take() {
                self.emit(run, onset.0, out);
            }
            self.pending = Some(onset);
        }
        let Some(pitch) = pitch else {
            self.close(out);
            return;
        };
        let (frame, voiced, level) = p;
        let taken = (frame, pitch, voiced, level);
        if self.slide.is_some() {
            self.slide_on(taken);
            return;
        }
        let Some(run) = &mut self.run else {
            let onset = self.pending.take();
            let since = |at: u64| frame - at;
            let start = match onset {
                Some((at, _)) if since(at) <= self.grace => at,
                _ => frame.saturating_sub(self.hop / 2),
            };
            let onset = onset.filter(|&(at, _)| since(at) <= self.window);
            let mut run = Run::new(start, onset, onset.map(|o| o.1), (self.held, self.hold));
            take(&mut run, taken);
            self.run = Some(run);
            return;
        };
        if (taken.1 - run.reference).abs() <= CHANGE_SEMITONES {
            // Back on pitch: a short excursion belongs to the note's time, not its pitch.
            for (frame, _, voiced, level) in self.departed.drain(..) {
                run.take_off_pitch(frame, voiced, level);
            }
            take(run, taken);
            if run.drifted_away() {
                // A glide slow enough to follow has still arrived somewhere new.
                self.leave(taken.0.saturating_sub(self.hop / 2), Vec::new(), out);
            }
            return;
        }
        self.departed.push(taken);
        if self.departed.len() >= self.hold {
            let at = self.departed[0].0.saturating_sub(self.hop / 2);
            let departed = std::mem::take(&mut self.departed);
            self.leave(at, departed, out);
        }
    }

    /// Ends the sounding note at `at`, where the pitch left it, and starts a slide there with
    /// the frames that have left it.
    fn leave(&mut self, at: u64, departed: Vec<Taken>, out: &mut Vec<Segment>) {
        let Some(run) = self.run.take() else {
            return;
        };
        if !run.has_held {
            // It never held: a scoop into the note it reaches, which starts where it did.
            self.slide = Some(Slide {
                start: run.start,
                strength: run.strength,
                onset: run.onset,
                frames: Vec::new(),
            });
        } else {
            let (strength, inherited, onset) = (run.strength(), run.strength, run.onset);
            let emitted = self.emit(run, at, out);
            // A note split off by a pitch change keeps the strength of the note before it; one
            // split off a dropped fragment is as if the fragment had not been.
            self.slide = Some(Slide {
                start: at,
                strength: if emitted { Some(strength) } else { inherited },
                onset: if emitted { None } else { onset },
                frames: Vec::new(),
            });
        }
        // A run can start only once `hold` frames have arrived, so at the last of these.
        for frame in departed {
            self.slide_on(frame);
        }
    }

    /// Takes a frame into the slide, and starts the next note once the pitch holds.
    fn slide_on(&mut self, taken: Taken) {
        let Some(slide) = &mut self.slide else {
            return;
        };
        slide.frames.push(taken);
        let recent = &slide.frames[slide.frames.len().saturating_sub(self.hold)..];
        let pitches: Vec<f32> = recent.iter().map(|f| f.1).collect();
        if recent.len() == self.hold && spread(&pitches) <= ARRIVED_SEMITONES {
            // Arrived and holding: the next note, from where the slide began, pitched only by
            // the frames that hold.
            let mut next = Run::new(slide.start, None, slide.strength, (self.held, self.hold));
            for &frame in recent {
                take(&mut next, frame);
            }
            self.slide = None;
            self.run = Some(next);
        }
    }

    /// Ends the sounding note after its last voiced frame. A slide that has not arrived is no
    /// note.
    fn close(&mut self, out: &mut Vec<Segment>) {
        if let Some(slide) = self.slide.take()
            && self.pending.is_none()
        {
            // The slide was no note, nor the fragment before it, so its onset is handed on.
            self.pending = slide.onset;
        }
        if let Some(mut run) = self.run.take() {
            for frame in self.departed.drain(..) {
                take(&mut run, frame);
            }
            let end = run.last + self.hop / 2;
            let onset = run.onset;
            // A dropped run was not a note, so its onset is handed on to the next.
            if !self.emit(run, end, out) && self.pending.is_none() {
                self.pending = onset;
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<Event>) {
        let mut segments = Vec::new();
        self.close(&mut segments);
        out.extend(segments.iter().map(Segment::event));
    }

    /// Ends the stream, emitting the note still sounding to `out`.
    pub(crate) fn finish_segments(&mut self, out: &mut Vec<Segment>) {
        self.close(out);
    }

    /// Emits `run` as a note ending at `end`, unless it is shorter than the shortest note or too
    /// weakly voiced to be one. Returns whether it was emitted.
    fn emit(&self, run: Run, end: u64, out: &mut Vec<Segment>) -> bool {
        let confidence = run.voicing / run.frames.max(1) as f32;
        if run.pitches.is_empty()
            || end < run.start + self.shortest
            || confidence < LEAST_CONFIDENCE
        {
            return false;
        }
        out.push(Segment {
            start: run.start,
            end,
            held_from: run.held_from.unwrap_or(run.start),
            median: run.median(),
            confidence,
            strength: run.strength(),
        });
        true
    }
}

fn take(run: &mut Run, (frame, pitch, voiced, level): Taken) {
    run.take(frame, pitch, voiced, level);
}

/// Frames for the stated limit of a slide that never arrives, one hop apart: a held note,
/// a weakly voiced glide straight down from it, a scoop up to the note it is sung to, and a
/// weakly voiced wobble of 60 cents either side at 9 swings a second on that note, then
/// silence. Each frame is its pitch in semitones from the note the wobble is on, `None` when
/// unvoiced, and its voicing. The note HMM's tests read the same frames.
#[cfg(test)]
pub(crate) fn wobbling_slide() -> Vec<(Option<f32>, f32)> {
    let hop_secs = 256.0 / 22_050.0;
    let mut frames = vec![(None, 0.0); 20];
    // Held three semitones above, then gliding down through barely voiced frames.
    frames.extend([(Some(3.0), 0.9); 25]);
    frames.extend((1..=6).map(|k| (Some(3.0 - 7.5 * k as f32 / 6.0), 0.1)));
    // The scoop up from 4.5 below, then the wobble, its voicing dipping mid-note.
    frames.extend((1..=8).map(|k| (Some(-4.5 + 4.5 * k as f32 / 8.0), 0.6)));
    frames.extend((0..30).map(|k| {
        let wobble = 0.6 * (std::f32::consts::TAU * 9.0 * k as f32 * hop_secs).sin();
        (Some(wobble), if (10..16).contains(&k) { 0.3 } else { 0.75 })
    }));
    frames.extend([(None, 0.0); 20]);
    frames
}

/// `frames` frames moving in a straight line from `from` to `to`, `to` included, in MIDI keys.
#[cfg(test)]
pub(crate) fn line(from: f32, to: f32, frames: usize) -> Vec<Option<f32>> {
    (1..=frames)
        .map(|k| Some(from + (to - from) * k as f32 / frames as f32))
        .collect()
}

/// `frames` frames held on `midi`.
#[cfg(test)]
pub(crate) fn held(midi: f32, frames: usize) -> Vec<Option<f32>> {
    vec![Some(midi); frames]
}

/// The first #28 dip witness, one frame a hop: a note held on key 60 falls 8 semitones and
/// comes straight back, within 0.2 s, with no onset and no silence, as a voiced consonant
/// between two syllables on one pitch. The same-key rejoin's tests read the same frames.
#[cfg(test)]
pub(crate) fn dip_between_syllables() -> Vec<Option<f32>> {
    [
        held(60.0, 25),
        line(60.0, 52.0, 9),
        line(52.0, 60.0, 8),
        held(60.0, 25),
    ]
    .concat()
}

/// The second #28 dip witness: 3 semitones down and back over 0.23 s, slow enough that the
/// frames at its turn sit within [`CHANGE_SEMITONES`] of one another, but never still.
#[cfg(test)]
pub(crate) fn slower_dip() -> Vec<Option<f32>> {
    [
        held(60.0, 25),
        line(60.0, 57.0, 10),
        line(57.0, 60.0, 10),
        held(60.0, 25),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PitchTrack;
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
        // The burst was heard, as an onset, and still made no note.
        assert!(
            events.iter().any(|e| matches!(e, Event::Onset { .. })),
            "{events:?}"
        );
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
        // Notes that end at onsets, and notes that end at held pitch changes.
        for signal in [scale(), legato_scale()] {
            assert_settles_within_latency(signal);
        }
    }

    fn assert_settles_within_latency(mut signal: Vec<f32>) {
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
        // A second of silence follows, so every note settles before the stream ends.
        assert_eq!(notes(&out).len(), SCALE.len(), "{out:?}");
    }

    /// The scale played legato: each note slides into the next over its last 30 ms, unbroken.
    fn legato_scale() -> Vec<f32> {
        let notes: Vec<(f64, f64)> = SCALE.iter().map(|&f| (f, 0.25)).collect();
        padded(synth::legato(RATE, &notes, 0.03, 4, 0.5))
    }

    fn strengths(events: &[Event]) -> Vec<f32> {
        events
            .iter()
            .filter_map(|e| match *e {
                Event::Note { strength, .. } => Some(strength),
                _ => None,
            })
            .collect()
    }

    fn onsets(events: &[Event]) -> Vec<(u64, f32)> {
        events
            .iter()
            .filter_map(|e| match *e {
                Event::Onset { frame, strength } => Some((frame, strength)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_legato_scale_splits_at_each_held_change_and_keeps_its_strength() {
        let events = run(&legato_scale(), 1000);
        let found = notes(&events);
        assert_eq!(
            found.iter().map(|n| n.2).collect::<Vec<_>>(),
            vec![60, 62, 64, 65, 67],
            "{events:?}"
        );
        // Only the entry is an onset, so every later split is a held pitch change.
        let heard = onsets(&events);
        assert_eq!(heard.len(), 1, "{events:?}");
        for (i, &(start, _, _)) in found.iter().enumerate().skip(1) {
            // Each note starts where the slide into it does, 30 ms before its boundary.
            let boundary = 0.2 + 0.25 * i as f64;
            assert!(
                (boundary - 0.04..=boundary).contains(&start),
                "note {i}: {found:?}"
            );
        }
        assert!(
            strengths(&events).iter().all(|&s| s == heard[0].1),
            "{events:?}"
        );
    }

    #[test]
    fn a_scoop_reads_as_the_pitch_it_holds() {
        // Held 45 cents above 56: the note is 57. Scooping up from 80 cents flat over its first
        // 15% pulls the mean of its frames below 56.5, so only the median reads 57.
        let held = 440.0 * ((56.55 - 69.0) / 12.0_f64).exp2();
        let signal = padded(synth::scoop(RATE, held, 0.6, 80.0, 0.15, 4, 0.4));
        let found = notes(&run(&signal, 1000));
        assert_eq!(
            found.iter().map(|n| n.2).collect::<Vec<_>>(),
            vec![57],
            "{found:?}"
        );

        let mut track = PitchTrack::new(RATE);
        let mut frames = Vec::new();
        track
            .push(
                Mono {
                    rate: RATE,
                    start_frame: 0,
                    samples: signal,
                },
                &mut frames,
            )
            .unwrap();
        track.finish(&mut frames).unwrap();
        let pitches: Vec<f32> = frames
            .iter()
            .filter_map(|p| p.hz)
            .map(|hz| 69.0 + 12.0 * (hz / 440.0).log2())
            .collect();
        let mean = pitches.iter().sum::<f32>() / pitches.len() as f32;
        assert_eq!(mean.round(), 56.0, "the mean would read {mean}");
    }

    #[test]
    fn a_noisy_but_pitched_note_is_kept() {
        let mut signal = padded(synth::tone(RATE, 220.0, 0.6, 4, 0.4));
        let noise = synth::noise_bursts(RATE, 1.1, &[(0.2, 0.6)], 100.0, 8000.0, 0.09);
        synth::add(&mut signal, &noise);
        let events = run(&signal, 1000);
        let found: Vec<(u8, f32)> = events
            .iter()
            .filter_map(|e| match *e {
                Event::Note {
                    midi, confidence, ..
                } => Some((midi, confidence)),
                _ => None,
            })
            .collect();
        assert_eq!(found.len(), 1, "{events:?}");
        assert_eq!(found[0].0, 57);
        // A mean voicing of 0.30 to 0.45 is a YIN trough about 0.10 to 0.12 deep: breathy.
        assert!((0.30..=0.45).contains(&found[0].1), "{events:?}");
    }

    #[test]
    fn a_consonant_before_the_vowel_gives_the_note_its_strength() {
        // 120 ms of hiss, then the vowel, which swells in with no attack of its own.
        let mut signal = synth::silence(RATE, 0.2);
        signal.extend(synth::noise_bursts(
            RATE,
            0.12,
            &[(0.0, 0.12)],
            2000.0,
            8000.0,
            0.3,
        ));
        let mut vowel = synth::tone(RATE, 220.0, 0.5, 4, 0.4);
        synth::fade_in(RATE, &mut vowel, 0.06);
        signal.extend(vowel);
        signal.extend(synth::silence(RATE, 0.3));
        let events = run(&signal, 1000);
        let hiss = onsets(&events)[0];
        assert!(
            (hiss.0 as f64 / f64::from(RATE) - 0.2).abs() <= 0.02,
            "{events:?}"
        );
        let found = notes(&events);
        assert_eq!(found.len(), 1, "{events:?}");
        // The note starts at the vowel, and its strength is the hiss's.
        assert!(found[0].0 >= 0.3, "{events:?}");
        assert_eq!(strengths(&events), vec![hiss.1], "{events:?}");
    }

    #[test]
    fn a_soft_entry_takes_its_strength_from_its_level() {
        let entry = |amplitude| {
            let mut tone = synth::tone(RATE, 220.0, 0.6, 4, amplitude);
            synth::fade_in(RATE, &mut tone, 0.15);
            let events = run(&padded(tone), 1000);
            // Nothing before the note is an onset: only the tone's hard end is.
            let late = u64::from(RATE) * 7 / 10;
            assert!(onsets(&events).iter().all(|o| o.0 > late), "{events:?}");
            match strengths(&events)[..] {
                [strength] => strength,
                _ => panic!("wanted one note: {events:?}"),
            }
        };
        let (soft, loud) = (entry(0.1), entry(0.4));
        assert!(soft > 0.0, "{soft}");
        assert!(loud > soft, "{soft} then {loud}");
    }

    #[test]
    fn notes_and_onsets_are_the_same_whatever_the_chunking() {
        let mut signal = scale();
        signal.extend(legato_scale());
        let reference = run(&signal, 1000);
        assert!(!notes(&reference).is_empty());
        for chunk in [1, 777, 4096] {
            let events = run(&signal, chunk);
            assert_eq!(notes(&events), notes(&reference), "chunk of {chunk}");
            assert_eq!(onsets(&events), onsets(&reference), "chunk of {chunk}");
        }
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
                let level = 0.1;
                segmenter.pitch(
                    PitchFrame {
                        frame,
                        hz,
                        fine_hz: hz,
                        voiced,
                        level,
                    },
                    &mut out,
                );
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
    fn a_note_split_by_a_held_change_inherits_its_strength() {
        let mut segmenter = Segmenter::new(RATE, 256);
        segmenter.onset(0, 0.7);
        let mut out = Vec::new();
        for (k, midi) in [60.0f32; 30].into_iter().chain([62.0; 30]).enumerate() {
            let hz = Some(440.0 * ((midi - 69.0) / 12.0).exp2());
            let frame = 128 + 256 * k as u64;
            let p = PitchFrame {
                frame,
                hz,
                fine_hz: hz,
                voiced: 1.0,
                level: 0.1,
            };
            segmenter.pitch(p, &mut out);
        }
        segmenter.finish(&mut out);
        assert_eq!(strengths(&out), vec![0.7, 0.7], "{out:?}");
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

    /// Feeds the segmenter one fully voiced frame per pitch, one hop (11.6 ms) apart, then
    /// silence; `None` is unvoiced.
    fn segment_pitches(pitches: &[Option<f32>]) -> Vec<(u64, u64, u8)> {
        let mut stretches: Vec<(Option<f32>, usize)> = pitches.iter().map(|&p| (p, 1)).collect();
        stretches.push((None, 10));
        segment(&stretches)
    }

    fn midis(found: &[(u64, u64, u8)]) -> Vec<u8> {
        found.iter().map(|n| n.2).collect()
    }

    #[test]
    fn a_sung_note_with_vibrato_and_a_slow_drift_is_one_note() {
        // 1.5 s at 57, swinging 40 cents either side, and sagging half a semitone in a straight
        // line over 0.6 s from 0.4 s in: it moves, but never arrives anywhere new and stays, so
        // it is not two notes yet. Every rate a singer's vibrato is likely to have, and every
        // phase it may start in, since a window over part of a swing reads it off centre.
        let hop = 256.0 / f64::from(RATE);
        for rate in [4.5, 5.0, 6.0, 7.0] {
            for eighth in 0..8 {
                let phase = f64::from(eighth) * std::f64::consts::TAU / 8.0;
                let pitches: Vec<Option<f32>> = (0..129)
                    .map(|k| {
                        let t = k as f64 * hop;
                        let vibrato = 0.4 * (std::f64::consts::TAU * rate * t + phase).sin();
                        let sag = 0.5 * ((t - 0.4) / 0.6).clamp(0.0, 1.0);
                        Some((57.0 + vibrato - sag) as f32)
                    })
                    .collect();
                let found = segment_pitches(&pitches);
                assert_eq!(
                    found.len(),
                    1,
                    "{rate} Hz from {eighth}/8 of a swing: {found:?}"
                );
            }
        }
    }

    #[test]
    fn a_wide_slow_vibrato_with_a_drift_is_one_note() {
        // Half a semitone either side, 4.5 to 5 swings a second, with the same half-semitone
        // sag: measured against the whole note so far, the sag and the swing together leave it.
        let hop = 256.0 / f64::from(RATE);
        for rate in [4.5, 5.0] {
            for eighth in 0..8 {
                let phase = f64::from(eighth) * std::f64::consts::TAU / 8.0;
                let pitches: Vec<Option<f32>> = (0..129)
                    .map(|k| {
                        let t = k as f64 * hop;
                        let vibrato = 0.5 * (std::f64::consts::TAU * rate * t + phase).sin();
                        let sag = 0.5 * ((t - 0.4) / 0.6).clamp(0.0, 1.0);
                        Some((57.0 + vibrato - sag) as f32)
                    })
                    .collect();
                let found = segment_pitches(&pitches);
                assert_eq!(
                    found.len(),
                    1,
                    "{rate} Hz from {eighth}/8 of a swing: {found:?}"
                );
            }
        }
    }

    #[test]
    fn a_quick_semitone_step_that_holds_is_two_notes() {
        let pitches = [held(60.0, 30), line(60.0, 61.0, 4), held(61.0, 30)].concat();
        assert_eq!(midis(&segment_pitches(&pitches)), vec![60, 61]);
    }

    #[test]
    fn a_dip_between_two_syllables_is_two_notes_and_the_dip_none() {
        assert_eq!(
            midis(&segment_pitches(&dip_between_syllables())),
            vec![60, 60]
        );
    }

    #[test]
    fn a_slower_dip_is_still_no_note_at_its_bottom() {
        let found = segment_pitches(&slower_dip());
        assert_eq!(midis(&found), vec![60, 60], "{found:?}");
    }

    #[test]
    fn a_whole_tone_portamento_is_no_note_of_its_own() {
        // A whole tone over 0.2 s, as a singer joins two notes: 10 semitones a second.
        let pitches = [held(60.0, 25), line(60.0, 62.0, 17), held(62.0, 25)].concat();
        let found = segment_pitches(&pitches);
        assert_eq!(midis(&found), vec![60, 62], "{found:?}");
    }

    #[test]
    fn a_slide_between_two_held_notes_is_no_note_of_its_own() {
        // 5 semitones over 150 ms: the slide belongs to the note it arrives at.
        let pitches = [held(60.0, 25), line(60.0, 65.0, 13), held(65.0, 25)].concat();
        let found = segment_pitches(&pitches);
        assert_eq!(midis(&found), vec![60, 65], "{found:?}");
        // The second note starts where the slide does, within the two frames it takes to move
        // further than CHANGE_SEMITONES, not where the pitch arrives 13 frames later.
        let slide = 25 * 256 - 128;
        assert!((slide..=slide + 2 * 256).contains(&found[1].0), "{found:?}");
        assert_eq!(found[0].1, found[1].0, "{found:?}");
    }

    #[test]
    fn a_slow_glide_to_a_new_note_never_merges_the_two() {
        // A slow glide is followed frame by frame, so it is never a quick departure; it must
        // still end the note it leaves once it is far from where that note was held, or both
        // notes read as one at a pitch between them.
        let hop = 256.0 / f64::from(RATE);
        for (semitones, secs) in [(2.0f32, 1.0f64), (3.0, 0.5), (5.0, 1.0)] {
            let frames = (secs / hop).round() as usize;
            let pitches = [
                held(60.0, 25),
                line(60.0, 60.0 + semitones, frames),
                held(60.0 + semitones, 25),
            ]
            .concat();
            let found = midis(&segment_pitches(&pitches));
            let case = format!("{semitones} st over {secs} s: {found:?}");
            assert_eq!(found.first(), Some(&60), "{case}");
            assert_eq!(found.last(), Some(&(60 + semitones as u8)), "{case}");
            assert!(found.windows(2).all(|w| w[0] < w[1]), "{case}");
        }
    }

    #[test]
    fn a_note_after_a_long_slide_is_pitched_by_where_it_holds() {
        // The slide's 13 frames outnumber the 8 that hold at 65: counted, they would pull the
        // median down to 64.
        let pitches = [held(60.0, 25), line(60.0, 65.0, 13), held(65.0, 8)].concat();
        assert_eq!(midis(&segment_pitches(&pitches)), vec![60, 65]);
    }

    #[test]
    fn a_fall_into_silence_is_no_note() {
        // The end of a phrase: the held note falls away 10 semitones as the voice stops.
        let pitches = [held(60.0, 25), line(60.0, 50.0, 10)].concat();
        let found = segment_pitches(&pitches);
        assert_eq!(midis(&found), vec![60], "{found:?}");
    }

    #[test]
    fn an_onset_reaches_the_next_note_past_a_dropped_fragment_and_slide() {
        // The onset's run is too short to be a note before the pitch slides away, and the slide
        // ends in silence; the next note, 151 ms after the onset, is the latest note since it.
        let mut segmenter = Segmenter::new(RATE, 256);
        segmenter.onset(0, 0.9);
        let mut out = Vec::new();
        let pitches = [
            held(60.0, 3),
            line(60.0, 66.0, 8),
            vec![None; 2],
            held(62.0, 20),
            vec![None; 10],
        ]
        .concat();
        for (k, midi) in pitches.into_iter().enumerate() {
            let hz = midi.map(|m| 440.0 * ((m - 69.0) / 12.0).exp2());
            let p = PitchFrame {
                frame: 256 * k as u64,
                hz,
                fine_hz: hz,
                voiced: if hz.is_some() { 1.0 } else { 0.0 },
                level: 0.1,
            };
            segmenter.pitch(p, &mut out);
        }
        segmenter.finish(&mut out);
        assert_eq!(strengths(&out), vec![0.9], "{out:?}");
    }

    #[test]
    fn a_short_excursion_early_or_through_vibrato_stays_in_the_note() {
        // A tracker blip of 1 to 4 frames, 2 to 9 semitones away, at a consonant or a break in
        // the voice: the pitch comes straight back, so it never left the note, and the blip
        // must not move where the note is held.
        let hop = 256.0 / f64::from(RATE);
        let blips: [&[f32]; 5] = [&[3.0; 3], &[-3.0; 4], &[5.0, 9.0, 5.0], &[2.0; 4], &[5.0]];
        for blip in blips {
            for at in [8usize, 40] {
                for eighth in [None, Some(0), Some(2), Some(4), Some(6)] {
                    let pitches: Vec<Option<f32>> = (0..100usize)
                        .map(|k| {
                            let vibrato = eighth.map_or(0.0, |e: i32| {
                                let phase = f64::from(e) * std::f64::consts::TAU / 8.0;
                                0.4 * (std::f64::consts::TAU * 6.0 * k as f64 * hop + phase).sin()
                            });
                            let away = k.checked_sub(at).and_then(|i| blip.get(i)).copied();
                            Some(60.0 + vibrato as f32 + away.unwrap_or(0.0))
                        })
                        .collect();
                    let found = segment_pitches(&pitches);
                    let case = format!("{blip:?} at frame {at}, vibrato {eighth:?}: {found:?}");
                    assert_eq!(midis(&found), vec![60], "{case}");
                }
            }
        }
    }

    #[test]
    fn a_scoop_at_a_notes_start_is_part_of_the_note_it_reaches() {
        // The voice comes in 4 semitones low and scoops up over 255 ms: it moves 0.73 semitone
        // over every 50 ms, so it never holds, and it is long enough before it leaves its start
        // to be a note if it had. It is the start of the note it reaches.
        let pitches = [vec![None; 5], line(50.0, 54.0, 22), held(54.0, 40)].concat();
        let found = segment_pitches(&pitches);
        assert_eq!(midis(&found), vec![54], "{found:?}");
        // It starts where the voice does, half a hop before its first frame.
        assert_eq!(found[0].0, 5 * 256 - 128, "{found:?}");
    }

    #[test]
    fn a_short_note_with_vibrato_or_jitter_before_a_step_is_kept() {
        // Held pitch with a swing or a tracker's jitter is still held: 70 to 93 ms of a 40-cent
        // vibrato at 6 to 7 a second, in any phase, or 465 ms of jitter up to 0.4 either side.
        let hop = 256.0 / f64::from(RATE);
        let step = |note: Vec<Option<f32>>| {
            let pitches = [vec![None; 5], note, line(60.0, 62.0, 2), held(62.0, 40)].concat();
            midis(&segment_pitches(&pitches))
        };
        for rate in [6.0, 7.0] {
            for frames in [6usize, 8] {
                for eighth in 0..8 {
                    let phase = f64::from(eighth) * std::f64::consts::TAU / 8.0;
                    let note = (0..frames)
                        .map(|k| {
                            let t = k as f64 * hop;
                            let swing = (std::f64::consts::TAU * rate * t + phase).sin();
                            Some(60.0 + 0.4 * swing as f32)
                        })
                        .collect();
                    let case = format!("{frames} frames at {rate} Hz from {eighth}/8");
                    assert_eq!(step(note), vec![60, 62], "{case}");
                }
            }
        }
        for seed in [1u32, 2, 3, 4] {
            let mut state = seed;
            let note = (0..40)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let unit = (state >> 8) as f32 / (1 << 24) as f32;
                    Some(60.0 + 0.8 * (unit - 0.5))
                })
                .collect();
            assert_eq!(step(note), vec![60, 62], "jitter, seed {seed}");
        }
    }

    #[test]
    fn a_short_note_that_holds_before_a_step_is_kept() {
        // 70 ms held still, then a quick step: a note, however short.
        let pitches = [
            vec![None; 5],
            held(50.0, 6),
            line(50.0, 54.0, 3),
            held(54.0, 40),
        ]
        .concat();
        assert_eq!(midis(&segment_pitches(&pitches)), vec![50, 54]);
    }

    #[test]
    fn a_run_shorter_than_the_shortest_note_is_dropped() {
        let found = segment(&[(None, 5), (Some(60.0), 3), (None, 5)]);
        assert_eq!(found, vec![]);
    }

    /// A stated limit, not a desired reading: a slide into a wobble that never spreads within
    /// [`ARRIVED_SEMITONES`] is no note, though it is sung for a third of a second. A change
    /// to how a slide arrives that reads it is a decision about this limit, and changes this
    /// test with it.
    #[test]
    fn stated_limit_a_slide_into_a_wobble_that_never_arrives_is_no_note() {
        let frames = wobbling_slide();
        let hold = Segmenter::new(RATE, 256).hold;
        let wobble: Vec<f32> = frames.iter().skip(59).map_while(|f| f.0).collect();
        let narrowest = wobble.windows(hold).map(spread).fold(f32::MAX, f32::min);
        assert!(
            narrowest > ARRIVED_SEMITONES,
            "the wobble arrives somewhere: {narrowest}"
        );
        let mut segmenter = Segmenter::new(RATE, 256);
        let mut out = Vec::new();
        for (k, &(pitch, voiced)) in frames.iter().enumerate() {
            // On D#3: the held note is F#3.
            let pitch = pitch.map(|p| 51.0 + p);
            segmenter.take_frame(k as u64 * 256, pitch, voiced, 0.1, &mut out);
        }
        segmenter.finish_segments(&mut out);
        let keys: Vec<f32> = out.iter().map(|s| s.median.round()).collect();
        assert_eq!(keys, [54.0], "only the held note: {out:?}");
    }
}
