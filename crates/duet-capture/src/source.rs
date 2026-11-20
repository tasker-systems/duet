use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_AC3, CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS,
    CODEC_ID_ALAC, CODEC_ID_DCA, CODEC_ID_EAC3, CODEC_ID_MONKEYS_AUDIO, CODEC_ID_MP1, CODEC_ID_MP2,
    CODEC_ID_OPUS, CODEC_ID_SPEEX, CODEC_ID_TTA, CODEC_ID_WAVPACK, CODEC_ID_WMA,
};
use symphonia::core::codecs::audio::{
    AudioCodecId, AudioCodecParameters, AudioDecoder, AudioDecoderOptions,
};
use symphonia::core::errors::Error as DecodeFailure;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, RawValue};

use duet_dsp::Chunk;

use crate::error::{Error, Result};

/// Audio, a chunk at a time: what feeds [`Prepare`](crate::Prepare).
///
/// It lives here rather than in `duet-dsp`, beside its one implementation, because a source
/// is I/O: its errors are about reading and decoding, which the analysis knows nothing of.
pub trait Source {
    /// The next chunk, or `None` at the end of the stream.
    fn next_chunk(&mut self) -> Result<Option<Chunk>>;
}

/// A file of audio, read one packet at a time through Symphonia.
///
/// WAV, FLAC, MP3, Ogg Vorbis and AAC-LC in M4A are decoded; the file is never held in
/// memory whole. Anything else is refused by name with [`Error::Undecodable`].
///
/// A damaged packet, whether the reader skips it or the decoder refuses it, is left out, and
/// its frames are left as a gap in the chunks' start frames, so what follows keeps its true
/// time. [`Prepare`](crate::Prepare) fills the gap with silence. Such a frame can also be one
/// nothing is wrong with except its size: Symphonia's FLAC reader scans at most 16 MB for a
/// frame's end, so a valid frame longer than that is skipped with only a `log` warning to say
/// so. Every gap left, by either path, is stated in [`gaps`](FileSource::gaps).
///
/// HE-AAC is refused when the file's header declares it (audio object type 5 or 29, which
/// is how afconvert and most encoders write it). AAC in M4A is trimmed to its true length by
/// the file's `iTunSMPB` tag (Symphonia does not, for MP4): without it a decode starts with
/// the encoder's priming. HE-AAC signalled only inside the audio data
/// decodes as AAC-LC at half bandwidth without a word: telling those apart would mean
/// comparing the declared sample rate with the decoded one, which this does not do.
pub struct FileSource {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// Frames already yielded.
    next_frame: u64,
    /// Where the last packet ended, in the track's time base (frames), to notice packets the
    /// container reader skipped.
    packet_end: Option<i64>,
    /// Whether a packet has yielded frames yet. A gap is only counted after one has: packets
    /// before the first frames (Vorbis's first packet yields none) are not a skip.
    started: bool,
    gapless: Option<Gapless>,
    /// Every gap left so far, in the order it was left.
    gaps: Vec<Gap>,
}

/// The most separate gaps a [`FileSource`] states before it refuses the file: a gap that
/// touches the one before it extends it, so this counts holes, not damaged packets.
pub const MAX_GAPS: usize = 512;

/// A span of frames the file's reader left out, filled with silence downstream: where it
/// starts and how long it runs, counted in frames at the take's own rate.
///
/// A gap states the audio as the chunks number it: `start` is the frame number the missing
/// audio would have begun at, and `frames` its length, so what follows keeps its true time.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Gap {
    /// The frame number at which the missing audio starts, at the take's own rate.
    pub start: u64,
    /// How many frames are missing, read as silence.
    pub frames: u64,
}

impl Gap {
    /// A gap of `frames` frames starting at `start`, both at the take's own rate.
    #[must_use]
    pub const fn new(start: u64, frames: u64) -> Gap {
        Gap { start, frames }
    }
}

/// Encoder delay and padding still to cut from the stream, from an `iTunSMPB` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gapless {
    /// Frames to drop from the start.
    skip: u64,
    /// Frames to keep after that.
    keep: u64,
}

impl fmt::Debug for FileSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSource")
            .field("path", &self.path)
            .field("track_id", &self.track_id)
            .field("next_frame", &self.next_frame)
            .finish_non_exhaustive()
    }
}

