//! The take: which audio an analysis was read from, by a digest of its decoded samples.
//!
//! A reading names its take by [`Take`]: the SHA-256 of the samples as the source decodes them,
//! with their rate, channel count and length. A file's name is never authoritative, and its bytes
//! change with every tag a phone or a DAW rewrites, so the digest is over the samples (ruled by
//! the owner, 2026-11-17, question 1 of the variants-and-comps design).
//!
//! # What is hashed
//!
//! The ASCII bytes `duet-take-1`, then the rate as a little-endian `u32` and the channel count
//! as a little-endian `u16`, then every sample, interleaved and in order, as the little-endian
//! bits of its `f32`. It is written `sha256:` and 64 lowercase hex digits ([`TakeDigest`]).
//!
//! A gap in the chunks' start frames (a packet the source dropped as damaged) is hashed as the
//! silence [`Prepare`](crate::Prepare) fills it with, so `frames` is the take's length on its
//! timeline and every frame after a gap keeps its time.
//!
//! # What keeps it, and what does not
//!
//! - **Kept:** a FLAC re-encoded at another compression level or block size; tags or pictures
//!   added, changed or removed; FLAC to WAV and back at the same bit depth. The samples are the
//!   same, so their `f32`s are.
//! - **Kept, checked:** the same 16-bit samples re-encoded at 24 bits (each shifted up 8 bits).
//!   Symphonia converts an integer sample to `f32` by dividing by a power of two (2¹⁵ for 16
//!   bits, 2²³ for 24), which is exact for every sample of either depth, so a 16-bit sample `s`
//!   and its 24-bit `s·2⁸` give the same `f32`. duet-capture's `tests/take.rs` holds it, for WAV
//!   and FLAC. A 24-bit take reduced to 16 bits is other samples, and another take.
//! - **Not kept:** a lossy re-encode (MP3, AAC, Ogg), which makes other samples; a take trimmed,
//!   resampled, downmixed or with its gain changed, which is another take; and a lossy file read
//!   by a later decoder that rounds differently, a stated limit.
//!
//! # The stream rule
//!
//! [`Digester`] is fed every chunk the source yields, beside [`Prepare`](crate::Prepare), and
//! keeps only the hash's state: nothing of the take is held. A live take can be digested the
//! same way.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::prepare::MAX_GAP_SECS;
use crate::stage::Chunk;

/// What starts every hash: the digest's own name and version. A change to what is hashed is a
/// new name, and so a new digest for every take.
const DOMAIN: &[u8] = b"duet-take-1";
/// How a digest is written: this prefix, then 64 lowercase hex digits.
const PREFIX: &str = "sha256:";

/// The SHA-256 of a take's decoded samples, written `sha256:` and 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TakeDigest(pub [u8; 32]);

impl fmt::Display for TakeDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PREFIX)?;
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for TakeDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::str::FromStr for TakeDigest {
    type Err = Error;

    /// Reads `sha256:` and 64 lowercase hex digits, and nothing else: an uppercase digit is
    /// refused, so a digest has one spelling.
    fn from_str(text: &str) -> Result<TakeDigest> {
        let refused = || {
            Error::Invalid(format!(
                "{text:?} is not a take digest: one is {PREFIX} and 64 lowercase hex digits"
            ))
        };
        let hex = text.strip_prefix(PREFIX).ok_or_else(refused)?;
        if hex.len() != 64 {
            return Err(refused());
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut bytes = [0u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            *byte =
                (digit(pair[0]).ok_or_else(refused)? << 4) | digit(pair[1]).ok_or_else(refused)?;
        }
        Ok(TakeDigest(bytes))
    }
}

impl Serialize for TakeDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TakeDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for TakeDigest {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("TakeDigest")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("duet_dsp::TakeDigest")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^sha256:[0-9a-f]{64}$",
            "description": "The SHA-256 of a take's decoded samples: the bytes `duet-take-1`, the \
                rate as a little-endian u32, the channel count as a little-endian u16, then every \
                sample, interleaved, as the little-endian bits of its f32.",
        })
    }
}

