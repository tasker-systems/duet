//! The performance: every settled pitch frame of a take, beside its onsets, before any reading
//! is made of it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::event::Event;
use crate::onset::Onsets;
use crate::pitch::{PitchFrame, PitchTrack};
use crate::stage::{Mono, Stage};

/// About how long a [`FrameBlock`] is: a second of frames.
const BLOCK_SECS: f64 = 1.0;

/// The pitch unit a performance is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub enum PitchUnit {
    /// Absolute cents, `1200·log2(hz/440) + 6900`: MIDI's numbering at 100 a key, so 6900.0 is
    /// 440 Hz and 6907.9 is 442 Hz. A unit, not a reading: it maps one-to-one onto Hz, and is
    /// never rounded to a key.
    #[serde(rename = "absolute-cents")]
    AbsoluteCents,
}

/// What every frame of a performance shares: the first line's head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PerformanceHead {
    /// The analysis rate, in Hz: what every frame number counts.
    #[schemars(range(min = 1))]
    pub rate: u32,
    /// Sample frames from one pitch frame to the next.
    #[schemars(range(min = 1))]
    pub hop: u32,
    /// The sample frame the first pitch frame sits at, on the take's timeline at `rate`: the
    /// middle of its integration window.
    pub first_frame: u64,
    /// The unit of every block's `pitch`.
    pub unit: PitchUnit,
}

impl PerformanceHead {
    /// A head for frames `hop` apart from `first_frame`, at `rate`, in absolute cents.
    #[must_use]
    pub const fn new(rate: u32, hop: u32, first_frame: u64) -> PerformanceHead {
        PerformanceHead {
            rate,
            hop,
            first_frame,
            unit: PitchUnit::AbsoluteCents,
        }
    }
}

/// A run of consecutive pitch frames, as columns: frame `k` of the block sits at sample frame
/// `start + k·hop`, and has `pitch[k]`, `voicing[k]` and `level[k]`. The three columns have one
/// length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FrameBlock {
    /// The sample frame the block's first frame sits at, on the take's timeline.
    pub start: u64,
    /// Each frame's fundamental, in the head's unit to 0.1, or `null` where the smoothed track
    /// says the frame is unvoiced.
    pub pitch: Vec<Option<f32>>,
    /// Each frame's own voicing probability, before smoothing.
    #[schemars(inner(range(min = 0.0, max = 1.0)))]
    pub voicing: Vec<f32>,
    /// Each frame's level: the root mean square of its integration window.
    #[schemars(inner(range(min = 0.0)))]
    pub level: Vec<f32>,
}

impl FrameBlock {
    /// A block from its columns.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the columns differ in length.
    pub fn new(
        start: u64,
        pitch: Vec<Option<f32>>,
        voicing: Vec<f32>,
        level: Vec<f32>,
    ) -> Result<FrameBlock> {
        let block = FrameBlock {
            start,
            pitch,
            voicing,
            level,
        };
        block.check()?;
        Ok(block)
    }

    /// The number of frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pitch.len()
    }

    /// Whether the block holds no frames.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pitch.is_empty()
    }

    /// Checks that the three columns have one length, that every voicing is a probability, and
    /// that every level is at least 0.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`], naming the lengths when they differ, or the first frame out of range.
    pub fn check(&self) -> Result<()> {
        let (p, v, l) = (self.pitch.len(), self.voicing.len(), self.level.len());
        if p != v || v != l {
            return Err(Error::Invalid(format!(
                "a frame block's columns differ in length: pitch {p}, voicing {v}, level {l}"
            )));
        }
        let out_of_range = |what: &str, k: usize, value: f32, range: &str| {
            Err(Error::Invalid(format!(
                "frame {k} of the block at {}: {what} {value} is not {range}",
                self.start
            )))
        };
        for (k, (&voicing, &level)) in self.voicing.iter().zip(&self.level).enumerate() {
            if !(0.0..=1.0).contains(&voicing) {
                return out_of_range("voicing", k, voicing, "from 0 to 1");
            }
            if !(level >= 0.0 && level.is_finite()) {
                return out_of_range("level", k, level, "a finite number from 0");
            }
        }
        Ok(())
    }

    /// An empty block starting at `start`.
    fn empty(start: u64) -> FrameBlock {
        FrameBlock {
            start,
            pitch: Vec::new(),
            voicing: Vec::new(),
            level: Vec::new(),
        }
    }
}

/// `hz` in absolute cents, `1200·log2(hz/440) + 6900`, unrounded.
#[must_use]
pub fn absolute_cents(hz: f64) -> f64 {
    1200.0 * (hz / 440.0).log2() + 6900.0
}

