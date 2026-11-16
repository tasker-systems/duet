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

use flacenc::bitsink::MemSink;
use flacenc::component::{BitRepr, Stream, StreamInfo};
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
    /// Samples that reached full scale or beyond, and were stored at full scale: a take
    /// with any may be clipped.
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
        let config = config.into_verified().map_err(|(_, e)| encoding(&e))?;
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
            if sample.abs() >= 1.0 {
                self.full_scale += 1;
            }
            // NaN, which no device should send, is stored as silence.
            let step = if sample.is_nan() {
                0.0
            } else {
                (sample * steps).round().clamp(lowest, highest)
            };
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
        let frame = flacenc::encode_fixed_size_frame(
            &self.config,
            &self.framebuf,
            self.flac_frames,
            &self.info,
        )
        .map_err(|e| failed(&e))?;
        let bytes = bytes_of(&frame).map_err(|e| write_failed(&self.path, e))?;
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
