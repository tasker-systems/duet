use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use crate::error::{Error, Result};
use crate::stage::{Chunk, Mono, Stage};

/// The one rate every analysis stage works at.
///
/// 22 050 Hz keeps everything capture listens for: pitch up to its Nyquist of 11 025 Hz and
/// the body of a hat's noise, at half the cost of 44 100 Hz in every stage after this one.
pub const ANALYSIS_RATE: u32 = 22_050;

/// Input frames the resampler takes per step.
const RESAMPLE_CHUNK: usize = 1024;

/// Downmixes to one channel and resamples to one rate, so every stage after it can assume both.
///
/// The input rate is the first chunk's, and a later chunk at another rate is refused.
/// Resampling holds back its filter delay and up to one step of input, which
/// [`latency_frames`](Stage::latency_frames) reports once the first chunk has fixed the rate.
#[derive(Debug)]
pub struct Prepare {
    rate: u32,
    input_rate: Option<u32>,
    resampling: Option<Resampling>,
}

impl Prepare {
    /// A stage that outputs mono at `rate`.
    pub fn new(rate: u32) -> Prepare {
        Prepare {
            rate,
            input_rate: None,
            resampling: None,
        }
    }
}

impl Stage for Prepare {
    type In = Chunk;
    type Out = Mono;

    fn push(&mut self, input: Chunk, out: &mut Vec<Mono>) -> Result<()> {
        let mono = downmix(&input)?;
        match self.input_rate {
            None => {
                self.input_rate = Some(input.rate);
                if input.rate != self.rate {
                    self.resampling = Some(Resampling::new(input.rate, self.rate)?);
                }
            }
            Some(rate) if rate != input.rate => {
                return Err(Error::Invalid(format!(
                    "the sample rate changed from {rate} Hz to {} Hz mid-stream",
                    input.rate
                )));
            }
            Some(_) => {}
        }

        match &mut self.resampling {
            None => out.push(Mono {
                rate: self.rate,
                start_frame: input.start_frame,
                samples: mono,
            }),
            Some(resampling) => {
                let samples = resampling.push(&mono)?;
                resampling.emit(samples, self.rate, out);
            }
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<Mono>) -> Result<()> {
        if let Some(resampling) = &mut self.resampling {
            let samples = resampling.finish()?;
            resampling.emit(samples, self.rate, out);
        }
        Ok(())
    }

    fn latency_frames(&self) -> u64 {
        self.resampling.as_ref().map_or(0, Resampling::latency)
    }
}

/// The mean of each frame's channels.
fn downmix(chunk: &Chunk) -> Result<Vec<f32>> {
    let channels = usize::from(chunk.channels);
    if channels == 0 {
        return Err(Error::Invalid("a chunk declares 0 channels".to_owned()));
    }
    if !chunk.samples.len().is_multiple_of(channels) {
        return Err(Error::Invalid(format!(
            "a chunk of {} samples is not a whole number of {channels}-channel frames",
            chunk.samples.len()
        )));
    }
    Ok(chunk
        .samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect())
}

/// A mono stream at one rate becoming a mono stream at another, carried across pushes.
struct Resampling {
    fft: Fft<f32>,
    from: u32,
    to: u32,
    /// Input frames waiting for a full step.
    pending: Vec<f32>,
    input_frames: u64,
    /// Output frames still to drop: the filter's delay, which would start the stream late.
    to_skip: usize,
    /// Output frames returned so far.
    emitted: u64,
}

impl std::fmt::Debug for Resampling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resampling")
            .field("from", &self.from)
            .field("to", &self.to)
            .finish_non_exhaustive()
    }
}

impl Resampling {
    fn new(from: u32, to: u32) -> Result<Resampling> {
        let fft = Fft::new(
            from as usize,
            to as usize,
            RESAMPLE_CHUNK,
            1,
            FixedSync::Input,
        )
        .map_err(|e| Error::Invalid(format!("cannot resample {from} Hz to {to} Hz: {e}")))?;
        let to_skip = fft.output_delay();
        Ok(Resampling {
            fft,
            from,
            to,
            pending: Vec::new(),
            input_frames: 0,
            to_skip,
            emitted: 0,
        })
    }

    /// Takes `mono` and returns the output frames that are now settled.
    fn push(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
        self.input_frames += mono.len() as u64;
        self.pending.extend_from_slice(mono);
        let mut produced = Vec::new();
        while self.pending.len() >= self.fft.input_frames_next() {
            self.step(&mut produced, None)?;
        }
        Ok(produced)
    }

    /// Flushes the filter with silence, and returns exactly the frames the input is worth.
    fn finish(&mut self) -> Result<Vec<f32>> {
        let wanted = (self.input_frames * u64::from(self.to) + u64::from(self.from) / 2)
            / u64::from(self.from);
        let mut produced = Vec::new();
        while self.emitted + (produced.len() as u64) < wanted {
            let held = self.pending.len();
            self.step(&mut produced, Some(held))?;
        }
        let keep = usize::try_from(wanted.saturating_sub(self.emitted)).unwrap_or(usize::MAX);
        produced.truncate(keep);
        Ok(produced)
    }

