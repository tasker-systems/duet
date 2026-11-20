//! Writing a take: interleaved samples in, a FLAC file out, a frame at a time.
//!
//! [`TakeWriter`] is what `duet record` keeps a take with. FLAC is lossless, so a take read
//! back by [`FileSource`](crate::FileSource) holds the samples that were written, up to the
//! [`Depth`] they were stored at, and a later reader of the take reads what was sung.
//!
//! The file is written as the samples arrive, never held whole: the header goes first with
//! the length and checksum unknown (the format allows both), each FLAC frame follows as soon
//! as it fills, and [`TakeWriter::finish`] goes back and fills the header in. A writer that
//! never finishes, because the process was killed mid-take, still leaves a file a decoder
//! reads, missing at most the last, unfilled frame.

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use flacenc::bitsink::{BitSink, MemSink};
use flacenc::component::{BitRepr, Frame, QuantizedParameters, Stream, StreamInfo, SubFrame};
use flacenc::config;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Context, Fill, FrameBuf};

use crate::error::{Error, Result};

/// Samples per channel in each FLAC frame: the encoder's default, and the reference
/// encoder's. The header always states it as both the smallest and the largest block, since
/// only the last frame may be shorter, and FLAC refuses a stated block under 16.
const BLOCK_SIZE: usize = 4096;

/// The bit depth a take is stored at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Depth {
    /// 16 bits: all a 16-bit device delivers.
    Sixteen,
    /// 24 bits: FLAC's deepest, and as much as a float sample's mantissa holds.
    TwentyFour,
}

impl Depth {
    /// Bits per sample.
    pub const fn bits(self) -> u32 {
        match self {
            Depth::Sixteen => 16,
            Depth::TwentyFour => 24,
        }
    }
}

/// What [`TakeWriter::finish`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Written {
    /// The file.
    pub path: PathBuf,
    /// Frames (one sample per channel) in the file, silence written for lost input included.
    pub frames: u64,
    /// Sample rate in Hz.
    pub rate: u32,
    pub channels: u16,
    pub depth: Depth,
    /// Samples stored at full scale, the top or bottom step, whether they reached it or went
    /// beyond: a take with any may be clipped.
    pub full_scale: u64,
}

/// A FLAC file being written from interleaved `f32` samples, as they arrive.
///
/// A sample is stored as the nearest step at its [`Depth`], where -1.0 and 1.0 are full
/// scale; one beyond full scale is stored at full scale and counted in
/// [`Written::full_scale`]. Nothing else is done to a sample: no resampling, mixing or gain.
pub struct TakeWriter {
    path: PathBuf,
    file: File,
    config: Verified<config::Encoder>,
    /// An encoder that stores each channel's samples as they are: what `own_frame` codes.
    verbatim: Verified<config::Encoder>,
    info: StreamInfo,
    rate: u32,
    channels: u16,
    depth: Depth,
    /// Interleaved samples waiting for their frame to fill.
    pending: Vec<i32>,
    framebuf: FrameBuf,
    context: Context,
    /// FLAC frames written.
    flac_frames: usize,
    /// The smallest and largest FLAC frame written, in bytes.
    frame_bytes: Option<(usize, usize)>,
    full_scale: u64,
}

impl std::fmt::Debug for TakeWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TakeWriter")
            .field("path", &self.path)
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .field("depth", &self.depth)
            .field("frames", &self.frames())
            .finish_non_exhaustive()
    }
}

