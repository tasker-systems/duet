//! Assembly: a take's settled events as a [`Song`], the one step that keeps the whole take,
//! and only as events.
//!
//! A frame becomes a position by the take's measured beat period, not its stated tempo: a
//! frame is `(frame - downbeat) / beat_period_frames` quarter notes from the downbeat, at the
//! song's 480 ticks per quarter. The song states the tempo as `Timing` gives it (a whole bpm
//! when estimated), so a note keeps its true beat however the tempo was rounded.

use duet_abc::pickup_meter_for;
use duet_core::time::beats_from_ticks;
use duet_core::{Grid, Key, Note, Pitch, Position, Song, TimeChange, TimeSig, Track};
use duet_drums::LaneMap;
use duet_dsp::{BeatTempo, Event, HitLane, KeyEstimate, Timing, beat_series, estimate_key};
use serde::Serialize;

use crate::error::{Error, Result};

/// The velocity of a note or hit of no strength. Velocity rises in a straight line from it to
/// 127, the velocity of the take's strongest note or hit, so no note is all but silent however
/// quiet the take.
const VELOCITY_FLOOR: u8 = 20;
/// The loudest velocity, which the take's strongest note or hit gets.
const VELOCITY_TOP: u8 = 127;
/// A hit's length, in quarter notes: a sixteenth, or less when the next hit on the same key
/// comes sooner. A drum grid keeps only where hits start, so the length only has to be short.
const HIT_BEATS: f64 = 0.25;
/// The MIDI channel of a melody: the first a reader would give it (`duet_abc` skips 10, the
/// drums').
const MELODY_CHANNEL: u8 = 1;
/// The lanes a beatboxed take's kick, snare and hat go to, by name in the lane map.
const KICK_LANE: &str = "kick";
const SNARE_LANE: &str = "snare";
const HAT_LANE: &str = "hh";

/// What a take is: a melody, or a rhythm whose hits go to drum lanes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Kind {
    /// Pitched notes, on one track named `melody`.
    Melody,
    /// Drum hits, on one track named `drums` on the lane map's channel. With `asked`, every
    /// hit goes to that lane (a name or alias in `lanes`); without it the take is beatboxed,
    /// and each hit goes to the `kick`, `snare` or `hh` lane its sound was told apart as.
    Rhythm {
        lanes: LaneMap,
        asked: Option<String>,
    },
}

/// What was done with the events before the downbeat, since a song cannot hold a position
/// before `1.1.0`. Nothing is ever dropped.
///
/// Deliberately exhaustive: there was nothing before the downbeat, or it became a pickup, or
/// it became bars of rests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadIn {
    /// Nothing came before the downbeat, which is `1.1.0`.
    None,
    /// A short first bar of this signature holds them, and the take's meter starts at bar 2,
    /// where the downbeat is.
    Pickup(TimeSig),
    /// Whole bars of the take's meter hold them, as rests up to the first event, and the
    /// downbeat is the bar after them: bar 2, unless the take starts more than a bar early.
    RestBar,
}

/// A take as a song, with the timing it was placed by, what was done before the downbeat, and
/// the analysis behind it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Captured {
    pub song: Song,
    pub timing: Timing,
    pub lead_in: LeadIn,
    /// The tick the downbeat bar starts at: 0 with no lead-in, the pickup's length after a
    /// pickup, or the length of the rest bars after them. Quantizing keeps bar lines where
    /// they are, so it is where the take's bar 1 is in the quantized song too.
    pub downbeat_tick: u64,
    pub analysis: Analysis,
}

/// Everything the analysis found, as `<stem>.capture.json` holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Analysis {
    /// The rate the events' frames count at.
    pub rate: u32,
    pub events: Vec<Event>,
    pub timing: Timing,
    /// The melody's key, given or estimated; `None` for a rhythm take.
    pub key: Option<KeyEstimate>,
    /// The tempo from each beat to the next.
    pub beats: Vec<BeatTempo>,
    /// The grid the score was quantized to, which `offsets` count steps of: the one asked for,
    /// or the finer one it was refined to ([`quantize_apart`](crate::quantize_apart)).
    /// Assembly leaves it `None`, since the grid is chosen later.
    pub grid: Option<Grid>,
    /// How far quantizing moved each note. Assembly leaves it empty: the grid is chosen
    /// later, and [`quantize_offsets`] fills it once the song is quantized.
    pub offsets: Vec<QuantizeOffset>,
}

/// How far quantizing moved one note.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[non_exhaustive]
pub struct QuantizeOffset {
    /// The note's index, counting through the tracks in order.
    pub note: usize,
    /// How far its start moved, in milliseconds: negative is earlier.
    pub moved_ms: f64,
    /// The same in steps of the grid.
    pub moved_steps: f64,
}

