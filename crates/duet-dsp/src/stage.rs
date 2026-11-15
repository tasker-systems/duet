use crate::error::Result;

/// A block of interleaved samples, as a source of audio yields it: [`Prepare`](crate::Prepare)'s
/// input.
///
/// Deliberately exhaustive, so other crates can build one in their own source.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Sample rate in Hz.
    pub rate: u32,
    pub channels: u16,
    /// Frames before this chunk, counted at `rate`.
    pub start_frame: u64,
    /// Interleaved: `channels` samples per frame.
    pub samples: Vec<f32>,
}

/// One channel at the analysis rate: what every analysis stage receives.
///
/// Deliberately exhaustive, so other crates can build one for their own stages.
#[derive(Debug, Clone, PartialEq)]
pub struct Mono {
    pub rate: u32,
    /// Frames before this chunk, counted at `rate`.
    pub start_frame: u64,
    pub samples: Vec<f32>,
}

/// One step of the chain. It may hold input back until its output is settled.
pub trait Stage {
    type In;
    type Out;

    /// Takes one input and appends whatever has settled to `out`.
    fn push(&mut self, input: Self::In, out: &mut Vec<Self::Out>) -> Result<()>;

    /// The stream has ended: appends everything still held back to `out`.
    fn finish(&mut self, out: &mut Vec<Self::Out>) -> Result<()>;

    /// The most frames this stage holds output back, so a live consumer knows how far
    /// behind real time its events run.
    fn latency_frames(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Emits each input one push late.
    struct OnePushLate {
        held: Option<u32>,
        delay: u64,
    }

    impl Stage for OnePushLate {
        type In = u32;
        type Out = u32;

        fn push(&mut self, input: u32, out: &mut Vec<u32>) -> Result<()> {
            out.extend(self.held.replace(input));
            Ok(())
        }

        fn finish(&mut self, out: &mut Vec<u32>) -> Result<()> {
            out.extend(self.held.take());
            Ok(())
        }

        fn latency_frames(&self) -> u64 {
            self.delay
        }
    }

    #[test]
    fn a_stage_that_holds_input_flushes_it_on_finish() {
        let mut stage = OnePushLate {
            held: None,
            delay: 512,
        };
        let mut out = Vec::new();
        stage.push(1, &mut out).unwrap();
        assert_eq!(out, Vec::<u32>::new());
        stage.push(2, &mut out).unwrap();
        stage.push(3, &mut out).unwrap();
        assert_eq!(out, vec![1, 2]);
        stage.finish(&mut out).unwrap();
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn latency_is_reported() {
        let stage = OnePushLate {
            held: None,
            delay: 512,
        };
        assert_eq!(stage.latency_frames(), 512);
    }
}
