//! A frame a FLAC reader cannot hand to the decoder is never read as silence without saying
//! so: [`FileSource`] states every gap it leaves ([`FileSource::gaps`]), and the take around
//! it stays whole.
//!
//! The files are made here, byte by byte: no fixture, and nothing from the owner's recordings.
//! `bomb.flac` holds one frame of a kind `duet record` once wrote: spec-valid, decodable by
//! the reference decoder, but over 20 MB of mostly zero bits (a fixed subframe whose rice
//! parameter writes each residual as a run of unary zeros), which libFLAC reads and
//! Symphonia's reader scans 16 MB past and skips without a word.
//! `damaged.flac` holds a frame whose CRC16 is right and whose subframe is a reserved type,
//! which reaches the decoder and is refused.

use std::fs;
use std::path::{Path, PathBuf};

use duet_capture::{Chunk, Digester, FileSource, MAX_GAPS, Source};

/// A scratch file under cargo's per-run target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(dir).unwrap();
    dir.join(name)
}

// --- A FLAC writer: streaminfo, frames, and the two checksums ---

/// FLAC's CRC-8 (polynomial 0x07), over a frame header.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// FLAC's CRC-16 (polynomial 0x8005), over a whole frame, table-driven.
fn crc16(bytes: &[u8]) -> u16 {
    let table: Vec<u16> = (0..256u16)
        .map(|byte| {
            let mut crc = byte << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    crc << 1 ^ 0x8005
                } else {
                    crc << 1
                };
            }
            crc
        })
        .collect();
    bytes.iter().fold(0u16, |crc, &byte| {
        (crc << 8) ^ table[usize::from((crc >> 8) as u8 ^ byte)]
    })
}

/// A big-endian bit stream.
struct Bits {
    bytes: Vec<u8>,
    /// Bits used in the last byte so far; 0 means the stream is byte-aligned.
    used: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits {
            bytes: Vec::new(),
            used: 0,
        }
    }

    /// The low `count` bits of `value`, most significant first.
    fn bits(&mut self, value: u64, count: u32) {
        for shift in (0..count).rev() {
            let bit = (value >> shift) & 1;
            if self.used == 0 {
                self.bytes.push(0);
            }
            *self.bytes.last_mut().unwrap() |= (bit as u8) << (7 - self.used);
            self.used = (self.used + 1) % 8;
        }
    }

    /// `count` zero bits, in bulk.
    fn zeros(&mut self, mut count: u64) {
        if self.used == 0 {
            let whole = usize::try_from(count / 8).unwrap();
            self.bytes.resize(self.bytes.len() + whole, 0);
            count %= 8;
        }
        for _ in 0..count {
            if self.used == 0 {
                self.bytes.push(0);
            }
            self.used = (self.used + 1) % 8;
        }
    }

    /// A unary-coded value: `value` zero bits, then a one bit.
    fn unary(&mut self, value: u64) {
        self.zeros(value);
        self.bits(1, 1);
    }

    /// Pads with zero bits to a byte boundary.
    fn align(&mut self) {
        if self.used != 0 {
            self.bits(0, 8 - self.used);
        }
    }
}

const RATE: u32 = 48_000;
const BLOCK: usize = 4096;

/// The stream marker and the one metadata block, `STREAMINFO`: mono, 24-bit, 48 kHz, fixed
/// blocks of `BLOCK`, `total` samples and no stated frame sizes or MD5.
fn streaminfo(total: u64) -> Vec<u8> {
    let mut out = b"fLaC".to_vec();
    out.extend([0x80, 0x00, 0x00, 34]); // the last block, type 0, 34 bytes
    out.extend((BLOCK as u16).to_be_bytes());
    out.extend((BLOCK as u16).to_be_bytes());
    out.extend([0, 0, 0, 0, 0, 0]); // frame sizes unknown
    let mut bits = Bits::new();
    bits.bits(u64::from(RATE), 20);
    bits.bits(0, 3); // one channel
    bits.bits(23, 5); // 24 bits a sample
    bits.bits(total, 36);
    out.extend(bits.bytes);
    out.extend([0; 16]); // no MD5
    out
}

/// A fixed-blocksize frame's header: the sync, the format codes, the frame number as UTF-8,
/// and the header's CRC-8.
fn frame_header(number: u64) -> Vec<u8> {
    let mut out = vec![0xff, 0xf8, 0xca, 0x0c]; // block 4096, 48 kHz, mono, 24-bit
    // The frame number as UTF-8: under 128 in one byte, else two.
    if number < 128 {
        out.push(number as u8);
    } else {
        out.push(0xc0 | (number >> 6) as u8);
        out.push(0x80 | (number & 0x3f) as u8);
    }
    let crc = crc8(&out);
    out.push(crc);
    out
}