impl TakeWriter {
    /// Creates `path` and writes the header of a FLAC stream with the given format.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when `path` already exists, since a take never replaces a file, or
    /// when FLAC cannot hold the format (a rate above 655,350 Hz, or more than 8 channels).
    /// [`Error::Write`] when the file cannot be created or written.
    pub fn create(path: &Path, rate: u32, channels: u16, depth: Depth) -> Result<TakeWriter> {
        let encoding = |e: &dyn std::fmt::Display| {
            Error::Invalid(format!(
                "FLAC cannot hold {channels} channels at {rate} Hz for {}: {e}",
                path.display()
            ))
        };
        if channels == 0 || channels > 8 {
            return Err(encoding(&"FLAC holds 1 to 8 channels"));
        }
        let width = usize::from(channels);
        let bits = depth.bits() as usize;
        let mut info = StreamInfo::new(rate as usize, width, bits).map_err(|e| encoding(&e))?;
        info.set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
            .map_err(|e| encoding(&e))?;
        // Zero frame sizes, length and checksum are "unknown": what an unfinished file says.
        info.set_frame_sizes(0, 0).map_err(|e| encoding(&e))?;
        let mut config = config::Encoder::default();
        config.block_size = BLOCK_SIZE;
        let mut verbatim = config.clone();
        verbatim.subframe_coding.use_fixed = false;
        verbatim.subframe_coding.use_lpc = false;
        verbatim.stereo_coding.use_leftside = false;
        verbatim.stereo_coding.use_rightside = false;
        verbatim.stereo_coding.use_midside = false;
        let config = config.into_verified().map_err(|(_, e)| encoding(&e))?;
        let verbatim = verbatim.into_verified().map_err(|(_, e)| encoding(&e))?;
        let framebuf = FrameBuf::with_size(width, BLOCK_SIZE).map_err(|e| encoding(&e))?;

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    Error::Invalid(format!(
                        "{} exists; a take never replaces a file, so move it or name another",
                        path.display()
                    ))
                } else {
                    write_failed(path, e)
                }
            })?;
        let head = header(&info).map_err(|e| write_failed(path, e))?;
        file.write_all(&head).map_err(|e| write_failed(path, e))?;
        Ok(TakeWriter {
            path: path.to_path_buf(),
            file,
            config,
            verbatim,
            info,
            rate,
            channels,
            depth,
            pending: Vec::with_capacity(BLOCK_SIZE * width),
            framebuf,
            context: Context::new(bits, width),
            flac_frames: 0,
            frame_bytes: None,
            full_scale: 0,
        })
    }

    /// Channels in each frame.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Frames (one sample per channel) taken so far, written or waiting for their FLAC
    /// frame to fill.
    pub fn frames(&self) -> u64 {
        (self.context.total_samples() + self.pending.len() / usize::from(self.channels)) as u64
    }

    /// Adds interleaved samples, `channels` to a frame.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when `samples` is not a whole number of frames; [`Error::Write`]
    /// when a filled frame cannot be encoded or written.
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        if !samples.len().is_multiple_of(usize::from(self.channels)) {
            return Err(Error::Invalid(format!(
                "{} samples are not a whole number of {}-channel frames",
                samples.len(),
                self.channels
            )));
        }
        let steps = (1i64 << (self.depth.bits() - 1)) as f32;
        let (lowest, highest) = (-steps, steps - 1.0);
        for &sample in samples {
            // NaN, which no device should send, is stored as silence.
            let step = if sample.is_nan() {
                0.0
            } else {
                (sample * steps).round().clamp(lowest, highest)
            };
            // Counted by the step stored, not the sample: an integer device's top step
            // arrives just under 1.0, as 32767 / 32768 does.
            if step <= lowest || step >= highest {
                self.full_scale += 1;
            }
            self.push(step as i32)?;
        }
        Ok(())
    }

    /// Adds `frames` of silence: what stands in for input that was lost, so what follows
    /// keeps its true time.
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when a filled frame cannot be encoded or written.
    pub fn silence(&mut self, frames: u64) -> Result<()> {
        for _ in 0..frames * u64::from(self.channels) {
            self.push(0)?;
        }
        Ok(())
    }

    /// Writes what is waiting, fills the header in, and closes the file.
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when the last frame or the header cannot be written.
    pub fn finish(mut self) -> Result<Written> {
        if !self.pending.is_empty() {
            self.encode()?;
        }
        let path = self.path.clone();
        let failed = |e: &dyn std::fmt::Display| Error::Write {
            path: path.clone(),
            source: io::Error::other(e.to_string()),
        };
        if let Some((smallest, largest)) = self.frame_bytes {
            self.info
                .set_frame_sizes(smallest, largest)
                .map_err(|e| failed(&e))?;
        }
        self.info.set_total_samples(self.context.total_samples());
        self.info.set_md5_digest(&self.context.md5_digest());
        let written = (|| -> io::Result<()> {
            self.file.seek(SeekFrom::Start(0))?;
            self.file.write_all(&header(&self.info)?)?;
            self.file.sync_all()
        })();
        written.map_err(|e| write_failed(&self.path, e))?;
        Ok(Written {
            path: self.path,
            frames: self.context.total_samples() as u64,
            rate: self.rate,
            channels: self.channels,
            depth: self.depth,
            full_scale: self.full_scale,
        })
    }

    fn push(&mut self, step: i32) -> Result<()> {
        self.pending.push(step);
        if self.pending.len() == BLOCK_SIZE * usize::from(self.channels) {
            self.encode()?;
        }
        Ok(())
    }

    /// Encodes and writes what is waiting as one FLAC frame.
    fn encode(&mut self) -> Result<()> {
        let path = self.path.clone();
        let failed = |e: &dyn std::fmt::Display| Error::Write {
            path: path.clone(),
            source: io::Error::other(format!("encoding FLAC: {e}")),
        };
        (&mut self.framebuf, &mut self.context)
            .fill_interleaved(&self.pending)
            .map_err(|e| failed(&e))?;
        let encode = |config: &Verified<config::Encoder>| {
            flacenc::encode_fixed_size_frame(config, &self.framebuf, self.flac_frames, &self.info)
                .map_err(|e| failed(&e))
        };
        let block = self.pending.len() / usize::from(self.channels);
        // flacenc's frame, the same re-coded, and duet's own: flacenc's is written unless one
        // of the others is smaller (`recoded_frame` and `own_frame` say why). flacenc's bit
        // count is exact even when its frame is ruinous, so a ruinous frame is never written
        // out to be measured.
        let theirs = encode(&self.config)?;
        let recoded = recoded_frame(&theirs, block).map_err(|e| write_failed(&self.path, e))?;
        let ours =
            own_frame(&encode(&self.verbatim)?, block).map_err(|e| write_failed(&self.path, e))?;
        let smaller = if ours.len() < recoded.len() {
            ours
        } else {
            recoded
        };
        let bytes = if smaller.len() * 8 < theirs.count_bits() {
            smaller
        } else {
            bytes_of(&theirs).map_err(|e| write_failed(&self.path, e))?
        };
        self.file
            .write_all(&bytes)
            .map_err(|e| write_failed(&self.path, e))?;
        self.frame_bytes = Some(match self.frame_bytes {
            None => (bytes.len(), bytes.len()),
            Some((smallest, largest)) => (smallest.min(bytes.len()), largest.max(bytes.len())),
        });
        self.flac_frames += 1;
        self.pending.clear();
        Ok(())
    }
}