/// `hz` in absolute cents to 0.1 cent, as a performance writes it.
fn written_cents(hz: f32) -> f32 {
    ((absolute_cents(f64::from(hz)) * 10.0).round() / 10.0) as f32
}

/// What [`Performance`] emits.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Layer1 {
    /// Emitted once, first, on the first push.
    Head(PerformanceHead),
    /// The next block of frames, once its last frame has settled.
    Frames(FrameBlock),
    /// An onset, as it settles: [`Event::Onset`]'s fields.
    Onset { frame: u64, strength: f32 },
}

/// Gathers settled pitch frames into blocks of a fixed number of frames, counted from the
/// first, so the blocks do not depend on how the stream is cut into chunks.
struct Blocks {
    size: usize,
    current: Option<FrameBlock>,
}

impl Blocks {
    fn new(rate: u32, hop: u64) -> Blocks {
        Blocks {
            size: ((BLOCK_SECS * f64::from(rate) / hop as f64).round() as usize).max(1),
            current: None,
        }
    }

    fn frame(&mut self, frame: &PitchFrame, out: &mut Vec<Layer1>) {
        let block = self
            .current
            .get_or_insert_with(|| FrameBlock::empty(frame.frame));
        block.pitch.push(frame.fine_hz.map(written_cents));
        block.voicing.push(frame.voiced);
        block.level.push(frame.level);
        if block.len() == self.size {
            out.extend(self.current.take().map(Layer1::Frames));
        }
    }

    fn finish(&mut self, out: &mut Vec<Layer1>) {
        out.extend(self.current.take().map(Layer1::Frames));
    }
}

/// The performance of a melody take: pitch tracking by [`PitchTrack`] and onsets by [`Onsets`]
/// over the same input, with every settled pitch frame kept rather than consumed.
///
/// It emits [`Layer1::Head`] on the first push, then each onset as it settles and each block of
/// frames once its last frame has settled. A block is a second's worth of frames (86 at
/// 22 050 Hz), counted from the first frame; the last may be shorter. A frame's pitch is
/// [`PitchFrame::fine_hz`] in absolute cents to 0.1 cent ([`absolute_cents`]), or `None` when
/// unvoiced; its voicing and level are the frame's own, unrounded.
///
/// The head, the blocks and the onsets do not depend on how the stream is cut into chunks; how
/// the onsets interleave with the blocks does.
///
/// [`Melody`](crate::Melody) runs one of these and segments its frames into notes.
pub struct Performance {
    pitch: PitchTrack,
    onsets: Onsets,
    blocks: Blocks,
    headed: bool,
}

impl std::fmt::Debug for Performance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Performance")
            .field("pitch", &self.pitch)
            .field("onsets", &self.onsets)
            .finish_non_exhaustive()
    }
}

impl Performance {
    /// A stage for mono input at `rate`.
    pub fn new(rate: u32) -> Performance {
        let pitch = PitchTrack::new(rate);
        let blocks = Blocks::new(rate, pitch.hop());
        Performance {
            pitch,
            onsets: Onsets::new(rate),
            blocks,
            headed: false,
        }
    }

    /// The head, once the first input has been pushed.
    #[must_use]
    pub fn head(&self) -> Option<PerformanceHead> {
        let first = self.pitch.first_frame()?;
        let hop = u32::try_from(self.pitch.hop()).expect("a hop fits in u32");
        Some(PerformanceHead::new(self.pitch.rate(), hop, first))
    }

    /// Samples between one pitch frame and the next.
    pub(crate) fn hop(&self) -> u64 {
        self.pitch.hop()
    }

    /// How long an onset is held back.
    pub(crate) fn onset_latency(&self) -> u64 {
        self.onsets.latency_frames()
    }

    /// How long a pitch frame is held back before it settles.
    pub(crate) fn pitch_latency(&self) -> u64 {
        self.pitch.latency_frames()
    }

    /// Pushes `input` to both trackers: the onsets that settle go to `onsets`, the pitch frames
    /// to `frames`.
    pub(crate) fn push_raw(
        &mut self,
        input: Mono,
        frames: &mut Vec<PitchFrame>,
        onsets: &mut Vec<Event>,
    ) -> Result<()> {
        self.onsets.push(input.clone(), onsets)?;
        self.pitch.push(input, frames)
    }

    /// Ends both trackers, as [`push_raw`](Self::push_raw) pushes to them.
    pub(crate) fn finish_raw(
        &mut self,
        frames: &mut Vec<PitchFrame>,
        onsets: &mut Vec<Event>,
    ) -> Result<()> {
        self.onsets.finish(onsets)?;
        self.pitch.finish(frames)
    }

