//! Playing a duet [`Song`]: the timed MIDI events it sounds as, a SoundFont synth that
//! renders them to audio, and a sender that hands them to a MIDI port at their times.
//!
//! The events come from the song's own MIDI file, [`Midi::to_midi`], so what plays is
//! exactly what `duet convert song.notes song.mid` writes: every note, control, program
//! and tempo change at the same tick. Ticks become times through the file's tempo map.
//!
//! Nothing here opens an audio device or a MIDI port. [`Renderer::fill`] is what an audio
//! callback calls, and [`send_timed`] takes its clock, its sleep and its port as functions,
//! so both run the same in a test as on a real device.
//!
//! ```
//! use duet_core::Song;
//! use duet_play::Schedule;
//!
//! let song = Song::from_text(
//!     "tempo 120\ntime 4/4\nend 2.1.0\n\n[track \"Piano\" ch1 prog0]\n1.2.0 C4 1 80\n",
//! )?;
//! let schedule = Schedule::from_song(&song)?;
//! // At 120 bpm a beat lasts half a second: the note starts on beat 2 and ends on beat 3.
//! let times: Vec<f64> = schedule.events().iter().map(|e| e.at.as_secs_f64()).collect();
//! assert_eq!(times, [0.0, 0.5, 1.0]); // program, note on, note off
//! assert_eq!(schedule.end().as_secs_f64(), 2.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use duet_core::Song;
use duet_midi::Midi;
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

/// Everything that can go wrong preparing a song to play.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The song cannot be written as MIDI, so it cannot be played either.
    #[error(transparent)]
    Midi(#[from] duet_midi::Error),
    /// The MIDI the song wrote could not be read back as events. This is a bug in duet.
    #[error("reading the song's own MIDI: {0}")]
    Events(String),
    /// The SoundFont could not be read.
    #[error("reading the SoundFont: {0}")]
    SoundFont(String),
    /// The synthesizer refused its settings, such as a sample rate it cannot run at.
    #[error("starting the synthesizer: {0}")]
    Synth(String),
}

/// A `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// One channel message, as the bytes a MIDI port is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    bytes: [u8; 3],
    len: u8,
}

impl Message {
    fn new(bytes: &[u8]) -> Message {
        let mut out = [0; 3];
        out[..bytes.len()].copy_from_slice(bytes);
        Message {
            bytes: out,
            len: bytes.len() as u8,
        }
    }

    /// The message's bytes: a status byte (the kind in the high four bits, the channel,
    /// 0 to 15, in the low four), then one or two data bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// The channel, 0 to 15 (MIDI channel 1 is 0).
    pub fn channel(&self) -> u8 {
        self.bytes[0] & 0x0F
    }

    /// The kind of message: the status byte's high four bits, such as `0x90` for note on.
    pub fn kind(&self) -> u8 {
        self.bytes[0] & 0xF0
    }

    fn data(&self) -> (u8, u8) {
        (self.bytes[1], self.bytes[2])
    }
}

/// A message and when it sounds, from the start of the song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// The time from the start of the song.
    pub at: Duration,
    /// The tick it falls on, from the start of the song.
    pub tick: u64,
    pub message: Message,
}

/// Every message a song sounds as, in the order it is sent, each with its time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    events: Vec<Event>,
    end: Duration,
}

impl Schedule {
    /// The song's events, read from the MIDI file [`Midi::to_midi`] writes for it.
    pub fn from_song(song: &Song) -> Result<Schedule> {
        Schedule::from_midi(&song.to_midi()?)
    }