/// The largest Rice partition order searched: the FLAC subset's limit, which every decoder
/// handles.
const MAX_PARTITION_ORDER: usize = 8;

/// FLAC's fixed predictors, of orders 0 to 4.
const FIXED_ORDERS: usize = 5;

/// Rice parameters 0 to 30: RICE2's range, of which RICE's 4-bit form takes 0 to 14. The
/// field's top value in each form escapes the partition, which is then stored as it is.
const RICE_PARAMETERS: usize = 31;

/// `frame`, flacenc's, as bytes with each predicted subframe's residual re-coded by
/// [`cheapest`]: flacenc's predictors, without its Rice coding.
///
/// flacenc's own Rice coding cannot be trusted to be small. Its parameter search (flacenc
/// 0.5.1, `rice.rs`) keeps each parameter's cost as a `u32` clamped at 2^28 - 1, adds two such
/// costs when it merges partitions without clamping again, then shifts the sum left by 4 to
/// pack it beside the parameter's index. A partition whose cost passed 2^28, as a clipped
/// full-scale 24-bit block's does, wraps round to look nearly free, and wins: one 4,096-sample
/// block of a take was written as a 100 MB frame, parameter 0 for residuals near 2^24, each a
/// unary code that long. The frame was valid FLAC, so nothing failed. And flacenc writes only
/// RICE's 4-bit parameters, with no escape, so a partition of residuals near 2^25 costs some
/// 2^11 bits a sample even when its parameter is right. Neither is configurable, so the
/// residual is coded here, as libFLAC codes it.
fn recoded_frame(frame: &Frame, block: usize) -> io::Result<Vec<u8>> {
    frame_bytes(frame, |subframe| match subframe {
        SubFrame::FixedLpc(fixed) => Predicted::of(
            Predictor::Fixed,
            fixed.warm_up(),
            fixed.bits_per_sample(),
            block,
            |t| fixed.residual().residual(t),
        ),
        SubFrame::Lpc(lpc) => Predicted::of(
            Predictor::Lpc(lpc.parameters()),
            lpc.warm_up(),
            lpc.bits_per_sample(),
            block,
            |t| lpc.residual().residual(t),
        ),
        _ => None,
    })
}