/// The audio an analysis was read from: its samples' digest, and their rate, channel count and
/// length, as decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Take {
    /// The SHA-256 of the decoded samples ([`TakeDigest`]).
    pub digest: TakeDigest,
    /// The source's own sample rate, in Hz, before any resampling.
    #[schemars(range(min = 1))]
    pub rate: u32,
    /// The source's channel count, before any downmix.
    #[schemars(range(min = 1))]
    pub channels: u16,
    /// The take's length in frames (one sample per channel) at `rate`, a filled gap included.
    pub frames: u64,
}

impl Take {
    /// The first 12 hex digits of the digest: enough to tell takes apart in a message.
    #[must_use]
    pub fn short(&self) -> String {
        self.digest.to_string()[PREFIX.len()..PREFIX.len() + 12].to_owned()
    }
}

/// A take's [`Take`], computed a chunk at a time as the take is read.
#[derive(Clone, Default)]
pub struct Digester {
    /// The hash, and the take's rate and channels, once the first chunk has come.
    state: Option<(Sha256, u32, u16)>,
    /// The frame after the last one hashed.
    next: u64,
}

impl fmt::Debug for Digester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Digester")
            .field("format", &self.state.as_ref().map(|(_, r, c)| (*r, *c)))
            .field("frames", &self.next)
            .finish_non_exhaustive()
    }
}

impl Digester {
    /// A digester that has seen nothing.
    #[must_use]
    pub fn new() -> Digester {
        Digester::default()
    }

    /// Hashes `chunk`'s samples, after silence for any gap before it.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the chunk changes the rate or channel count, is not a whole number
    /// of frames, starts before the last one ended, or follows a gap longer than
    /// [`MAX_GAP_SECS`], each as [`Prepare`](crate::Prepare) refuses it.
    pub fn push(&mut self, chunk: &Chunk) -> Result<()> {
        let channels = usize::from(chunk.channels);
        if channels == 0 || !chunk.samples.len().is_multiple_of(channels) {
            return Err(Error::Invalid(format!(
                "{} samples are not a whole number of {}-channel frames",
                chunk.samples.len(),
                chunk.channels
            )));
        }
        let (hash, rate, held) = self.state.get_or_insert_with(|| {
            let mut hash = Sha256::new();
            hash.update(DOMAIN);
            hash.update(chunk.rate.to_le_bytes());
            hash.update(chunk.channels.to_le_bytes());
            (hash, chunk.rate, chunk.channels)
        });
        if (*rate, *held) != (chunk.rate, chunk.channels) {
            return Err(Error::Invalid(format!(
                "the take changed from {rate} Hz and {held} channels to {} Hz and {} mid-stream",
                chunk.rate, chunk.channels
            )));
        }
        let Some(gap) = chunk.start_frame.checked_sub(self.next) else {
            return Err(Error::Invalid(format!(
                "the audio goes back from frame {} to frame {}; a recording only runs forward",
                self.next, chunk.start_frame
            )));
        };
        if gap > u64::from(MAX_GAP_SECS) * u64::from(*rate) {
            return Err(Error::Invalid(format!(
                "the audio skips {gap} frames; a gap longer than {MAX_GAP_SECS} s is a broken \
                 file, not a damaged packet, so it is not filled"
            )));
        }
        let silence = 0.0f32.to_le_bytes();
        for _ in 0..gap * u64::from(*held) {
            hash.update(silence);
        }
        let mut bytes = Vec::with_capacity(chunk.samples.len() * 4);
        for sample in &chunk.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        hash.update(&bytes);
        self.next = chunk.start_frame + (chunk.samples.len() / channels) as u64;
        Ok(())
    }