    /// The events of a Standard MIDI File with metrical timing. Messages from every track
    /// are merged by tick; at the same tick, the earlier track's come first, and each
    /// track's keep their order in the file.
    pub fn from_midi(bytes: &[u8]) -> Result<Schedule> {
        let smf = Smf::parse(bytes).map_err(|e| Error::Events(e.to_string()))?;
        let Timing::Metrical(tpq) = smf.header.timing else {
            return Err(Error::Events("the file's timing is not metrical".into()));
        };
        let tpq = u64::from(tpq.as_int());
        if tpq == 0 {
            return Err(Error::Events("ticks per quarter is 0".into()));
        }

        let mut tempos: Vec<(u64, u32)> = Vec::new();
        let mut messages: Vec<(u64, Message)> = Vec::new();
        let mut end_tick = 0;
        for track in &smf.tracks {
            let mut tick = 0u64;
            for event in track {
                tick += u64::from(event.delta.as_int());
                match event.kind {
                    TrackEventKind::Meta(MetaMessage::Tempo(micros)) => {
                        tempos.push((tick, micros.as_int()));
                    }
                    TrackEventKind::Meta(MetaMessage::EndOfTrack) => end_tick = end_tick.max(tick),
                    TrackEventKind::Midi { channel, message } => {
                        if let Some(message) = channel_message(channel.as_int(), message) {
                            messages.push((tick, message));
                        }
                    }
                    _ => {}
                }
            }
            end_tick = end_tick.max(tick);
        }
        // Stable: the same tick keeps track order, then file order.
        messages.sort_by_key(|(tick, _)| *tick);
        tempos.sort_by_key(|(tick, _)| *tick);

        let clock = Clock::new(tpq, &tempos);
        let events = messages
            .into_iter()
            .map(|(tick, message)| Event {
                at: clock.at(tick),
                tick,
                message,
            })
            .collect();
        Ok(Schedule {
            events,
            end: clock.at(end_tick),
        })
    }

    /// The events, in the order they are sent.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// When the song ends: its stated end, which every track of its MIDI ends on.
    pub fn end(&self) -> Duration {
        self.end
    }
}

/// The bytes of a channel message, or `None` for one a song never writes.
fn channel_message(channel: u8, message: MidiMessage) -> Option<Message> {
    let status = |kind: u8| kind | (channel & 0x0F);
    Some(match message {
        MidiMessage::NoteOff { key, vel } => {
            Message::new(&[status(0x80), key.as_int(), vel.as_int()])
        }
        MidiMessage::NoteOn { key, vel } => {
            Message::new(&[status(0x90), key.as_int(), vel.as_int()])
        }
        MidiMessage::Controller { controller, value } => {
            Message::new(&[status(0xB0), controller.as_int(), value.as_int()])
        }
        MidiMessage::ProgramChange { program } => Message::new(&[status(0xC0), program.as_int()]),
        MidiMessage::PitchBend { bend } => {
            let raw = bend.0.as_int();
            Message::new(&[status(0xE0), (raw & 0x7F) as u8, (raw >> 7) as u8])
        }
        MidiMessage::Aftertouch { key, vel } => {
            Message::new(&[status(0xA0), key.as_int(), vel.as_int()])
        }
        MidiMessage::ChannelAftertouch { vel } => Message::new(&[status(0xD0), vel.as_int()]),
    })
}

/// Ticks to time, through a tempo map. Time is kept as tick × microseconds-per-quarter
/// sums, so no rounding builds up across tempo changes.
struct Clock {
    tpq: u64,
    /// (tick, microseconds per quarter from it, tick × micros summed before it)
    segments: Vec<(u64, u64, u128)>,
}

/// MIDI's tempo when a file states none: 120 beats per minute.
const DEFAULT_MICROS: u64 = 500_000;

impl Clock {
    fn new(tpq: u64, tempos: &[(u64, u32)]) -> Clock {
        let mut segments = vec![(0u64, DEFAULT_MICROS, 0u128)];
        for &(tick, micros) in tempos {
            let &(start, prev, sum) = segments.last().expect("never empty");
            let sum = sum + u128::from(tick - start) * u128::from(prev);
            if tick == start {
                // A later tempo at the same tick replaces the earlier one.
                segments.pop();
            }
            segments.push((tick, u64::from(micros), sum));
        }
        Clock { tpq, segments }
    }