impl FileSource {
    /// Opens `path` and finds its audio track.
    ///
    /// # Errors
    ///
    /// [`Error::Undecodable`] for a file that is not audio, holds HE-AAC, or uses a codec
    /// Symphonia lacks. [`Error::Invalid`] when the file cannot be read.
    pub fn open(path: &Path) -> Result<FileSource> {
        let undecodable = |codec: &str| Error::Undecodable {
            path: path.to_path_buf(),
            codec: codec.to_owned(),
        };

        let file = File::open(path)
            .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", path.display())))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }

        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| match e {
                DecodeFailure::IoError(e) => {
                    Error::Invalid(format!("cannot read {}: {e}", path.display()))
                }
                _ => undecodable("not a recognised audio format"),
            })?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| undecodable("no audio track"))?;
        let track_id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| undecodable("no audio track"))?
            .clone();
        if declares_he_aac(&params) {
            return Err(undecodable("HE-AAC"));
        }
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|_| undecodable(&codec_name(params.codec)))?;
        // Only AAC needs the tag: Symphonia's MP3 decoder trims by itself, and a second trim
        // from an ID3 copy of the tag would cut twice.
        let gapless = if params.codec == CODEC_ID_AAC {
            format.metadata().current().and_then(itunsmpb)
        } else {
            None
        };
        Ok(FileSource {
            path: path.to_path_buf(),
            format,
            decoder,
            track_id,
            next_frame: 0,
            packet_end: None,
            started: false,
            gapless,
            gaps: Vec::new(),
        })
    }

    /// Every gap left so far, in the order it was left: frames the file's reader skipped or
    /// whose packets the decoder refused, filled with silence downstream.
    ///
    /// Read it once the stream has reached its end: a gap is known when the chunk after it
    /// comes, or the stream does.
    pub fn gaps(&self) -> &[Gap] {
        &self.gaps
    }

    /// Leaves `frames` frames out at the current position, as one gap with the gap before it
    /// when the two touch, so a run of damaged packets is one gap however many packets it
    /// spans.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when this would be gap [`MAX_GAPS`] + 1: a file with that many
    /// separate holes is broken, not damaged, and counting them would let a file of tiny
    /// refused frames grow the list without bound.
    fn leave_gap(&mut self, frames: u64) -> Result<()> {
        let next = self.next_frame;
        if let Some(last) = self.gaps.last_mut()
            && last.start + last.frames == next
        {
            last.frames += frames;
        } else if self.gaps.len() == MAX_GAPS {
            return Err(Error::Invalid(format!(
                "{} leaves more than {MAX_GAPS} separate gaps where its packets could not be \
                 read; a file with that many holes is broken, not damaged",
                self.path.display()
            )));
        } else {
            self.gaps.push(Gap {
                start: next,
                frames,
            });
        }
        self.next_frame += frames;
        Ok(())
    }
}

impl Source for FileSource {
    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(e) => return Err(self.failed(&e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            // A reader that skips a damaged packet leaves a gap in the timestamps; count it, once
            // the stream has started.
            if self.started
                && let Some(end) = self.packet_end
            {
                let skipped = u64::try_from(packet.pts.get() - end).unwrap_or(0);
                if skipped > 0 {
                    self.leave_gap(skipped)?;
                }
            }
            self.packet_end = Some(packet.pts.get().saturating_add_unsigned(packet.dur.get()));
            let buffer = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                // A damaged packet is dropped and its frames still counted, so the start
                // frames of what follows stay true; the stream goes on.
                Err(DecodeFailure::DecodeError(_)) => {
                    self.leave_gap(packet.dur.get())?;
                    continue;
                }
                Err(e) => return Err(self.failed(&e)),
            };
            if buffer.frames() == 0 {
                continue;
            }

            let channels = u16::try_from(buffer.spec().channels().count()).map_err(|_| {
                Error::Invalid(format!("{} has too many channels", self.path.display()))
            })?;
            let mut samples = Vec::new();
            buffer.copy_to_vec_interleaved::<f32>(&mut samples);
            if let Some(gapless) = &mut self.gapless {
                gapless.trim(&mut samples, usize::from(channels));
            }
            if samples.is_empty() {
                if self.gapless.is_some_and(|g| g.keep == 0) {
                    return Ok(None);
                }
                continue;
            }
            let frames = (samples.len() / usize::from(channels)) as u64;
            let chunk = Chunk {
                rate: buffer.spec().rate(),
                channels,
                start_frame: self.next_frame,
                samples,
            };
            self.next_frame += frames;
            self.started = true;
            return Ok(Some(chunk));
        }
    }
}