/// The take's events as a song, placed by `timing`, with the analysis behind it. Every
/// frame is on the input timeline at `rate`, and the events come in the order they settled,
/// which is the order they start in.
///
/// - **Lengths** are at least a tick, so no note has none.
/// - **One track:** a melody's [`Event::Note`]s on `melody`, channel 1; or a rhythm's
///   [`Event::Hit`]s on `drums`, on the lane map's channel, each the key of its lane and a
///   sixteenth long (`HIT_BEATS`), or up to the next hit on its key when that is sooner. Other events are kept in the analysis only.
/// - **Key:** a melody's is `key` when given, used exactly, or else estimated from its notes
///   ([`estimate_key`]). It is the song's key, and black keys are spelled by it; no note's MIDI
///   number changes.
/// - **Velocity** is relative to the take: `strength / strongest`, mapped in a straight line
///   from 20 (`VELOCITY_FLOOR`) at no strength to 127 for the strongest. Two takes that differ
///   only in level get the same velocities, and the velocities are left as they come, not
///   snapped to any levels.
/// - **Before the downbeat** ([`LeadIn`]): a melody's lead-in, carried up to a whole number of
///   the meter's beats so its bar line keeps the beat, becomes a pickup when that is shorter
///   than a bar and [`pickup_meter_for`] gives it a signature, which is the rule the ABC writer
///   writes a pickup by. Otherwise, and always for a rhythm, since drum grids number bars from 1
///   with no gaps, it becomes whole bars of rests. Nothing is dropped.
///
/// Fails with [`Error::NothingFound`] when a melody has no notes or a rhythm no hits, and with
/// [`Error::Invalid`] when a rhythm is given a key or asks for a lane its map lacks.
pub fn assemble(
    events: &[Event],
    rate: u32,
    timing: &Timing,
    key: Option<Key>,
    kind: &Kind,
) -> Result<Captured> {
    let (played, estimate) = match kind {
        Kind::Melody => (melody(events)?, estimate_key(events, key)),
        Kind::Rhythm { lanes, asked } => {
            if let Some(key) = key {
                return Err(Error::Invalid(format!(
                    "a rhythm take has no key, so it cannot be given {key}"
                )));
            }
            (hits(events, lanes, asked.as_deref())?, None)
        }
    };

    let mut song = Song::default();
    let tpq = song.ticks_per_quarter;
    let place = Placer {
        downbeat: *timing.downbeat_frame.value() as f64,
        period: timing.beat_period_frames,
        tpq,
    };
    let meter = *timing.meter.value();
    let lead_ticks = played
        .iter()
        .map(|p| -place.ticks(p.start))
        .max()
        .unwrap_or(0)
        .max(0) as u64;
    let (lead_in, offset) = lead_in(lead_ticks, meter, tpq, kind);

    song.tempo_bpm = *timing.tempo_bpm.value();
    song.time_sig = match lead_in {
        LeadIn::Pickup(sig) => sig,
        LeadIn::None | LeadIn::RestBar => meter,
    };
    if let LeadIn::Pickup(_) = lead_in {
        song.time_changes.push(TimeChange {
            at: Position::bar(2),
            time_sig: meter,
        });
    }
    song.key = estimate.as_ref().map(|e| *e.key.value());
    let map = song.time_map()?;

    let (name, channel) = match kind {
        Kind::Melody => ("melody", MELODY_CHANNEL),
        Kind::Rhythm { lanes, .. } => ("drums", lanes.channel()),
    };
    let mut track = Track::new(name, channel);
    let at = |frame: u64| (offset as i64 + place.ticks(frame)).max(0) as u64;
    let strongest = played.iter().map(|p| p.strength).fold(0.0, f32::max);
    for (i, p) in played.iter().enumerate() {
        let start = at(p.start);
        let end = match p.end {
            Some(end) => at(end).max(start + 1),
            None => {
                let next = played[i + 1..].iter().find(|q| q.key == p.key);
                let short = start + (HIT_BEATS * f64::from(tpq)).round() as u64;
                next.map_or(short, |q| short.min(at(q.start)))
                    .max(start + 1)
            }
        };
        let mut pitch = Pitch::new(p.key)?;
        if let Some(key) = song.key {
            pitch = pitch.spelled_flat(key.spells_flat(p.key % 12));
        }
        track.notes.push(Note::new(
            map.position(start),
            pitch,
            beats_from_ticks(end - start, tpq),
            velocity(p.strength, strongest),
        ));
    }
    song.tracks.push(track);

    let analysis = Analysis {
        rate,
        events: events.to_vec(),
        timing: timing.clone(),
        key: estimate,
        beats: beat_series(events, rate, timing),
        grid: None,
        offsets: Vec::new(),
    };
    Ok(Captured {
        song,
        timing: timing.clone(),
        lead_in,
        downbeat_tick: offset,
        analysis,
    })
}