    /// The take, or `None` when no chunk came: a take with no audio states none.
    #[must_use]
    pub fn finish(self) -> Option<Take> {
        let (hash, rate, channels) = self.state?;
        Some(Take {
            digest: TakeDigest(hash.finalize().into()),
            rate,
            channels,
            frames: self.next,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(start_frame: u64, channels: u16, samples: Vec<f32>) -> Chunk {
        Chunk {
            rate: 22_050,
            channels,
            start_frame,
            samples,
        }
    }

    fn digest_of(chunks: &[Chunk]) -> Take {
        let mut digester = Digester::new();
        for c in chunks {
            digester.push(c).unwrap();
        }
        digester.finish().unwrap()
    }

    #[test]
    fn the_digest_is_the_sha256_of_the_stated_bytes() {
        // Worked by hand: the domain, 22050 and 1 little-endian, then 0.5 and -0.25.
        let mut bytes = b"duet-take-1".to_vec();
        bytes.extend(22_050u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(0.5f32.to_le_bytes());
        bytes.extend((-0.25f32).to_le_bytes());
        let expected = TakeDigest(Sha256::digest(&bytes).into());
        let take = digest_of(&[chunk(0, 1, vec![0.5, -0.25])]);
        assert_eq!(take.digest, expected);
        assert_eq!((take.rate, take.channels, take.frames), (22_050, 1, 2));
    }

    #[test]
    fn how_the_samples_are_cut_into_chunks_does_not_change_it() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0).sin()).collect();
        let whole = digest_of(&[chunk(0, 2, samples.clone())]);
        let pieces: Vec<Chunk> = samples
            .chunks(46)
            .enumerate()
            .map(|(i, piece)| chunk(i as u64 * 23, 2, piece.to_vec()))
            .collect();
        assert_eq!(digest_of(&pieces), whole);
        assert_eq!(whole.frames, 500);
    }

    #[test]
    fn a_gap_is_hashed_as_the_silence_that_fills_it() {
        let gapped = digest_of(&[chunk(0, 1, vec![0.5; 4]), chunk(7, 1, vec![0.25; 2])]);
        let filled = digest_of(&[chunk(
            0,
            1,
            [vec![0.5; 4], vec![0.0; 3], vec![0.25; 2]].concat(),
        )]);
        assert_eq!(gapped, filled);
        assert_eq!(gapped.frames, 9);
    }

    #[test]
    fn the_rate_the_channels_and_every_sample_change_it() {
        let base = digest_of(&[chunk(0, 1, vec![0.5, 0.25])]);
        let mut other = chunk(0, 1, vec![0.5, 0.25]);
        other.rate = 44_100;
        assert_ne!(digest_of(&[other]).digest, base.digest);
        assert_ne!(
            digest_of(&[chunk(0, 2, vec![0.5, 0.25])]).digest,
            base.digest
        );
        let one_step = 0.25 + 1.0 / 32_768.0;
        assert_ne!(
            digest_of(&[chunk(0, 1, vec![0.5, one_step])]).digest,
            base.digest
        );
    }

    #[test]
    fn what_prepare_refuses_is_refused() {
        let mut digester = Digester::new();
        digester.push(&chunk(10, 1, vec![0.0; 4])).unwrap();
        let back = digester.push(&chunk(12, 1, vec![0.0])).unwrap_err();
        assert!(back.to_string().contains("goes back"), "{back}");
        let mut stereo = chunk(14, 2, vec![0.0; 2]);
        let changed = digester.push(&stereo).unwrap_err();
        assert!(changed.to_string().contains("mid-stream"), "{changed}");
        stereo.channels = 1;
        let far = 14 + u64::from(MAX_GAP_SECS) * 22_050 + 1;
        stereo.start_frame = far;
        let skipped = digester.push(&stereo).unwrap_err();
        assert!(skipped.to_string().contains("skips"), "{skipped}");
        let ragged = Digester::new()
            .push(&chunk(0, 2, vec![0.0; 3]))
            .unwrap_err();
        assert!(ragged.to_string().contains("whole number"), "{ragged}");
        assert_eq!(Digester::new().finish(), None);
    }

    #[test]
    fn a_digest_has_one_spelling_and_reads_back() {
        let take = digest_of(&[chunk(0, 1, vec![0.5])]);
        let text = take.digest.to_string();
        assert!(text.starts_with("sha256:") && text.len() == 71, "{text}");
        assert_eq!(text.parse::<TakeDigest>().unwrap(), take.digest);
        assert_eq!(take.short(), text[7..19]);
        for bad in [
            text.to_uppercase(),
            text.replace("sha256:", "sha1:"),
            text[..70].to_owned(),
            format!("{text}0"),
            text.replacen(&text[7..8], "g", 1),
        ] {
            assert!(bad.parse::<TakeDigest>().is_err(), "{bad} is read");
        }
        let json = serde_json::to_string(&take).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"digest":"{text}","rate":22050,"channels":1,"frames":1}}"#)
        );
        assert_eq!(serde_json::from_str::<Take>(&json).unwrap(), take);
    }
}