/// A frame of duet's own for `verbatim`, flacenc's frame of each channel's samples as they
/// are: each channel the cheapest of its samples as they are and FLAC's fixed predictors,
/// coded by [`cheapest`].
///
/// flacenc chooses its predictors by Rice costs from the search [`recoded_frame`] describes,
/// so a block that breaks the search can lose its predictor as well as its coding. The fixed
/// predictors are libFLAC's own first resort.
fn own_frame(verbatim: &Frame, block: usize) -> io::Result<Vec<u8>> {
    frame_bytes(verbatim, |subframe| match subframe {
        SubFrame::Verbatim(stored) => fixed(stored.samples(), block, stored.bits_per_sample()),
        _ => None,
    })
}

/// `frame` as bytes, each subframe written as `predict` codes it, or as flacenc does where it
/// gives `None`.
fn frame_bytes<'a>(
    frame: &'a Frame,
    predict: impl Fn(&'a SubFrame) -> Option<Predicted<'a>>,
) -> io::Result<Vec<u8>> {
    let mut sink = MemSink::<u8>::with_capacity(frame.count_bits());
    let sunk = |e: &dyn std::fmt::Display| io::Error::other(format!("encoding FLAC: {e}"));
    frame.header().write(&mut sink).map_err(|e| sunk(&e))?;
    for subframe in (0..frame.subframe_count()).filter_map(|ch| frame.subframe(ch)) {
        match predict(subframe) {
            Some(predicted) => predicted.write(&mut sink),
            None => subframe.write(&mut sink).map_err(|e| sunk(&e))?,
        }
    }
    sink.align_to_byte().map_err(|e| sunk(&e))?;
    let mut bytes = sink.into_inner();
    let crc = crc16(&bytes);
    bytes.extend(crc.to_be_bytes());
    Ok(bytes)
}

/// A subframe's predictor.
enum Predictor<'a> {
    /// FLAC's fixed predictor of the warm-up's length.
    Fixed,
    /// A linear predictor with these quantized coefficients.
    Lpc(&'a QuantizedParameters),
}

/// A predicted subframe: its predictor, its warm-up samples as they are, then what the
/// predictor leaves of the rest, coded by [`cheapest`].
struct Predicted<'a> {
    predictor: Predictor<'a>,
    warm_up: &'a [i32],
    bits: usize,
    residuals: Vec<i32>,
    coding: Coding,
}

impl<'a> Predicted<'a> {
    /// The subframe of a `block`-sample residual, `residual(t)` for each sample after the
    /// warm-up, coded by [`cheapest`]; `None` when no coding fits it.
    fn of(
        predictor: Predictor<'a>,
        warm_up: &'a [i32],
        bits: usize,
        block: usize,
        residual: impl Fn(usize) -> i32,
    ) -> Option<Predicted<'a>> {
        let order = warm_up.len();
        let residuals: Vec<i32> = (0..block)
            .map(|t| if t < order { 0 } else { residual(t) })
            .collect();
        // Folding for Rice coding doubles a residual: it must stay within 32 bits.
        if residuals.iter().any(|r| r.unsigned_abs() >= 1 << 30) {
            return None;
        }
        let coding = cheapest(&residuals, order)?;
        Some(Predicted {
            predictor,
            warm_up,
            bits,
            residuals,
            coding,
        })
    }

    /// The bits the subframe takes.
    fn cost(&self) -> u64 {
        let head = match self.predictor {
            Predictor::Fixed => 0,
            // The precision, the shift, then each coefficient.
            Predictor::Lpc(parameters) => 4 + 5 + parameters.precision() * parameters.order(),
        };
        (8 + head + self.warm_up.len() * self.bits) as u64 + self.coding.bits
    }

    fn write(&self, sink: &mut MemSink<u8>) {
        let order = self.warm_up.len();
        // Zero padding, the type with the order, no wasted bits.
        match self.predictor {
            Predictor::Fixed => put(sink, (0b00_1000 | order as u64) << 1, 8),
            Predictor::Lpc(_) => put(sink, (0b10_0000 | (order as u64 - 1)) << 1, 8),
        }
        for &sample in self.warm_up {
            put(sink, signed(sample, self.bits), self.bits);
        }
        if let Predictor::Lpc(parameters) = self.predictor {
            let precision = parameters.precision();
            put(sink, precision as u64 - 1, 4);
            put(sink, signed(i32::from(parameters.shift()), 5), 5);
            for coefficient in (0..order).filter_map(|i| parameters.coefficient(i)) {
                put(sink, signed(i32::from(coefficient), precision), precision);
            }
        }
        let coding = &self.coding;
        let width = if coding.rice2 { 5 } else { 4 };
        put(sink, u64::from(coding.rice2), 2);
        put(sink, coding.order as u64, 4);
        let len = self.residuals.len() >> coding.order;
        for (part, partition) in coding.partitions.iter().enumerate() {
            let values = &self.residuals[(part * len).max(order)..(part + 1) * len];
            match *partition {
                Partition::Rice(p) => {
                    put(sink, p as u64, width);
                    for &value in values {
                        let folded = fold(value);
                        let mut quotient = (folded >> p) as usize;
                        while quotient >= 32 {
                            put(sink, 0, 32);
                            quotient -= 32;
                        }
                        // The quotient's zeros and the one that ends them, then the remainder.
                        put(sink, 1, quotient + 1);
                        if p > 0 {
                            put(sink, u64::from(folded) & ((1 << p) - 1), p);
                        }
                    }
                }
                Partition::Escaped(n) => {
                    put(sink, (1 << width) - 1, width);
                    put(sink, n as u64, 5);
                    for &value in values {
                        put(sink, signed(value, n), n);
                    }
                }
            }
        }
    }
}