/// A frame whose one subframe is verbatim: the raw 24-bit samples, big-endian.
fn verbatim_frame(number: u64, samples: &[i32]) -> Vec<u8> {
    assert_eq!(samples.len(), BLOCK);
    let mut out = frame_header(number);
    out.push(0x02); // a verbatim subframe, no wasted bits
    for &sample in samples {
        out.extend(sample.to_be_bytes()[1..].iter().copied());
    }
    let crc = crc16(&out);
    out.extend(crc.to_be_bytes());
    out
}

/// The kind of frame `duet record` once wrote for a clipped block: a fixed subframe with rice
/// parameter 0, so every residual of the block's run is written unary — here +20000, -20000,
/// which the reference decoder reads as the alternating samples 20000, 0, 20000, ... — about
/// 20 MB of mostly zero bits for 4 096 samples, longer than the 16 MB Symphonia's FLAC reader
/// scans for a frame's end.
fn bomb_frame(number: u64) -> Vec<u8> {
    let mut out = frame_header(number);
    let mut bits = Bits::new();
    bits.bits(0b00010010, 8); // a fixed subframe of order 1, no wasted bits
    bits.bits(0, 24); // the warmup sample
    bits.bits(0, 2); // partitioned rice, 4-bit parameters
    bits.bits(0, 4); // partition order 0: one partition
    bits.bits(0, 4); // rice parameter 0: every residual is unary
    for n in 1..BLOCK {
        // Residual +20000 (zigzag 40000) or -20000 (zigzag 39999): samples 20000, 0, ...
        bits.unary(if n % 2 == 0 { 39_999 } else { 40_000 });
    }
    bits.align();
    out.extend(bits.bytes);
    let crc = crc16(&out);
    out.extend(crc.to_be_bytes());
    out
}

/// A frame whose CRC16 is right but whose subframe type is reserved (0x08): it reaches the
/// decoder and is refused.
fn reserved_frame(number: u64) -> Vec<u8> {
    let mut out = frame_header(number);
    out.extend([0x08, 0xaa, 0x55, 0xaa, 0x55]);
    let crc = crc16(&out);
    out.extend(crc.to_be_bytes());
    out
}

/// `BLOCK`-sample ramps as the frames' contents, distinct per frame.
fn ramp(base: i32) -> Vec<i32> {
    (0..BLOCK).map(|i| base + 100 * (i as i32 % 97)).collect()
}

/// The samples of `ramp` decoded to `f32`, as a 24-bit read gives them.
fn to_f32(samples: &[i32]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 8_388_608.0).collect()
}

/// `bomb.flac`: a ramp, then the bomb frame, then another ramp: 12 288 samples, the whole
/// middle frame over 16 MB.
fn write_bomb_flac(path: &Path) -> (Vec<f32>, Vec<f32>) {
    let first = ramp(1000);
    let after = ramp(-500_000);
    let mut bytes = streaminfo(3 * BLOCK as u64);
    bytes.extend(verbatim_frame(0, &first));
    bytes.extend(bomb_frame(1));
    bytes.extend(verbatim_frame(2, &after));
    fs::write(path, bytes).unwrap();
    (to_f32(&first), to_f32(&after))
}

#[test]
fn a_decodable_frame_the_flac_reader_skips_is_stated_and_the_take_around_it_is_whole() {
    let path = scratch("bomb.flac");
    let (first, after) = write_bomb_flac(&path);
    let mut source = FileSource::open(&path).unwrap();
    let mut chunks = Vec::new();
    let mut digester = Digester::new();
    while let Some(chunk) = source.next_chunk().unwrap() {
        digester.push(&chunk).unwrap();
        chunks.push(chunk);
    }
    let take = digester.finish().unwrap();

    // The gap is stated: where it starts, and how long it runs.
    let gaps = source.gaps();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!((gaps[0].start, gaps[0].frames), (4096, 4096), "{gaps:?}");

    // The take around it is whole: the chunk after the gap carries its true start frame and
    // its own samples, and nothing before it moved.
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(chunks[0].start_frame, 0);
    assert_eq!(chunks[0].samples, first, "the first frame's samples");
    assert_eq!(
        chunks[1].start_frame, 8192,
        "the frame after the gap's place"
    );
    assert_eq!(chunks[1].samples, after, "the third frame's samples");
    assert_eq!((take.rate, take.channels, take.frames), (RATE, 1, 12_288));

    // And the digest is over the gap read as silence: the file decodes to the same take as
    // the three spans read with the silence written in.
    let mut filled = Digester::new();
    filled
        .push(&duet_capture::Chunk {
            rate: RATE,
            channels: 1,
            start_frame: 0,
            samples: [first, to_f32(&[0; BLOCK]), after].concat(),
        })
        .unwrap();
    assert_eq!(filled.finish().unwrap().digest, take.digest);
}