impl Gapless {
    /// Cuts the delay off the front of the stream and the padding off the end.
    fn trim(&mut self, samples: &mut Vec<f32>, channels: usize) {
        let frames = (samples.len() / channels) as u64;
        let dropped = self.skip.min(frames);
        self.skip -= dropped;
        samples.drain(..dropped as usize * channels);
        let kept = self.keep.min(frames - dropped);
        self.keep -= kept;
        samples.truncate(kept as usize * channels);
    }
}

/// The `iTunSMPB` tag of a metadata revision, if it has one.
fn itunsmpb(revision: &MetadataRevision) -> Option<Gapless> {
    revision.media.tags.iter().find_map(|tag| {
        let RawValue::String(value) = &tag.raw.value else {
            return None;
        };
        if tag.raw.key.ends_with("iTunSMPB") {
            parse_itunsmpb(value)
        } else {
            None
        }
    })
}

/// A readable name for a codec Symphonia cannot decode, or its id when it has none.
fn codec_name(id: AudioCodecId) -> String {
    let named = [
        (CODEC_ID_OPUS, "Opus"),
        (CODEC_ID_SPEEX, "Speex"),
        (CODEC_ID_MP1, "MP1"),
        (CODEC_ID_MP2, "MP2"),
        (CODEC_ID_AC3, "AC-3"),
        (CODEC_ID_EAC3, "E-AC-3"),
        (CODEC_ID_DCA, "DTS"),
        (CODEC_ID_WMA, "WMA"),
        (CODEC_ID_ALAC, "ALAC"),
        (CODEC_ID_WAVPACK, "WavPack"),
        (CODEC_ID_MONKEYS_AUDIO, "Monkey's Audio"),
        (CODEC_ID_TTA, "TTA"),
        (CODEC_ID_ADPCM_MS, "ADPCM"),
        (CODEC_ID_ADPCM_IMA_WAV, "ADPCM"),
        (CODEC_ID_ADPCM_IMA_QT, "ADPCM"),
    ];
    match named.iter().find(|(known, _)| *known == id) {
        Some((_, name)) => (*name).to_owned(),
        None => format!("audio codec {id}"),
    }
}

/// The delay and valid length in an `iTunSMPB` tag: hexadecimal words after a leading zero
/// one, as ` 00000000 <delay> <padding> <valid frames> ...`.
fn parse_itunsmpb(tag: &str) -> Option<Gapless> {
    let mut words = tag.split_whitespace().skip(1);
    let mut next = || u64::from_str_radix(words.next()?, 16).ok();
    let skip = next()?;
    let _padding = next()?;
    let keep = next()?;
    Some(Gapless { skip, keep })
}

impl FileSource {
    fn failed(&self, e: &DecodeFailure) -> Error {
        Error::Invalid(format!("cannot read {}: {e}", self.path.display()))
    }
}

/// Whether the AAC header (the MPEG-4 AudioSpecificConfig, ISO/IEC 14496-3 1.6.2.1) declares
/// SBR: audio object type 5 or 29 up front, or the backward-compatible extension after an
/// AAC-LC header, which is how afconvert writes HE-AAC. Symphonia reports both as AAC-LC.
fn declares_he_aac(params: &AudioCodecParameters) -> bool {
    let Some(config) = params.extra_data.as_deref() else {
        return false;
    };
    let mut bits = Bits {
        data: config,
        at: 0,
    };
    let Some(object) = audio_object_type(&mut bits) else {
        return false;
    };
    if matches!(object, 5 | 29) {
        return true;
    }
    // Past the object type: sampling frequency, channel configuration, then for AAC-LC and
    // its kin the GASpecificConfig, then (if there is room) the extension.
    let mut declared_extension = || -> Option<bool> {
        if !(1..=4).contains(&object) {
            return None;
        }
        if bits.read(4)? == 15 {
            bits.read(24)?;
        }
        let channel_configuration = bits.read(4)?;
        bits.read(1)?; // frame length flag
        if bits.read(1)? == 1 {
            bits.read(14)?; // core coder delay
        }
        bits.read(1)?; // extension flag
        if channel_configuration == 0 {
            return None; // a program config element follows; not parsed
        }
        const SYNC_EXTENSION: u32 = 0x2b7;
        if bits.read(11)? != SYNC_EXTENSION {
            return None;
        }
        if audio_object_type(&mut bits)? != 5 {
            return None;
        }
        // sbrPresentFlag: 0 declares the extension without SBR, which is plain AAC-LC.
        Some(bits.read(1)? == 1)
    };
    declared_extension().unwrap_or(false)
}