/// The cheapest fixed-predictor subframe of `samples` at `bits` per sample, when one costs
/// less than the samples as they are.
fn fixed(samples: &[i32], block: usize, bits: usize) -> Option<Predicted<'_>> {
    let verbatim = (8 + block * bits) as u64;
    (0..FIXED_ORDERS.min(block))
        .filter_map(|order| {
            let residuals = fixed_residuals(samples, order)?;
            Predicted::of(Predictor::Fixed, &samples[..order], bits, block, |t| {
                residuals[t]
            })
        })
        .filter(|predicted| predicted.cost() < verbatim)
        .min_by_key(Predicted::cost)
}

/// What FLAC's fixed predictor of `order` leaves of `samples`, zero for the warm-up; `None`
/// when a residual does not fit the 32 bits FLAC allows it.
fn fixed_residuals(samples: &[i32], order: usize) -> Option<Vec<i32>> {
    let mut residuals = vec![0; samples.len()];
    for t in order..samples.len() {
        let x = |back: usize| i64::from(samples[t - back]);
        let residual = match order {
            0 => x(0),
            1 => x(0) - x(1),
            2 => x(0) - 2 * x(1) + x(2),
            3 => x(0) - 3 * x(1) + 3 * x(2) - x(3),
            _ => x(0) - 4 * x(1) + 6 * x(2) - 4 * x(3) + x(4),
        };
        residuals[t] = i32::try_from(residual).ok()?;
    }
    Some(residuals)
}

/// How one partition of a residual is coded.
#[derive(Clone, Copy)]
enum Partition {
    /// Rice coded with this parameter.
    Rice(usize),
    /// Escaped: each residual stored as it is, in this many bits.
    Escaped(usize),
}

/// A residual's coding: RICE or RICE2, the partition order, each partition's coding, and the
/// bits it all takes.
struct Coding {
    rice2: bool,
    order: usize,
    partitions: Vec<Partition>,
    bits: u64,
}

/// What one partition costs: at each Rice parameter, and escaped.
#[derive(Clone, Copy)]
struct Costs {
    rice: [u64; RICE_PARAMETERS],
    /// The fewest bits that hold every residual of the partition, signed.
    width: usize,
    len: u64,
}

impl Costs {
    fn of(values: &[i32]) -> Costs {
        let mut rice = [0u64; RICE_PARAMETERS];
        for (p, cost) in rice.iter_mut().enumerate() {
            let quotients: u64 = values.iter().map(|&v| u64::from(fold(v) >> p)).sum();
            // Each residual: its quotient in unary, the bit that ends it, `p` remainder bits.
            *cost = quotients + values.len() as u64 * (p as u64 + 1);
        }
        // An escaped partition of zeros still takes one bit each: zero-bit samples are legal
        // FLAC, but not every decoder reads them.
        let width = values.iter().map(|&v| width(v)).max().unwrap_or(1).max(1);
        Costs {
            rice,
            width,
            len: values.len() as u64,
        }
    }

    fn merge(&self, other: &Costs) -> Costs {
        let mut rice = self.rice;
        rice.iter_mut().zip(&other.rice).for_each(|(a, b)| *a += b);
        Costs {
            rice,
            width: self.width.max(other.width),
            len: self.len + other.len,
        }
    }

