//! Reading and writing Standard MIDI Files with midly.

use std::collections::{HashMap, VecDeque};

use midly::num::{u4, u7, u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};

use crate::error::{Error, Result};
use crate::model::{Control, KeyChange, Note, Song, TempoChange, TimeChange, Track};
use crate::pitch::Pitch;
use crate::time::{
    Key, TimeMap, TimeSig, beats_from_ticks, bpm_from_micros, micros_from_bpm, ticks_from_beats,
};

const DEFAULT_TEMPO_MICROS: u32 = 500_000;

/// A note with absolute ticks, before positions are worked out.
struct RawNote {
    start: u64,
    len: u64,
    key: u8,
    vel: u8,
}

enum RawControl {
    Cc(u8, u8),
    Bend(i16),
    Program(u8),
}

#[derive(Default)]
struct ChannelEvents {
    notes: Vec<RawNote>,
    controls: Vec<(u64, RawControl)>,
}

/// Keeps the last event at each tick, in tick order.
fn last_per_tick<T: Copy>(mut events: Vec<(u64, T)>) -> Vec<(u64, T)> {
    events.sort_by_key(|(tick, _)| *tick);
    let mut out: Vec<(u64, T)> = Vec::new();
    for (tick, value) in events {
        match out.last_mut() {
            Some(last) if last.0 == tick => last.1 = value,
            _ => out.push((tick, value)),
        }
    }
    out
}