    fn at(&self, tick: u64) -> Duration {
        let &(start, micros, sum) = self
            .segments
            .iter()
            .rev()
            .find(|(start, ..)| *start <= tick)
            .expect("the first segment starts at tick 0");
        let total = sum + u128::from(tick - start) * u128::from(micros);
        // total / tpq is microseconds; keep nanoseconds.
        let nanos = total * 1000 / u128::from(self.tpq);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// How long a renderer keeps sounding after the song's end, so the last notes' release
/// is heard rather than cut off.
pub const TAIL: Duration = Duration::from_secs(1);

/// Reads a SoundFont (.sf2) from its bytes.
pub fn load_soundfont(mut bytes: &[u8]) -> Result<Arc<SoundFont>> {
    SoundFont::new(&mut bytes)
        .map(Arc::new)
        .map_err(|e| Error::SoundFont(e.to_string()))
}

/// The sample a time falls on at a sample rate, rounded to the nearest.
pub fn sample_at(at: Duration, sample_rate: u32) -> u64 {
    let nanos = at.as_nanos() * u128::from(sample_rate);
    u64::try_from((nanos + 500_000_000) / 1_000_000_000).unwrap_or(u64::MAX)
}

/// A schedule played through a SoundFont synth, a buffer at a time. Each event is handed
/// to the synth at its own sample, however a device's buffers fall, so the sound does not
/// depend on the buffer size the device asks for. The synth itself works in blocks of 64
/// samples, and an event takes effect from the next block: within 1.5 ms at 44.1 kHz.
pub struct Renderer {
    synth: Synthesizer,
    events: Vec<(u64, Message)>,
    next: usize,
    position: u64,
    total: u64,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("events", &self.events.len())
            .field("next", &self.next)
            .field("position", &self.position)
            .field("total", &self.total)
            .finish_non_exhaustive()
    }
}

impl Renderer {
    /// A renderer for a schedule at a sample rate, such as 44100.
    pub fn new(schedule: &Schedule, font: &Arc<SoundFont>, sample_rate: u32) -> Result<Renderer> {
        let rate = i32::try_from(sample_rate)
            .map_err(|_| Error::Synth(format!("a sample rate of {sample_rate}")))?;
        let synth = Synthesizer::new(font, &SynthesizerSettings::new(rate))
            .map_err(|e| Error::Synth(e.to_string()))?;
        let events = schedule
            .events()
            .iter()
            .map(|e| (sample_at(e.at, sample_rate), e.message))
            .collect();
        Ok(Renderer {
            synth,
            events,
            next: 0,
            position: 0,
            total: sample_at(schedule.end() + TAIL, sample_rate),
        })
    }

    /// How many frames the whole render is: the song to its end, then [`TAIL`].
    pub fn total_frames(&self) -> u64 {
        self.total
    }

    /// Whether every frame has been rendered.
    pub fn is_done(&self) -> bool {
        self.position >= self.total
    }

    /// Renders the next `left.len()` frames into `left` and `right`, which must be the
    /// same length, and returns how many belong to the render. Frames past its end are
    /// silence.
    pub fn fill(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        assert_eq!(
            left.len(),
            right.len(),
            "left and right are the same length"
        );
        let mut done = 0;
        while done < left.len() && self.position < self.total {
            while let Some(&(at, message)) = self.events.get(self.next) {
                if at > self.position {
                    break;
                }
                let (data1, data2) = message.data();
                self.synth.process_midi_message(
                    i32::from(message.channel()),
                    i32::from(message.kind()),
                    i32::from(data1),
                    i32::from(data2),
                );
                self.next += 1;
            }
            let until = self
                .events
                .get(self.next)
                .map_or(self.total, |&(at, _)| at.min(self.total));
            let room = (left.len() - done) as u64;
            let n = (until - self.position).min(room) as usize;
            self.synth
                .render(&mut left[done..done + n], &mut right[done..done + n]);
            done += n;
            self.position += n as u64;
        }
        let rendered = done;
        left[done..].fill(0.0);
        right[done..].fill(0.0);
        rendered
    }
}

/// Renders a whole schedule offline, as (left, right) at a sample rate: the song to its
/// end, then [`TAIL`].
pub fn render(
    schedule: &Schedule,
    font: &Arc<SoundFont>,
    sample_rate: u32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let mut renderer = Renderer::new(schedule, font, sample_rate)?;
    let frames = usize::try_from(renderer.total_frames())
        .map_err(|_| Error::Synth("the song is too long to render in memory".into()))?;
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    renderer.fill(&mut left, &mut right);
    Ok((left, right))
}

/// Sends each event of a schedule at its time: `now` gives the time since the start,
/// `sleep` waits, and `send` hands a message's bytes to the port. Stops at the first
/// error `send` returns. After the last event it waits until the song's end, so a port
/// is not closed early.
pub fn send_timed<E>(
    schedule: &Schedule,
    mut now: impl FnMut() -> Duration,
    mut sleep: impl FnMut(Duration),
    mut send: impl FnMut(&[u8]) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let mut wait_until = |at: Duration| {
        let elapsed = now();
        if at > elapsed {
            sleep(at - elapsed);
        }
    };
    for event in schedule.events() {
        wait_until(event.at);
        send(event.message.bytes())?;
    }
    wait_until(schedule.end());
    Ok(())
}