    /// The cheapest coding with parameters below `escape`, the field's top value, and what it
    /// costs, the parameter field included.
    fn cheapest(&self, escape: usize, field: u64) -> (Partition, u64) {
        let (p, rice) = self.rice[..escape]
            .iter()
            .enumerate()
            .min_by_key(|&(_, bits)| *bits)
            .expect("there is always a parameter");
        let escaped = 5 + self.len * self.width as u64;
        if escaped < *rice && self.width < 32 {
            (Partition::Escaped(self.width), field + escaped)
        } else {
            (Partition::Rice(p), field + rice)
        }
    }
}

/// The cheapest coding of `residuals` after `warmup` warm-up samples, with every cost counted
/// in `u64` so none wraps; `None` when no partition order fits the block.
///
/// Each partition's costs are counted at the finest order, then pairs are summed for each
/// coarser one: exact, since a partition's cost is a sum over its samples.
fn cheapest(residuals: &[i32], warmup: usize) -> Option<Coding> {
    let block = residuals.len();
    // The finest order whose partitions are all one length, the first longer than the
    // warm-up, as FLAC requires.
    let finest = (0..=MAX_PARTITION_ORDER)
        .take_while(|&order| block.is_multiple_of(1 << order) && block >> order > warmup)
        .last()?;
    let len = block >> finest;
    let mut costs: Vec<Costs> = (0..1usize << finest)
        .map(|part| Costs::of(&residuals[(part * len).max(warmup)..(part + 1) * len]))
        .collect();

    let mut best: Option<Coding> = None;
    for order in (0..=finest).rev() {
        for (rice2, escape, field) in [(false, 15, 4), (true, 31, 5)] {
            // The method and the partition order, then each partition.
            let mut bits = 2 + 4;
            let mut partitions = Vec::with_capacity(costs.len());
            for partition in &costs {
                let (coding, cost) = partition.cheapest(escape, field);
                bits += cost;
                partitions.push(coding);
            }
            if best.as_ref().is_none_or(|coding| bits < coding.bits) {
                best = Some(Coding {
                    rice2,
                    order,
                    partitions,
                    bits,
                });
            }
        }
        costs = costs
            .chunks(2)
            .map(|pair| match pair {
                [first, second] => first.merge(second),
                [only] => *only,
                _ => unreachable!("chunks of two"),
            })
            .collect();
    }
    best
}

/// A residual with its sign folded into the lowest bit, as Rice coding takes it: 0, -1, 1,
/// -2, … as 0, 1, 2, 3, ….
fn fold(value: i32) -> u32 {
    (value.unsigned_abs() << 1) - u32::from(value < 0)
}

/// The fewest bits that hold `value` in two's complement.
fn width(value: i32) -> usize {
    let magnitude = if value < 0 { !value } else { value };
    33 - magnitude.leading_zeros() as usize
}

/// `value` as its low `bits` bits, two's complement.
fn signed(value: i32, bits: usize) -> u64 {
    u64::from(value as u32) & ((1u64 << bits) - 1)
}

/// Writes the low `bits` bits of `value`, 1 to 64 of them.
fn put(sink: &mut MemSink<u8>, value: u64, bits: usize) {
    // A `MemSink` grows to take what it is given: it never fails.
    let Ok(()) = sink.write_lsbs(value, bits);
}

/// FLAC's frame checksum: CRC-16, polynomial 0x8005, initial value zero, unreflected.
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |crc, &byte| {
        (0..8).fold(crc ^ (u16::from(byte) << 8), |crc, _| {
            if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x8005
            }
        })
    })
}

/// The stream marker and its one metadata block, `STREAMINFO`: always the same length, so
/// the finished header overwrites the first one exactly.
fn header(info: &StreamInfo) -> io::Result<Vec<u8>> {
    bytes_of(&Stream::with_stream_info(info.clone()))
}

/// A FLAC component as the bytes it is written as.
fn bytes_of(component: &impl BitRepr) -> io::Result<Vec<u8>> {
    let mut sink = MemSink::<u8>::with_capacity(component.count_bits());
    component
        .write(&mut sink)
        .map_err(|e| io::Error::other(format!("encoding FLAC: {e}")))?;
    Ok(sink.into_inner())
}

fn write_failed(path: &Path, source: io::Error) -> Error {
    Error::Write {
        path: path.to_path_buf(),
        source,
    }
}