    /// One resampler step. `partial` is the frames of real input when fewer than a full step
    /// remain; the rest is silence.
    fn step(&mut self, produced: &mut Vec<f32>, partial: Option<usize>) -> Result<()> {
        let wanted = self.fft.input_frames_next();
        let real = partial.unwrap_or(wanted);
        let mut input: Vec<f32> = self.pending.drain(..real).collect();
        input.resize(wanted, 0.0);

        let capacity = self.fft.output_frames_max();
        let mut output = vec![0.0f32; capacity];
        let failed = |e: &dyn std::fmt::Display| Error::Invalid(format!("resampling failed: {e}"));
        let input_adapter = InterleavedSlice::new(&input, 1, wanted).map_err(|e| failed(&e))?;
        let mut output_adapter =
            InterleavedSlice::new_mut(&mut output, 1, capacity).map_err(|e| failed(&e))?;
        let indexing = Indexing {
            partial_len: partial,
            ..Indexing::default()
        };
        let (_, written) = self
            .fft
            .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
            .map_err(|e| failed(&e))?;

        let skipped = self.to_skip.min(written);
        self.to_skip -= skipped;
        produced.extend_from_slice(&output[skipped..written]);
        Ok(())
    }

    fn emit(&mut self, samples: Vec<f32>, rate: u32, out: &mut Vec<Mono>) {
        if samples.is_empty() {
            return;
        }
        let start_frame = self.emitted;
        self.emitted += samples.len() as u64;
        out.push(Mono {
            rate,
            start_frame,
            samples,
        });
    }

    /// Output frames held back: the filter's delay plus a step of input still to arrive.
    fn latency(&self) -> u64 {
        let step = (self.fft.input_frames_next() as u64 * u64::from(self.to))
            .div_ceil(u64::from(self.from));
        self.fft.output_delay() as u64 + step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(rate: u32, channels: u16, start_frame: u64, samples: Vec<f32>) -> Chunk {
        Chunk {
            rate,
            channels,
            start_frame,
            samples,
        }
    }

    fn run(prepare: &mut Prepare, chunks: Vec<Chunk>) -> Vec<Mono> {
        let mut out = Vec::new();
        for c in chunks {
            prepare.push(c, &mut out).unwrap();
        }
        prepare.finish(&mut out).unwrap();
        out
    }

    #[test]
    fn a_chunk_that_is_not_whole_frames_is_refused_naming_both_numbers() {
        let mut prepare = Prepare::new(22_050);
        let err = prepare
            .push(chunk(44_100, 2, 0, vec![0.0; 7]), &mut Vec::new())
            .unwrap_err();
        let Error::Invalid(message) = err else {
            panic!("expected Invalid, got {err:?}");
        };
        assert!(message.contains('7') && message.contains('2'), "{message}");
    }

    #[test]
    fn a_chunk_with_no_channels_is_refused() {
        let mut prepare = Prepare::new(22_050);
        let err = prepare
            .push(chunk(44_100, 0, 0, vec![0.0; 4]), &mut Vec::new())
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    }

    #[test]
    fn channels_are_averaged_and_a_matching_rate_passes_through() {
        let mut prepare = Prepare::new(44_100);
        let out = run(
            &mut prepare,
            vec![chunk(44_100, 2, 10, vec![1.0, 0.0, 0.5, -0.5, -1.0, -1.0])],
        );
        assert_eq!(
            out,
            vec![Mono {
                rate: 44_100,
                start_frame: 10,
                samples: vec![0.5, 0.0, -1.0],
            }]
        );
        assert_eq!(prepare.latency_frames(), 0);
    }

    #[test]
    fn resampling_keeps_the_duration_across_awkward_chunk_sizes() {
        let mut prepare = Prepare::new(22_050);
        let mut start = 0;
        let mut chunks = Vec::new();
        for frames in [1000, 1, 4096, 777, 2500] {
            chunks.push(chunk(44_100, 1, start, vec![0.25; frames]));
            start += frames as u64;
        }
        let out = run(&mut prepare, chunks);
        let total: usize = out.iter().map(|m| m.samples.len()).sum();
        assert_eq!(total, 4187); // 8374 frames at half the rate
        assert!(out.iter().all(|m| m.rate == 22_050));
    }

    #[test]
    fn output_start_frames_follow_on_from_each_other() {
        let mut prepare = Prepare::new(22_050);
        let chunks = (0..8)
            .map(|i| chunk(48_000, 1, i * 1500, vec![0.1; 1500]))
            .collect();
        let out = run(&mut prepare, chunks);
        let mut expected = 0;
        for m in &out {
            assert_eq!(m.start_frame, expected);
            expected += m.samples.len() as u64;
        }
        assert_eq!(expected, 5513); // 12000 frames at 48 kHz, to 22.05 kHz, rounded
    }

    #[test]
    fn a_resampling_stage_reports_its_hold_back() {
        let mut prepare = Prepare::new(22_050);
        prepare
            .push(chunk(44_100, 1, 0, vec![0.0; 10]), &mut Vec::new())
            .unwrap();
        assert!(prepare.latency_frames() > 0);
    }

    #[test]
    fn the_rate_cannot_change_mid_stream() {
        let mut prepare = Prepare::new(22_050);
        prepare
            .push(chunk(44_100, 1, 0, vec![0.0; 100]), &mut Vec::new())
            .unwrap();
        let err = prepare
            .push(chunk(48_000, 1, 100, vec![0.0; 100]), &mut Vec::new())
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    }
}