fn audio_object_type(bits: &mut Bits<'_>) -> Option<u32> {
    match bits.read(5)? {
        31 => Some(32 + bits.read(6)?),
        object => Some(object),
    }
}

/// Reads bits, most significant first.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    /// The next `count` bits (at most 32) as a number, or `None` past the end.
    fn read(&mut self, count: usize) -> Option<u32> {
        if self.at + count > self.data.len() * 8 {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..count {
            let byte = self.data[self.at / 8];
            value = (value << 1) | u32::from((byte >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::common::FourCc;

    #[test]
    fn itunsmpb_gives_the_delay_and_the_valid_length() {
        let tag = " 00000000 00000840 0000037C 000000000000AC44 00000000 00000000";
        assert_eq!(
            parse_itunsmpb(tag),
            Some(Gapless {
                skip: 0x840,
                keep: 0xac44
            })
        );
        assert_eq!(parse_itunsmpb("not a tag"), None);
    }

    #[test]
    fn gapless_trims_across_chunk_boundaries() {
        let mut gapless = Gapless { skip: 3, keep: 4 };
        let mut all = Vec::new();
        for start in [0, 2, 4, 6] {
            // Two frames of two channels, each sample numbering its frame.
            let mut chunk: Vec<f32> = (start..start + 2).flat_map(|f| [f as f32; 2]).collect();
            gapless.trim(&mut chunk, 2);
            all.extend(chunk);
        }
        // Frames 3 to 6 survive; the last chunk (frames 6 and 7) is cut at the 4-frame mark.
        assert_eq!(all, vec![3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 6.0, 6.0]);
    }

    fn bits_of(config: &[u8]) -> Bits<'_> {
        Bits {
            data: config,
            at: 0,
        }
    }

    #[test]
    fn bits_read_most_significant_first_and_stop_at_the_end() {
        let mut bits = bits_of(&[0b1010_0000, 0xff]);
        assert_eq!(bits.read(3), Some(0b101));
        assert_eq!(bits.read(5), Some(0));
        assert_eq!(bits.read(8), Some(0xff));
        assert_eq!(bits.read(1), None);
    }

    fn params(extra_data: &[u8]) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params.with_extra_data(extra_data.to_vec().into_boxed_slice());
        params
    }

    #[test]
    fn plain_aac_lc_is_not_he_aac() {
        // object type 2, 48 kHz (index 3), stereo: what afconvert writes for -d aac.
        assert!(!declares_he_aac(&params(&[0x11, 0x90])));
    }

    #[test]
    fn sbr_declared_after_an_lc_header_is_he_aac() {
        // object type 2, 24 kHz, stereo, then the 0x2b7 extension naming object type 5.
        assert!(declares_he_aac(&params(&[0x13, 0x10, 0x56, 0xe5, 0x98])));
    }

    #[test]
    fn a_sync_extension_with_sbr_absent_is_plain_aac_lc() {
        // The same bytes with sbrPresentFlag cleared: the extension is there, SBR is not.
        assert!(!declares_he_aac(&params(&[0x13, 0x10, 0x56, 0xe5, 0x18])));
    }

    #[test]
    fn unsupported_codecs_are_named() {
        assert_eq!(codec_name(CODEC_ID_OPUS), "Opus");
        assert_eq!(codec_name(CODEC_ID_ALAC), "ALAC");
        assert!(codec_name(AudioCodecId::new(FourCc::new(*b"zzzz"))).starts_with("audio codec"));
    }

    #[test]
    fn sbr_declared_up_front_is_he_aac() {
        // object type 5 (00101), index 3, stereo.
        assert!(declares_he_aac(&params(&[0x29, 0x90])));
        // object type 29 (11101), index 3, stereo.
        assert!(declares_he_aac(&params(&[0xe9, 0x90])));
    }

    #[test]
    fn a_missing_or_truncated_header_is_not_he_aac() {
        assert!(!declares_he_aac(&AudioCodecParameters::new()));
        assert!(!declares_he_aac(&params(&[0x13])));
    }
}