impl Song {
    /// Reads a Standard MIDI File.
    ///
    /// Notes, tempo, time signature and key changes, track names, program changes,
    /// controllers and pitch bend are kept exactly. Aftertouch, SysEx, markers, lyrics and
    /// other text events, and note-off velocities are dropped. A MIDI track that uses
    /// several channels becomes one [`Track`] per channel.
    pub fn from_midi(bytes: &[u8]) -> Result<Song> {
        let smf = Smf::parse(bytes).map_err(|e| Error::Midi(e.to_string()))?;
        let tpq = match smf.header.timing {
            Timing::Metrical(tpq) => tpq.as_int(),
            Timing::Timecode(..) => {
                return Err(Error::Midi(
                    "files timed in SMPTE frames are not supported, only ticks per quarter".into(),
                ));
            }
        };

        let mut tempos: Vec<(u64, u32)> = Vec::new();
        let mut sigs: Vec<(u64, (u8, u8))> = Vec::new();
        let mut keys: Vec<(u64, (i8, bool))> = Vec::new();
        let mut title = None;
        // (midi track index, channel, name, events), in file order.
        let mut parts: Vec<(usize, u8, Option<String>, ChannelEvents)> = Vec::new();

        for (index, events) in smf.tracks.iter().enumerate() {
            let mut tick = 0u64;
            let mut name = None;
            let mut channels: HashMap<u8, ChannelEvents> = HashMap::new();
            let mut open: HashMap<(u8, u8), VecDeque<(u64, u8)>> = HashMap::new();

            for event in events {
                tick += u64::from(event.delta.as_int());
                match event.kind {
                    TrackEventKind::Meta(meta) => match meta {
                        MetaMessage::Tempo(micros) => tempos.push((tick, micros.as_int())),
                        MetaMessage::TimeSignature(num, den_power, _, _) => {
                            sigs.push((tick, (num, den_power)))
                        }
                        MetaMessage::KeySignature(sharps, minor) => {
                            keys.push((tick, (sharps, minor)))
                        }
                        MetaMessage::TrackName(bytes) if name.is_none() => {
                            name = Some(String::from_utf8_lossy(bytes).into_owned())
                        }
                        _ => {}
                    },
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        let part = channels.entry(ch).or_default();
                        match message {
                            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                                open.entry((ch, key.as_int()))
                                    .or_default()
                                    .push_back((tick, vel.as_int()));
                            }
                            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                                if let Some((start, vel)) = open
                                    .get_mut(&(ch, key.as_int()))
                                    .and_then(VecDeque::pop_front)
                                {
                                    part.notes.push(RawNote {
                                        start,
                                        len: tick - start,
                                        key: key.as_int(),
                                        vel,
                                    });
                                }
                            }
                            MidiMessage::Controller { controller, value } => part
                                .controls
                                .push((tick, RawControl::Cc(controller.as_int(), value.as_int()))),
                            MidiMessage::PitchBend { bend } => {
                                part.controls.push((tick, RawControl::Bend(bend.as_int())))
                            }
                            MidiMessage::ProgramChange { program } => part
                                .controls
                                .push((tick, RawControl::Program(program.as_int()))),
                            MidiMessage::Aftertouch { .. }
                            | MidiMessage::ChannelAftertouch { .. } => {}
                        }
                    }
                    TrackEventKind::SysEx(_) | TrackEventKind::Escape(_) => {}
                }
            }

            // Notes still sounding when the track ends last until its end.
            for ((ch, key), starts) in open {
                let part = channels.entry(ch).or_default();
                for (start, vel) in starts {
                    part.notes.push(RawNote {
                        start,
                        len: tick - start,
                        key,
                        vel,
                    });
                }
            }

            channels.retain(|_, part| !part.notes.is_empty() || !part.controls.is_empty());
            if channels.is_empty() {
                if title.is_none() {
                    title = name;
                }
                continue;
            }
            let mut chans: Vec<_> = channels.into_iter().collect();
            chans.sort_by_key(|(ch, _)| *ch);
            for (ch, part) in chans {
                parts.push((index, ch, name.clone(), part));
            }
        }

        // Tempo, meter and key.
        let tempos = last_per_tick(tempos);
        let sigs = last_per_tick(sigs);
        let keys = last_per_tick(keys);

        let to_sig = |(num, den_power): (u8, u8)| {
            if den_power > 6 {
                return Err(Error::Midi(format!(
                    "time signature denominator 2^{den_power} is not supported"
                )));
            }
            TimeSig::new(num, 1 << den_power).map_err(|e| Error::Midi(e.to_string()))
        };
        let to_key = |(sharps, minor): (i8, bool)| {
            Key::new(sharps, minor).map_err(|e| Error::Midi(e.to_string()))
        };

        let (initial_sig, sig_rest) = match sigs.split_first() {
            Some((&(0, sig), rest)) => (to_sig(sig)?, rest),
            _ => (TimeSig::default(), &sigs[..]),
        };
        let mut map = TimeMap::new(tpq, initial_sig).map_err(|e| Error::Midi(e.to_string()))?;
        let mut time_changes = Vec::new();
        for &(tick, sig) in sig_rest {
            let time_sig = to_sig(sig)?;
            let at = map
                .change_at_tick(tick, time_sig)
                .map_err(|e| Error::Midi(e.to_string()))?;
            time_changes.push(TimeChange { at, time_sig });
        }

        let (initial_tempo, tempo_rest) = match tempos.split_first() {
            Some((&(0, micros), rest)) => (micros, rest),
            _ => (DEFAULT_TEMPO_MICROS, &tempos[..]),
        };
        let tempo_changes = tempo_rest
            .iter()
            .map(|&(tick, micros)| TempoChange {
                at: map.position(tick),
                bpm: bpm_from_micros(micros),
            })
            .collect();

        let (key, key_rest) = match keys.split_first() {
            Some((&(0, key), rest)) => (Some(to_key(key)?), rest),
            _ => (None, &keys[..]),
        };
        let key_changes = key_rest
            .iter()
            .map(|&(tick, key)| {
                Ok(KeyChange {
                    at: map.position(tick),
                    key: to_key(key)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // Spell black keys the way the opening key does.
        let flat = |midi: u8| key.is_some_and(|k| k.spells_flat(midi % 12));

        let tracks = parts
            .into_iter()
            .map(|(index, ch, name, mut part)| {
                part.notes.sort_by_key(|n| (n.start, n.key, n.len, n.vel));
                part.controls.sort_by_key(|(tick, _)| *tick);
                let mut track = Track::new(
                    name.unwrap_or_else(|| format!("Track {}", index + 1)),
                    ch + 1,
                );
                let mut controls = part.controls.into_iter().peekable();
                if let Some((0, RawControl::Program(_))) = controls.peek()
                    && let Some((_, RawControl::Program(p))) = controls.next()
                {
                    track.program = Some(p);
                }
                track.controls = controls
                    .map(|(tick, control)| {
                        let at = map.position(tick);
                        match control {
                            RawControl::Cc(controller, value) => Control::Cc {
                                at,
                                controller,
                                value,
                            },
                            RawControl::Bend(value) => Control::Bend { at, value },
                            RawControl::Program(program) => Control::Program { at, program },
                        }
                    })
                    .collect();
                track.notes = part
                    .notes
                    .into_iter()
                    .map(|n| {
                        Ok(Note {
                            at: map.position(n.start),
                            pitch: Pitch::new(n.key)?.spelled_flat(flat(n.key)),
                            beats: beats_from_ticks(n.len, tpq),
                            vel: n.vel,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(track)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Song {
            title,
            ticks_per_quarter: tpq,
            tempo_bpm: bpm_from_micros(initial_tempo),
            time_sig: initial_sig,
            key,
            tempo_changes,
            time_changes,
            key_changes,
            tracks,
        })
    }

    /// Writes the song as a format 1 Standard MIDI File: a first track with the title,
    /// tempo, time signature and key, then one track per [`Track`].
    pub fn to_midi(&self) -> Result<Vec<u8>> {
        let tpq = self.ticks_per_quarter;
        if tpq == 0 || tpq > 0x7FFF {
            return Err(Error::invalid(format!(
                "ticks per quarter must be 1 to 32767, not {tpq}"
            )));
        }
        let map = self.time_map()?;

        // Each event is (tick, order class, sequence, event); sorting by the first three
        // keeps a note's end before a new note at the same tick and keeps file order.
        type Timed<'a> = (u64, u8, usize, TrackEventKind<'a>);

        let mut conductor: Vec<Timed> = Vec::new();
        if let Some(title) = &self.title {
            conductor.push((
                0,
                0,
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(title.as_bytes())),
            ));
        }
        let sig_event = |sig: TimeSig| {
            TrackEventKind::Meta(MetaMessage::TimeSignature(sig.num, sig.den_power(), 24, 8))
        };
        let key_event =
            |key: Key| TrackEventKind::Meta(MetaMessage::KeySignature(key.sharps, key.minor));
        let tempo_event = |bpm: f64| -> Result<TrackEventKind<'static>> {
            Ok(TrackEventKind::Meta(MetaMessage::Tempo(u24::new(
                micros_from_bpm(bpm)?,
            ))))
        };

        conductor.push((0, 1, 0, sig_event(self.time_sig)));
        {
            let mut changes = TimeMap::new(tpq, self.time_sig)?;
            for (i, change) in self.time_changes.iter().enumerate() {
                let tick = changes.change_at(change.at, change.time_sig)?;
                conductor.push((tick, 1, i + 1, sig_event(change.time_sig)));
            }
        }
        if let Some(key) = self.key {
            conductor.push((0, 2, 0, key_event(key)));
        }
        for (i, change) in self.key_changes.iter().enumerate() {
            conductor.push((map.tick(change.at)?, 2, i + 1, key_event(change.key)));
        }
        conductor.push((0, 3, 0, tempo_event(self.tempo_bpm)?));
        for (i, change) in self.tempo_changes.iter().enumerate() {
            conductor.push((map.tick(change.at)?, 3, i + 1, tempo_event(change.bpm)?));
        }

        let mut tracks = vec![finish_track(conductor)?];

        for track in &self.tracks {
            if !(1..=16).contains(&track.channel) {
                return Err(Error::invalid(format!(
                    "track '{}': channel {} is not 1 to 16",
                    track.name, track.channel
                )));
            }
            let channel = u4::new(track.channel - 1);
            let midi = |message| TrackEventKind::Midi { channel, message };
            let seven = |what: &str, v: u8| {
                if v > 127 {
                    Err(Error::invalid(format!(
                        "track '{}': {what} {v} is above 127",
                        track.name
                    )))
                } else {
                    Ok(u7::new(v))
                }
            };

            let mut events: Vec<Timed> = vec![(
                0,
                0,
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
            )];
            if let Some(program) = track.program {
                let program = seven("program", program)?;
                events.push((0, 1, 0, midi(MidiMessage::ProgramChange { program })));
            }
            for (i, control) in track.controls.iter().enumerate() {
                let tick = map.tick(control.at())?;
                let message = match *control {
                    Control::Cc {
                        controller, value, ..
                    } => MidiMessage::Controller {
                        controller: seven("controller", controller)?,
                        value: seven("controller value", value)?,
                    },
                    Control::Bend { value, .. } => {
                        if !(-8192..=8191).contains(&value) {
                            return Err(Error::invalid(format!(
                                "track '{}': pitch bend {value} is not -8192 to 8191",
                                track.name
                            )));
                        }
                        MidiMessage::PitchBend {
                            bend: PitchBend::from_int(value),
                        }
                    }
                    Control::Program { program, .. } => MidiMessage::ProgramChange {
                        program: seven("program", program)?,
                    },
                };
                events.push((tick, 2, i, midi(message)));
            }
            let mut timed_notes = Vec::with_capacity(track.notes.len());
            for note in &track.notes {
                if !(1..=127).contains(&note.vel) {
                    return Err(Error::invalid(format!(
                        "track '{}': note {} at {} has velocity {}, not 1 to 127",
                        track.name, note.pitch, note.at, note.vel
                    )));
                }
                let start = map.tick(note.at)?;
                let len = ticks_from_beats(note.beats, tpq)?;
                timed_notes.push((start, len, note));
            }
            check_no_nested_notes(&track.name, &timed_notes)?;
            // A reader pairs each note-off with the earliest note-on still sounding on that
            // key. Writing same-tick note-ons shortest first makes the first note-off belong
            // to the note it ends, so notes that start together keep their own lengths.
            timed_notes.sort_by_key(|(start, len, _)| (*start, *len));
            for (i, &(start, len, note)) in timed_notes.iter().enumerate() {
                let key = u7::new(note.pitch.midi());
                events.push((
                    start,
                    3,
                    i,
                    midi(MidiMessage::NoteOn {
                        key,
                        vel: u7::new(note.vel),
                    }),
                ));
                // A zero-length note must end after it starts, so it sorts last.
                let off_class = if len == 0 { 4 } else { 0 };
                events.push((
                    start + len,
                    off_class,
                    i,
                    midi(MidiMessage::NoteOff {
                        key,
                        vel: u7::new(64),
                    }),
                ));
            }
            tracks.push(finish_track(events)?);
        }

        let mut smf = Smf::new(Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(tpq)),
        ));
        smf.tracks = tracks;
        let mut out = Vec::new();
        smf.write_std(&mut out)
            .map_err(|e| Error::Midi(e.to_string()))?;
        Ok(out)
    }

    /// Puts the song in the form [`Song::from_midi`] would give: lengths rounded to whole
    /// ticks and written as their shortest decimal, tempos rounded to what MIDI stores,
    /// and positions written in canonical bar.beat.tick form (so `1.1.960` in 4/4 at 480
    /// ticks per quarter becomes `1.3.0`). The order of notes and controls is kept.
    pub fn normalize(&mut self) -> Result<()> {
        let tpq = self.ticks_per_quarter;
        let bpm = |bpm: f64| micros_from_bpm(bpm).map(bpm_from_micros);
        self.tempo_bpm = bpm(self.tempo_bpm)?;

        let mut map = TimeMap::new(tpq, self.time_sig)?;
        for change in &mut self.time_changes {
            let tick = map.tick(change.at)?;
            change.at = map.change_at_tick(tick, change.time_sig)?;
        }
        let canonical = |at| map.tick(at).map(|tick| map.position(tick));
        for change in &mut self.tempo_changes {
            change.at = canonical(change.at)?;
            change.bpm = bpm(change.bpm)?;
        }
        for change in &mut self.key_changes {
            change.at = canonical(change.at)?;
        }
        for track in &mut self.tracks {
            for note in &mut track.notes {
                note.at = canonical(note.at)?;
                note.beats = beats_from_ticks(ticks_from_beats(note.beats, tpq)?, tpq);
            }
            for control in &mut track.controls {
                let at = canonical(control.at())?;
                match control {
                    Control::Cc { at: a, .. }
                    | Control::Bend { at: a, .. }
                    | Control::Program { at: a, .. } => *a = at,
                }
            }
        }
        Ok(())
    }

    /// The map from positions to ticks for this song's time signatures.
    pub fn time_map(&self) -> Result<TimeMap> {
        let mut map = TimeMap::new(self.ticks_per_quarter, self.time_sig)?;
        for change in &self.time_changes {
            map.change_at(change.at, change.time_sig)?;
        }
        Ok(map)
    }
}

/// Refuses a note that starts after another note of the same pitch and ends before it.
///
/// MIDI has no note ids: a note-off ends whichever note on that key a reader pairs it
/// with. notefmt pairs the earliest note still sounding, so a nested note would come back
/// with its length swapped with the outer note's. Rather than change the notes silently,
/// writing them is an error. Notes that start together, or that only overlap, are fine.
fn check_no_nested_notes(track: &str, notes: &[(u64, u64, &Note)]) -> Result<()> {
    let mut by_key: HashMap<u8, Vec<(u64, u64, &Note)>> = HashMap::new();
    for &(start, len, note) in notes {
        by_key
            .entry(note.pitch.midi())
            .or_default()
            .push((start, start + len, note));
    }
    for mut same in by_key.into_values() {
        same.sort_by_key(|(start, end, _)| (*start, *end));
        // The longest-lasting note among those that start strictly earlier.
        let mut outer: Option<(u64, &Note)> = None;
        let mut group_start = 0;
        while group_start < same.len() {
            let start = same[group_start].0;
            let group_end = same[group_start..]
                .iter()
                .position(|(s, _, _)| *s != start)
                .map_or(same.len(), |n| group_start + n);
            for &(_, end, inner) in &same[group_start..group_end] {
                if let Some((outer_end, outer_note)) = outer
                    && end < outer_end
                {
                    return Err(Error::invalid(format!(
                        "track '{track}': the {} at {} lies inside the {} at {}; MIDI \
                             cannot keep a note of the same pitch inside another, so shorten \
                             the note at {} to end by {}",
                        inner.pitch,
                        inner.at,
                        outer_note.pitch,
                        outer_note.at,
                        outer_note.at,
                        inner.at
                    )));
                }
            }
            for &(_, end, note) in &same[group_start..group_end] {
                if outer.is_none_or(|(outer_end, _)| end > outer_end) {
                    outer = Some((end, note));
                }
            }
            group_start = group_end;
        }
    }
    Ok(())
}

/// Sorts timed events, turns them into delta times and ends the track.
fn finish_track(
    mut events: Vec<(u64, u8, usize, TrackEventKind<'_>)>,
) -> Result<Vec<TrackEvent<'_>>> {
    events.sort_by_key(|(tick, class, seq, _)| (*tick, *class, *seq));
    let mut out = Vec::with_capacity(events.len() + 1);
    let mut last = 0u64;
    for (tick, _, _, kind) in events {
        let delta = tick - last;
        if delta > 0x0FFF_FFFF {
            return Err(Error::invalid("a gap between events is too long for MIDI"));
        }
        out.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        last = tick;
    }
    out.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(out)
}