/// A note or hit to place: its start and, for a note, end frame, its MIDI key, its strength.
#[derive(Debug, Clone, Copy)]
struct Played {
    start: u64,
    end: Option<u64>,
    key: u8,
    strength: f32,
}

/// A melody's notes, or [`Error::NothingFound`].
fn melody(events: &[Event]) -> Result<Vec<Played>> {
    let notes: Vec<Played> = events
        .iter()
        .filter_map(|e| match *e {
            Event::Note {
                start,
                end,
                midi,
                strength,
                ..
            } => Some(Played {
                start,
                end: Some(end),
                key: midi,
                strength,
            }),
            _ => None,
        })
        .collect();
    if notes.is_empty() {
        return Err(Error::NothingFound { what: "notes" });
    }
    Ok(notes)
}

/// A rhythm's hits, each on its lane's key: the `asked` lane for every hit when there is one,
/// or else the beatboxed lane it was told apart as. Fails with [`Error::NothingFound`], or
/// with [`Error::Invalid`] for a lane the map lacks or a hit on an asked lane with none asked.
fn hits(events: &[Event], lanes: &LaneMap, asked: Option<&str>) -> Result<Vec<Played>> {
    let key_of = |name: &str| {
        lanes.lookup(name).map(|lane| lane.key).ok_or_else(|| {
            Error::Invalid(format!(
                "the {} lane map has no lane named \"{name}\"",
                lanes.name()
            ))
        })
    };
    let asked = asked.map(key_of).transpose()?;
    let mut hits = Vec::new();
    for event in events {
        let Event::Hit {
            frame,
            lane,
            strength,
        } = *event
        else {
            continue;
        };
        let key = match (asked, lane) {
            (Some(key), _) => key,
            (None, HitLane::Kick) => key_of(KICK_LANE)?,
            (None, HitLane::Snare) => key_of(SNARE_LANE)?,
            (None, HitLane::Hat) => key_of(HAT_LANE)?,
            (None, HitLane::Asked) => {
                return Err(Error::Invalid(
                    "a hit is on the asked lane, but no lane was asked for".into(),
                ));
            }
            // HitLane is non_exhaustive now that it is duet-dsp's: a lane added there is
            // refused by name until assembly knows its key.
            (None, other) => {
                return Err(Error::Invalid(format!(
                    "a hit is on the {other:?} lane, which assembly has no key for"
                )));
            }
        };
        hits.push(Played {
            start: frame,
            end: None,
            key,
            strength,
        });
    }
    if hits.is_empty() {
        return Err(Error::NothingFound { what: "hits" });
    }
    Ok(hits)
}

/// Frames to ticks from the downbeat, by the measured beat period.
#[derive(Debug, Clone, Copy)]
struct Placer {
    downbeat: f64,
    period: f64,
    tpq: u16,
}

impl Placer {
    /// How many ticks after the downbeat `frame` is: negative before it.
    fn ticks(&self, frame: u64) -> i64 {
        ((frame as f64 - self.downbeat) / self.period * f64::from(self.tpq)).round() as i64
    }
}

/// What to do with a lead-in `lead` ticks long before a downbeat in `meter`, and how many ticks
/// the downbeat then comes after `1.1.0`.
fn lead_in(lead: u64, meter: TimeSig, tpq: u16, kind: &Kind) -> (LeadIn, u64) {
    if lead == 0 {
        return (LeadIn::None, 0);
    }
    let beat = 4 * u64::from(tpq) / u64::from(meter.den);
    let bar = beat * u64::from(meter.num);
    let pickup = lead.div_ceil(beat) * beat;
    if *kind == Kind::Melody
        && pickup < bar
        && let Some(sig) = pickup_meter_for(pickup, tpq, meter)
    {
        return (LeadIn::Pickup(sig), pickup);
    }
    (LeadIn::RestBar, lead.div_ceil(bar) * bar)
}