    /// Emits the head, if it is not yet emitted, and every block `frames` completes.
    pub(crate) fn keep(&mut self, frames: &[PitchFrame], out: &mut Vec<Layer1>) {
        if !self.headed
            && let Some(head) = self.head()
        {
            self.headed = true;
            out.push(Layer1::Head(head));
        }
        for frame in frames {
            self.blocks.frame(frame, out);
        }
    }

    /// Emits the last, partial block.
    pub(crate) fn keep_last(&mut self, out: &mut Vec<Layer1>) {
        self.blocks.finish(out);
    }
}

fn onsets_out(events: Vec<Event>, out: &mut Vec<Layer1>) {
    out.extend(events.into_iter().filter_map(|event| match event {
        Event::Onset { frame, strength } => Some(Layer1::Onset { frame, strength }),
        _ => None,
    }));
}

impl Stage for Performance {
    type In = Mono;
    type Out = Layer1;

    fn push(&mut self, input: Mono, out: &mut Vec<Layer1>) -> Result<()> {
        let (mut frames, mut onsets) = (Vec::new(), Vec::new());
        self.push_raw(input, &mut frames, &mut onsets)?;
        self.keep(&[], out);
        onsets_out(onsets, out);
        self.keep(&frames, out);
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<Layer1>) -> Result<()> {
        let (mut frames, mut onsets) = (Vec::new(), Vec::new());
        self.finish_raw(&mut frames, &mut onsets)?;
        onsets_out(onsets, out);
        self.keep(&frames, out);
        self.keep_last(out);
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        let block = (self.blocks.size as u64 - 1) * self.hop();
        (self.pitch_latency() + block).max(self.onset_latency())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    const RATE: u32 = 22_050;

    fn run(signal: &[f32], chunk: usize) -> Vec<Layer1> {
        let mut stage = Performance::new(RATE);
        let mut out = Vec::new();
        for (i, piece) in signal.chunks(chunk).enumerate() {
            let mono = Mono {
                rate: RATE,
                start_frame: 1000 + (i * chunk) as u64,
                samples: piece.to_vec(),
            };
            stage.push(mono, &mut out).unwrap();
        }
        stage.finish(&mut out).unwrap();
        out
    }

    fn head(out: &[Layer1]) -> PerformanceHead {
        match out.first() {
            Some(Layer1::Head(head)) => *head,
            other => panic!("the first output is not the head: {other:?}"),
        }
    }

    fn blocks(out: &[Layer1]) -> Vec<&FrameBlock> {
        out.iter()
            .filter_map(|o| match o {
                Layer1::Frames(block) => Some(block),
                _ => None,
            })
            .collect()
    }

    fn onsets(out: &[Layer1]) -> Vec<(u64, f32)> {
        out.iter()
            .filter_map(|o| match *o {
                Layer1::Onset { frame, strength } => Some((frame, strength)),
                _ => None,
            })
            .collect()
    }

    /// The voiced pitches of the frames well inside a tone that runs from `from` to `to`
    /// seconds of the signal.
    fn pitches_within(out: &[Layer1], from: f64, to: f64) -> Vec<f32> {
        let h = head(out);
        let at = |secs: f64| 1000 + (secs * f64::from(RATE)) as u64;
        blocks(out)
            .iter()
            .flat_map(|b| {
                b.pitch
                    .iter()
                    .enumerate()
                    .map(move |(k, p)| (b.start + k as u64 * u64::from(h.hop), *p))
            })
            .filter(|&(frame, _)| frame > at(from) && frame < at(to))
            .map(|(frame, p)| p.unwrap_or_else(|| panic!("frame {frame} is unvoiced")))
            .collect()
    }

    fn padded(tone: Vec<f32>) -> Vec<f32> {
        [synth::silence(RATE, 0.3), tone, synth::silence(RATE, 0.3)].concat()
    }

    #[test]
    fn a_442_hz_tone_is_written_at_6907_9_cents() {
        let out = run(&padded(synth::tone(RATE, 442.0, 1.5, 4, 0.3)), 4096);
        let pitches = pitches_within(&out, 0.45, 1.65);
        assert!(pitches.len() > 100, "{}", pitches.len());
        for p in pitches {
            assert!((p - 6907.9).abs() <= 1.0, "{p}");
        }
    }

    #[test]
    fn a_tone_13_cents_off_the_grid_is_written_13_cents_off() {
        let hz = 440.0 * (13.0f64 / 1200.0).exp2();
        let out = run(&padded(synth::tone(RATE, hz, 1.5, 4, 0.3)), 4096);
        let pitches = pitches_within(&out, 0.45, 1.65);
        assert!(pitches.len() > 100, "{}", pitches.len());
        for p in pitches {
            assert!((p - 6913.0).abs() <= 1.0, "{p}");
        }
    }

    #[test]
    fn pitch_is_written_to_a_tenth_of_a_cent_and_unvoiced_frames_are_none() {
        let out = run(&padded(synth::tone(RATE, 300.0, 0.8, 4, 0.3)), 512);
        let all: Vec<Option<f32>> = blocks(&out).iter().flat_map(|b| b.pitch.clone()).collect();
        assert!(all.iter().any(Option::is_none), "the silence is voiced");
        for p in all.into_iter().flatten() {
            let tenth = ((f64::from(p) * 10.0).round() / 10.0) as f32;
            assert_eq!(p, tenth);
        }
    }

    #[test]
    fn the_same_frames_whatever_the_chunking() {
        let mut signal =
            synth::note_sequence(RATE, &[(261.6, 0.6), (329.6, 0.7), (392.0, 0.5)], 4, 0.3);
        signal.extend(synth::silence(RATE, 0.2));
        let reference = run(&signal, 4096);
        assert!(blocks(&reference).len() > 1);
        for chunk in [1, 512, 4096] {
            let out = run(&signal, chunk);
            assert_eq!(head(&out), head(&reference), "chunk of {chunk}");
            assert_eq!(blocks(&out), blocks(&reference), "chunk of {chunk}");
            assert_eq!(onsets(&out), onsets(&reference), "chunk of {chunk}");
        }
    }

    #[test]
    fn blocks_are_a_second_of_consecutive_frames_from_the_head() {
        let out = run(&synth::tone(RATE, 300.0, 2.5, 4, 0.3), 1000);
        let h = head(&out);
        assert_eq!(
            (h.rate, h.hop, h.unit),
            (RATE, 256, PitchUnit::AbsoluteCents)
        );
        let blocks = blocks(&out);
        assert_eq!(blocks[0].start, h.first_frame);
        let mut next = h.first_frame;
        for (i, block) in blocks.iter().enumerate() {
            block.check().unwrap();
            assert_eq!(block.start, next);
            if i + 1 < blocks.len() {
                assert_eq!(block.len(), 86);
            }
            next += block.len() as u64 * u64::from(h.hop);
        }
        // Every frame the pitch track settles, from the first, is in a block.
        let total: usize = blocks.iter().map(|b| b.len()).sum();
        let mut track = PitchTrack::new(RATE);
        let mut frames = Vec::new();
        track
            .push(
                Mono {
                    rate: RATE,
                    start_frame: 1000,
                    samples: synth::tone(RATE, 300.0, 2.5, 4, 0.3),
                },
                &mut frames,
            )
            .unwrap();
        track.finish(&mut frames).unwrap();
        assert_eq!(total, frames.len());
    }

    #[test]
    fn a_block_is_emitted_within_the_stage_latency() {
        let signal = synth::tone(RATE, 300.0, 3.0, 4, 0.3);
        let mut stage = Performance::new(RATE);
        let mut out = Vec::new();
        let chunk = 64;
        for (i, piece) in signal.chunks(chunk).enumerate() {
            let before = out.len();
            stage
                .push(
                    Mono {
                        rate: RATE,
                        start_frame: (i * chunk) as u64,
                        samples: piece.to_vec(),
                    },
                    &mut out,
                )
                .unwrap();
            let fed = ((i + 1) * chunk) as u64;
            for o in &out[before..] {
                if let Layer1::Frames(block) = o {
                    assert!(fed <= block.start + stage.latency_frames() + chunk as u64);
                }
            }
        }
        // And no block waits for the end of the stream.
        assert!(blocks(&out).len() >= 2);
    }

    #[test]
    fn columns_of_different_lengths_are_refused() {
        let err =
            FrameBlock::new(0, vec![None, Some(6900.0)], vec![0.1], vec![0.0, 0.1]).unwrap_err();
        assert!(
            err.to_string().contains("pitch 2, voicing 1, level 2"),
            "{err}"
        );
    }

    #[test]
    fn a_voicing_or_level_out_of_range_is_refused() {
        let err = FrameBlock::new(512, vec![None], vec![1.5], vec![0.0]).unwrap_err();
        assert!(
            err.to_string()
                .contains("frame 0 of the block at 512: voicing 1.5"),
            "{err}"
        );
        let err =
            FrameBlock::new(0, vec![None, None], vec![0.0, 0.0], vec![0.0, -0.1]).unwrap_err();
        assert!(
            err.to_string()
                .contains("frame 1 of the block at 0: level -0.1"),
            "{err}"
        );
    }

    #[test]
    fn absolute_cents_is_midi_at_a_hundred_a_key() {
        assert_eq!(absolute_cents(440.0), 6900.0);
        assert!((absolute_cents(261.625_565) - 6000.0).abs() < 1e-4);
        assert_eq!(written_cents(442.0), 6907.9);
    }
}