#[test]
fn a_packet_the_decoder_refuses_is_stated_and_the_stream_goes_on() {
    let path = scratch("damaged.flac");
    let first = ramp(2000);
    let last = ramp(-700_000);
    let mut bytes = streaminfo(3 * BLOCK as u64);
    bytes.extend(verbatim_frame(0, &first));
    bytes.extend(reserved_frame(1));
    bytes.extend(verbatim_frame(2, &last));
    fs::write(&path, bytes).unwrap();

    let mut source = FileSource::open(&path).unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = source.next_chunk().unwrap() {
        chunks.push(chunk);
    }
    let gaps = source.gaps();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!((gaps[0].start, gaps[0].frames), (4096, 4096), "{gaps:?}");
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(chunks[0].samples, to_f32(&first));
    assert_eq!(chunks[1].start_frame, 8192);
    assert_eq!(chunks[1].samples, to_f32(&last));
}

#[test]
fn a_whole_decode_states_no_gaps() {
    for name in ["notes22.flac", "notes44.flac", "tone-stereo48.flac"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let mut source = FileSource::open(&path).unwrap();
        while source.next_chunk().unwrap().is_some() {}
        assert!(source.gaps().is_empty(), "{name}: {:?}", source.gaps());
    }
}

/// Reads `path` to its end, returning the chunks, or the first error.
fn read_all(path: &Path) -> (FileSource, duet_capture::Result<Vec<Chunk>>) {
    let mut source = FileSource::open(path).unwrap();
    let mut chunks = Vec::new();
    loop {
        match source.next_chunk() {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => return (source, Ok(chunks)),
            Err(e) => return (source, Err(e)),
        }
    }
}

#[test]
fn a_run_of_refused_packets_is_one_gap() {
    // A valid frame, 40 refused ones in a row, then a valid one: one gap of 40 blocks, not 40
    // gaps, so a file of many tiny refused frames holds one entry, not one per frame.
    let path = scratch("refused-run.flac");
    let first = ramp(3000);
    let last = ramp(-600_000);
    let mut bytes = streaminfo(42 * BLOCK as u64);
    bytes.extend(verbatim_frame(0, &first));
    for n in 1..=40 {
        bytes.extend(reserved_frame(n));
    }
    bytes.extend(verbatim_frame(41, &last));
    fs::write(&path, bytes).unwrap();

    let (source, chunks) = read_all(&path);
    let chunks = chunks.unwrap();
    let gaps: Vec<(u64, u64)> = source.gaps().iter().map(|g| (g.start, g.frames)).collect();
    assert_eq!(gaps, [(BLOCK as u64, 40 * BLOCK as u64)]);
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(chunks[1].start_frame, 41 * BLOCK as u64);
    assert_eq!(chunks[1].samples, to_f32(&last));
}

#[test]
fn a_file_with_more_separate_gaps_than_the_bound_is_refused() {
    // Valid and refused frames alternating: each refused frame is a separate gap. MAX_GAPS of
    // them are stated; one more refuses the file by name.
    let read = |holes: usize| {
        let path = scratch(&format!("holes-{holes}.flac"));
        let mut bytes = streaminfo((2 * holes as u64 + 1) * BLOCK as u64);
        for n in 0..holes as u64 {
            bytes.extend(verbatim_frame(2 * n, &ramp(n as i32)));
            bytes.extend(reserved_frame(2 * n + 1));
        }
        bytes.extend(verbatim_frame(2 * holes as u64, &ramp(-1)));
        fs::write(&path, bytes).unwrap();
        read_all(&path)
    };
    let (source, chunks) = read(MAX_GAPS);
    assert!(chunks.is_ok(), "{chunks:?}");
    assert_eq!(source.gaps().len(), MAX_GAPS);

    let (source, chunks) = read(MAX_GAPS + 1);
    let err = chunks.expect_err("one gap past the bound refuses the file");
    assert!(err.to_string().contains("separate gaps"), "{err}");
    assert_eq!(source.gaps().len(), MAX_GAPS);
}