/// The velocity of `strength` in a take whose strongest note or hit is `strongest`.
fn velocity(strength: f32, strongest: f32) -> u8 {
    let share = if strongest > 0.0 {
        (strength / strongest).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let span = f32::from(VELOCITY_TOP - VELOCITY_FLOOR);
    (f32::from(VELOCITY_FLOOR) + share * span).round() as u8
}

/// How far each note of `raw` moved in `quantized`, its copy quantized to `grid`. Notes are
/// paired by index, since quantizing keeps their order.
///
/// Milliseconds are read at `ms_per_quarter`, the length of a quarter note in the take: for a
/// captured take, `1000 * beat_period_frames / rate`, the measured period its notes were placed
/// by, not the whole bpm the song states.
///
/// Fails when `grid` cannot divide the songs' ticks per quarter, or a position is not in the
/// song, neither of which a song quantized to `grid` can have.
pub fn quantize_offsets(
    raw: &Song,
    quantized: &Song,
    grid: Grid,
    ms_per_quarter: f64,
) -> Result<Vec<QuantizeOffset>> {
    let step = grid.step_ticks(raw.ticks_per_quarter)? as f64;
    let ms_per_tick = ms_per_quarter / f64::from(raw.ticks_per_quarter);
    let (raw_map, quantized_map) = (raw.time_map()?, quantized.time_map()?);
    let notes = |song: &Song| -> Vec<Position> {
        song.tracks
            .iter()
            .flat_map(|t| t.notes.iter().map(|n| n.at))
            .collect()
    };
    notes(raw)
        .into_iter()
        .zip(notes(quantized))
        .enumerate()
        .map(|(note, (from, to))| {
            let moved = quantized_map.tick(to)? as f64 - raw_map.tick(from)? as f64;
            Ok(QuantizeOffset {
                note,
                moved_ms: moved * ms_per_tick,
                moved_steps: moved / step,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;
    use duet_dsp::{Given, Mono, Onsets, Origin, Stage, estimate};

    const RATE: u32 = 22_050;

    fn meter(s: &str) -> TimeSig {
        s.parse().unwrap()
    }

    fn key(s: &str) -> Key {
        s.parse().unwrap()
    }

    /// The frame `secs` into the take.
    fn frame(secs: f64) -> u64 {
        (secs * f64::from(RATE)).round() as u64
    }

    /// A timing given in full: `bpm`, `meter`, and the downbeat `downbeat` seconds in.
    ///
    /// `Timing` is duet-dsp's and non-exhaustive, so it is built the way the command line
    /// builds one: everything given, which `estimate` uses exactly and needs no onsets for.
    fn given(bpm: f64, meter_text: &str, downbeat: f64) -> Timing {
        let timing = estimate(
            &[],
            RATE,
            &Given {
                tempo_bpm: Some(bpm),
                meter: Some(meter(meter_text)),
                downbeat_secs: Some(downbeat),
            },
        )
        .unwrap();
        assert_eq!(timing.tempo_bpm, Origin::Given(bpm));
        assert_eq!(timing.tempo_alternatives, None);
        assert_eq!(timing.beat_period_frames, 60.0 * f64::from(RATE) / bpm);
        assert_eq!(timing.meter, Origin::Given(meter(meter_text)));
        assert_eq!(timing.downbeat_frame, Origin::Given(frame(downbeat)));
        timing
    }

    /// A note from `start` for `secs` seconds.
    fn note(start: f64, secs: f64, midi: u8, strength: f32) -> Event {
        Event::Note {
            start: frame(start),
            end: frame(start + secs),
            midi,
            confidence: 0.9,
            strength,
        }
    }

    fn hit(at: f64, lane: HitLane, strength: f32) -> Event {
        Event::Hit {
            frame: frame(at),
            lane,
            strength,
        }
    }

    fn beatbox() -> Kind {
        Kind::Rhythm {
            lanes: LaneMap::gm(),
            asked: None,
        }
    }

    fn starts(captured: &Captured) -> Vec<Position> {
        captured.song.tracks[0].notes.iter().map(|n| n.at).collect()
    }

    #[test]
    fn notes_land_where_the_tempo_puts_them() {
        let events = [note(1.0, 0.5, 60, 0.03), note(1.5, 0.25, 62, 0.03)];
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 1.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        assert_eq!(
            starts(&captured),
            [Position::new(1, 1, 0), Position::new(1, 2, 0)]
        );
        assert_eq!(captured.song.tracks[0].notes[0].beats, 1.0);
        assert_eq!(captured.song.tracks[0].notes[1].beats, 0.5);
        assert_eq!(captured.song.ticks_per_quarter, 480);
        assert_eq!(captured.song.tracks.len(), 1);
        assert_eq!(captured.song.tracks[0].name, "melody");
        assert_eq!(captured.lead_in, LeadIn::None);
    }

    #[test]
    fn a_given_tempo_is_the_songs_tempo() {
        let events = [note(1.0, 0.5, 60, 0.03)];
        let captured =
            assemble(&events, RATE, &given(97.0, "4/4", 1.0), None, &Kind::Melody).unwrap();
        assert_eq!(captured.song.tempo_bpm, 97.0);
    }

    #[test]
    fn notes_are_placed_by_the_measured_period_not_the_rounded_tempo() {
        // A click at 92.4 a minute from 0.25 s, and a note on every click.
        let lead = 0.25;
        let period = 60.0 / 92.4;
        let times: Vec<f64> = (0..45).map(|k| lead + f64::from(k) * period).collect();
        let signal = synth::clicks(RATE, 31.0, &times, 0.5);
        let mut stage = Onsets::new(RATE);
        let mut events = Vec::new();
        stage
            .push(
                Mono {
                    rate: RATE,
                    start_frame: 0,
                    samples: signal,
                },
                &mut events,
            )
            .unwrap();
        stage.finish(&mut events).unwrap();
        let timing = estimate(
            &events,
            RATE,
            &Given {
                meter: Some(meter("4/4")),
                downbeat_secs: Some(lead),
                ..Given::default()
            },
        )
        .unwrap();
        events.extend(times.iter().map(|&t| note(t, period / 2.0, 60, 0.03)));

        let captured = assemble(&events, RATE, &timing, None, &Kind::Melody).unwrap();
        assert_eq!(captured.song.tempo_bpm, 92.0);
        let map = captured.song.time_map().unwrap();
        let fortieth = map.tick(captured.song.tracks[0].notes[40].at).unwrap();
        assert!(
            fortieth.abs_diff(40 * 480) <= 1,
            "beat 40 is at tick {fortieth}"
        );
        // The stated 92 would have put it most of a sixteenth early.
        let by_stated = (times[40] - lead) * 92.0 / 60.0 * 480.0;
        assert!(40.0 * 480.0 - by_stated > 60.0, "{by_stated}");
    }

    #[test]
    fn a_pickup_that_fits_becomes_a_short_first_bar() {
        // One beat before the downbeat, at 120: half a second.
        let events = [note(0.5, 0.5, 67, 0.03), note(1.0, 2.0, 72, 0.03)];
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 1.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        assert_eq!(captured.lead_in, LeadIn::Pickup(meter("1/4")));
        assert_eq!(captured.song.time_sig, meter("1/4"));
        assert_eq!(captured.song.time_changes.len(), 1);
        assert_eq!(captured.song.time_changes[0].at, Position::bar(2));
        assert_eq!(captured.song.time_changes[0].time_sig, meter("4/4"));
        assert_eq!(starts(&captured), [Position::bar(1), Position::bar(2)]);
    }

    #[test]
    fn a_pickup_that_does_not_fit_becomes_a_bar_of_rests() {
        // Three and a half beats early in 4/4: a pickup that long would be a whole bar.
        let events = [note(0.25, 0.5, 67, 0.03), note(2.0, 1.0, 72, 0.03)];
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 2.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        assert_eq!(captured.lead_in, LeadIn::RestBar);
        assert_eq!(captured.song.time_sig, meter("4/4"));
        assert!(captured.song.time_changes.is_empty());
        assert_eq!(
            starts(&captured),
            [Position::new(1, 1, 240), Position::bar(2)]
        );
    }

    #[test]
    fn rhythm_always_gets_a_rest_bar_lead_in() {
        // One beat early, which would fit a pickup in a melody.
        let events = [
            hit(0.5, HitLane::Snare, 0.03),
            hit(1.0, HitLane::Kick, 0.03),
        ];
        let captured =
            assemble(&events, RATE, &given(120.0, "4/4", 1.0), None, &beatbox()).unwrap();
        assert_eq!(captured.lead_in, LeadIn::RestBar);
        assert_eq!(captured.song.time_sig, meter("4/4"));
        assert_eq!(
            starts(&captured),
            [Position::new(1, 4, 0), Position::bar(2)]
        );
    }

    #[test]
    fn nothing_is_dropped() {
        let melody = |first: f64| {
            vec![
                Event::Onset {
                    frame: frame(first),
                    strength: 0.03,
                },
                note(first, 0.2, 60, 0.03),
                note(2.0, 0.5, 62, 0.02),
                note(2.5, 0.5, 64, 0.01),
            ]
        };
        let rhythm = |first: f64| {
            vec![
                hit(first, HitLane::Hat, 0.02),
                hit(2.0, HitLane::Kick, 0.03),
                hit(2.25, HitLane::Hat, 0.01),
            ]
        };
        let timing = given(120.0, "4/4", 2.0);
        let cases = [
            (melody(2.0), Kind::Melody, LeadIn::None),
            (melody(1.5), Kind::Melody, LeadIn::Pickup(meter("1/4"))),
            (melody(0.1), Kind::Melody, LeadIn::RestBar),
            (rhythm(2.0), beatbox(), LeadIn::None),
            (rhythm(1.5), beatbox(), LeadIn::RestBar),
        ];
        for (events, kind, lead_in) in cases {
            let played = events
                .iter()
                .filter(|e| matches!(e, Event::Note { .. } | Event::Hit { .. }))
                .count();
            let captured = assemble(&events, RATE, &timing, None, &kind).unwrap();
            assert_eq!(captured.lead_in, lead_in);
            assert_eq!(captured.song.tracks[0].notes.len(), played, "{lead_in:?}");
            assert_eq!(captured.analysis.events, events);
            captured.song.check_end().unwrap();
            let mut normalized = captured.song.clone();
            normalized.normalize().unwrap();
            assert_eq!(normalized, captured.song, "{lead_in:?}: already normal");
        }
    }

    #[test]
    fn hits_map_to_their_lanes_keys_on_channel_10() {
        let events = [
            hit(1.0, HitLane::Kick, 0.03),
            hit(1.25, HitLane::Snare, 0.03),
            hit(1.5, HitLane::Hat, 0.03),
        ];
        let timing = given(120.0, "4/4", 1.0);
        let captured = assemble(&events, RATE, &timing, None, &beatbox()).unwrap();
        let drums = &captured.song.tracks[0];
        assert_eq!(drums.name, "drums");
        assert_eq!(drums.channel, 10);
        let keys: Vec<u8> = drums.notes.iter().map(|n| n.pitch.midi()).collect();
        assert_eq!(keys, [36, 38, 42]);
        assert_eq!(captured.analysis.key, None);

        let asked = [
            hit(1.0, HitLane::Asked, 0.03),
            hit(1.5, HitLane::Asked, 0.03),
        ];
        let tapped = Kind::Rhythm {
            lanes: LaneMap::gm(),
            asked: Some("hat".into()),
        };
        let captured = assemble(&asked, RATE, &timing, None, &tapped).unwrap();
        let keys: Vec<u8> = captured.song.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch.midi())
            .collect();
        assert_eq!(keys, [42, 42]);
    }

    #[test]
    fn an_asked_lane_the_map_lacks_is_refused_by_name() {
        let tapped = Kind::Rhythm {
            lanes: LaneMap::gm(),
            asked: Some("cowbell-xl".into()),
        };
        let events = [hit(1.0, HitLane::Asked, 0.03)];
        let err = assemble(&events, RATE, &given(120.0, "4/4", 1.0), None, &tapped).unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(msg) if msg.contains("cowbell-xl")),
            "{err}"
        );
    }

    #[test]
    fn hits_are_short_and_never_overlap_on_one_key() {
        // Two kicks 60 ms apart, closer than a sixteenth at 120 (125 ms).
        let events = [
            hit(1.0, HitLane::Kick, 0.03),
            hit(1.06, HitLane::Kick, 0.03),
        ];
        let captured =
            assemble(&events, RATE, &given(120.0, "4/4", 1.0), None, &beatbox()).unwrap();
        let notes = &captured.song.tracks[0].notes;
        assert_eq!(notes[1].beats, 0.25);
        let first_end = 480.0 * notes[0].beats;
        let second_start = f64::from(notes[1].at.tick);
        assert!(first_end <= second_start, "{notes:?}");
    }

    #[test]
    fn no_events_is_refused() {
        let onsets = [Event::Onset {
            frame: 100,
            strength: 0.03,
        }];
        let timing = given(120.0, "4/4", 0.0);
        let err = assemble(&onsets, RATE, &timing, None, &Kind::Melody).unwrap_err();
        assert!(
            matches!(err, Error::NothingFound { what: "notes" }),
            "{err}"
        );
        assert_eq!(err.to_string(), "no notes found");
        let err = assemble(&[], RATE, &timing, None, &beatbox()).unwrap_err();
        assert!(matches!(err, Error::NothingFound { what: "hits" }), "{err}");
    }

    #[test]
    fn velocity_is_relative_to_the_take() {
        let take = |level: f32| {
            [0.06_f32, 0.03, 0.015, 0.0]
                .iter()
                .enumerate()
                .map(|(i, &s)| note(1.0 + 0.5 * i as f64, 0.25, 60, s * level))
                .collect::<Vec<_>>()
        };
        let velocities = |events: &[Event]| {
            let captured =
                assemble(events, RATE, &given(120.0, "4/4", 1.0), None, &Kind::Melody).unwrap();
            captured.song.tracks[0]
                .notes
                .iter()
                .map(|n| n.vel)
                .collect::<Vec<_>>()
        };
        let loud = velocities(&take(1.0));
        assert_eq!(loud[0], 127, "the strongest note is the loudest");
        assert_eq!(loud[3], 20, "no note falls below the floor");
        assert!(loud[1] > loud[2] && loud[2] > loud[3], "{loud:?}");
        assert_eq!(
            velocities(&take(0.37)),
            loud,
            "a quieter take keeps its dynamics"
        );
        assert_eq!(velocities(&take(4.0)), loud);
    }

    #[test]
    fn a_given_key_is_the_songs_key() {
        // A C major scale, which would be estimated as C major.
        let events: Vec<Event> = [60, 62, 64, 65, 67, 69, 71, 72]
            .iter()
            .enumerate()
            .map(|(i, &m)| note(1.0 + 0.5 * i as f64, 0.5, m, 0.03))
            .collect();
        let eb = Key::new(-3, false).unwrap();
        let timing = given(120.0, "4/4", 1.0);
        let captured = assemble(&events, RATE, &timing, Some(eb), &Kind::Melody).unwrap();
        assert_eq!(captured.song.key, Some(eb));
        let estimate = captured.analysis.key.as_ref().unwrap();
        assert_eq!(estimate.key, Origin::Given(eb));

        let unasked = assemble(&events, RATE, &timing, None, &Kind::Melody).unwrap();
        assert_eq!(unasked.song.key, Some(key("C major")));
    }

    #[test]
    fn the_key_changes_spelling_never_pitch() {
        // Eb F G Ab Bb C D Eb.
        let midi = [63, 65, 67, 68, 70, 72, 74, 75];
        let events: Vec<Event> = midi
            .iter()
            .enumerate()
            .map(|(i, &m)| note(1.0 + 0.5 * i as f64, 0.5, m, 0.03))
            .collect();
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 1.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        assert_eq!(captured.song.key, Some(Key::new(-3, false).unwrap()));
        let notes = &captured.song.tracks[0].notes;
        let got: Vec<u8> = notes.iter().map(|n| n.pitch.midi()).collect();
        assert_eq!(got, midi);
        let spelled: Vec<String> = notes.iter().map(|n| n.pitch.to_string()).collect();
        assert_eq!(
            spelled,
            ["Eb4", "F4", "G4", "Ab4", "Bb4", "C5", "D5", "Eb5"]
        );
    }

    #[test]
    fn a_rhythm_take_has_no_key() {
        let events = [hit(1.0, HitLane::Kick, 0.03)];
        let timing = given(120.0, "4/4", 1.0);
        let err = assemble(&events, RATE, &timing, Some(key("Eb major")), &beatbox()).unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(msg) if msg.contains("key")),
            "{err}"
        );
    }

    #[test]
    fn the_analysis_holds_the_events_timing_and_beats() {
        let mut events: Vec<Event> = (0..12)
            .map(|k| Event::Onset {
                frame: frame(1.0 + 0.5 * f64::from(k)),
                strength: 0.03,
            })
            .collect();
        events.push(note(1.0, 0.5, 60, 0.03));
        let timing = given(120.0, "4/4", 1.0);
        let captured = assemble(&events, RATE, &timing, None, &Kind::Melody).unwrap();
        let analysis = &captured.analysis;
        assert_eq!(analysis.rate, RATE);
        assert_eq!(analysis.events, events);
        assert_eq!(analysis.timing, timing);
        assert_eq!(captured.timing, timing);
        assert_eq!(
            analysis.beats,
            duet_dsp::beat_series(&events, RATE, &timing)
        );
        assert!(!analysis.beats.is_empty());
        assert!(analysis.offsets.is_empty());
        assert!(analysis.key.is_some());
    }

    #[test]
    fn quantize_offsets_say_how_far_each_note_moved() {
        // At 120 a sixteenth is 125 ms: one note 30 ms after the second sixteenth, one on it.
        let events = [note(1.155, 0.1, 60, 0.03), note(1.25, 0.1, 62, 0.03)];
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 1.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        let grid: Grid = "1/16".parse().unwrap();
        let quantized = captured.song.quantized(grid).unwrap();
        let quarter_ms = 1000.0 * captured.timing.beat_period_frames / f64::from(RATE);
        let offsets = quantize_offsets(&captured.song, &quantized, grid, quarter_ms).unwrap();
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets[0].note, 0);
        assert!((offsets[0].moved_ms + 30.0).abs() < 1.0, "{offsets:?}");
        assert!((offsets[0].moved_steps + 0.24).abs() < 0.01, "{offsets:?}");
        assert_eq!(offsets[1].note, 1);
        assert_eq!(offsets[1].moved_ms, 0.0);
        assert_eq!(offsets[1].moved_steps, 0.0);
    }

    #[test]
    fn quantize_offsets_are_timed_by_the_measured_period_not_the_stated_tempo() {
        // Stated as 120, measured at 122: the note is placed, and its move timed, at 122.
        let mut timing = given(120.0, "4/4", 1.0);
        timing.tempo_bpm = Origin::Estimated {
            value: 120.0,
            confidence: 0.9,
        };
        timing.beat_period_frames = 60.0 * f64::from(RATE) / 122.0;
        let events = [note(1.155, 0.1, 60, 0.03)];
        let captured = assemble(&events, RATE, &timing, None, &Kind::Melody).unwrap();
        let grid: Grid = "1/16".parse().unwrap();
        let quantized = captured.song.quantized(grid).unwrap();
        let quarter_ms = 1000.0 * timing.beat_period_frames / f64::from(RATE);
        let offsets = quantize_offsets(&captured.song, &quantized, grid, quarter_ms).unwrap();
        // 0.155 s at 122 is 151 ticks, which quantizes to 120: 31 ticks earlier.
        let at_122 = -31.0 * 60_000.0 / 122.0 / 480.0;
        assert!((offsets[0].moved_ms - at_122).abs() < 1e-9, "{offsets:?}");
        let at_120 = -31.0 * 60_000.0 / 120.0 / 480.0;
        assert!((offsets[0].moved_ms - at_120).abs() > 0.4, "{offsets:?}");
    }

    #[test]
    fn the_downbeat_tick_is_where_bar_1_of_the_take_starts() {
        let timing = given(120.0, "4/4", 2.0);
        let take = |first: f64| [note(first, 0.2, 60, 0.03), note(2.0, 0.5, 62, 0.03)];
        let cases = [
            (2.0, Kind::Melody, LeadIn::None, 0),
            (1.5, Kind::Melody, LeadIn::Pickup(meter("1/4")), 480),
            (0.25, Kind::Melody, LeadIn::RestBar, 1920),
        ];
        for (first, kind, lead_in, tick) in cases {
            let captured = assemble(&take(first), RATE, &timing, None, &kind).unwrap();
            assert_eq!(captured.lead_in, lead_in);
            assert_eq!(captured.downbeat_tick, tick, "{lead_in:?}");
            let map = captured.song.time_map().unwrap();
            assert_eq!(map.tick(captured.song.tracks[0].notes[1].at).unwrap(), tick);
        }

        // A rhythm starting a bar and a half early gets two rest bars.
        let hits = [
            hit(0.25, HitLane::Hat, 0.03),
            hit(3.25, HitLane::Kick, 0.03),
        ];
        let captured = assemble(&hits, RATE, &given(120.0, "4/4", 3.25), None, &beatbox()).unwrap();
        assert_eq!(captured.lead_in, LeadIn::RestBar);
        assert_eq!(captured.downbeat_tick, 2 * 1920);
        assert_eq!(
            starts(&captured),
            [Position::new(1, 3, 0), Position::bar(3)]
        );
    }

    #[test]
    fn a_lead_in_of_part_of_a_beat_is_carried_to_a_whole_beat() {
        // 1.3 beats before the downbeat at 120 is 0.65 s: a pickup of two beats, the first note
        // 0.7 of a beat (336 ticks) into it.
        let events = [note(1.35, 0.2, 67, 0.03), note(2.0, 1.0, 72, 0.03)];
        let captured = assemble(
            &events,
            RATE,
            &given(120.0, "4/4", 2.0),
            None,
            &Kind::Melody,
        )
        .unwrap();
        assert_eq!(captured.lead_in, LeadIn::Pickup(meter("2/4")));
        assert_eq!(captured.downbeat_tick, 960);
        assert_eq!(
            starts(&captured),
            [Position::new(1, 1, 336), Position::bar(2)]
        );
    }

    #[test]
    fn no_note_or_hit_has_no_length() {
        // Two kicks a frame apart land on one tick; a note shorter than half a tick rounds to
        // none.
        let timing = given(120.0, "4/4", 1.0);
        let start = frame(1.0);
        let hits = [
            Event::Hit {
                frame: start,
                lane: HitLane::Kick,
                strength: 0.03,
            },
            Event::Hit {
                frame: start + 1,
                lane: HitLane::Kick,
                strength: 0.03,
            },
        ];
        let captured = assemble(&hits, RATE, &timing, None, &beatbox()).unwrap();
        let notes = &captured.song.tracks[0].notes;
        assert_eq!(notes[0].at, notes[1].at, "they share a tick");
        assert!(notes.iter().all(|n| n.beats > 0.0), "{notes:?}");

        let blip = [Event::Note {
            start,
            end: start + 2,
            midi: 60,
            confidence: 0.9,
            strength: 0.03,
        }];
        let captured = assemble(&blip, RATE, &timing, None, &Kind::Melody).unwrap();
        let beats = captured.song.tracks[0].notes[0].beats;
        assert_eq!(duet_core::time::ticks_from_beats(beats, 480).unwrap(), 1);
    }
}
