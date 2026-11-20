//! A song as one [`Part`] on its composition's clock: every placed part at its placement, each
//! repeat back to back, under the composition's tempo, meter and key maps. `duet play` plays
//! what comes back through the same path as any part, so a song sounds, and goes to a port,
//! exactly as a part does.
//!
//! The rules are the songs spec's §1, "The composition's maps and a part's own conductor":
//!
//! - **The composition is the clock.** A placed part's notes are read in bars and beats from
//!   its own `1.1.0`, quarter note for quarter note, and played at the composition's tempo
//!   map. Its own tempo is not applied.
//! - **A disagreement is stated, never fixed.** A part whose own tempo, meter or key differs
//!   from the song's where it is placed, two parts that share a channel with different
//!   programs while both play, and anything a placement or the song's stated end leaves out,
//!   are each said in [`Arranged::stated`]. The part is never rewritten to match.
//!
//! A placement's stretch is its `from` to its `to`, in the part's own positions, or the whole
//! part, from `1.1.0` to its stated end. Each repeat starts where the one before it ends. A
//! note that starts in the stretch and runs past its `to` is cut at `to`, and said, so one
//! repeat never sounds over the next.

use duet_core::time::{beats_from_ticks, ticks_from_beats};
use duet_core::{Control, KeyChange, Part, Position, TempoChange, TimeChange, TimeMap, Track};
use duet_schemas::Song;
use duet_schemas::project::Name;

/// A song arranged as one part, and what was stated arranging it.
#[derive(Debug, Clone, PartialEq)]
pub struct Arranged {
    /// The song on its composition's clock: one track for each track of each placement, every
    /// repeat in it.
    pub part: Part,
    /// Each disagreement, and each thing left out, in the order of the placements, one line
    /// each.
    pub stated: Vec<String>,
}

/// One placement's track, as it lands on the song's ticks: for the channel check.
struct Span {
    part: Name,
    channel: u8,
    program: Option<u8>,
    start: u64,
    end: u64,
}

/// Arranges `song` from its placed parts, each by its name. Every placement must name one of
/// `parts`; [`Song::check_parts`] says so by name first.
///
/// # Errors
///
/// A placement naming a part that is not in `parts`, a stretch that runs past its part's end
/// or plays nothing, or a song that places nothing, each by name.
pub fn arrange(song: &Song, parts: &[(Name, Part)]) -> Result<Arranged, String> {
    let comp = &song.composition;
    let map = comp.time_map()?;
    let tpq = comp.ticks_per_quarter;
    let tick_of = |at: Position| map.tick(at).map_err(|e| e.to_string());
    if comp.placements.is_empty() {
        return Err(
            "the song places no part, so it has nothing to play: place one in \
             composition.placements"
                .into(),
        );
    }

    let mut out = Part::default();
    out.title = Some(song.title.clone());
    out.ticks_per_quarter = tpq;
    // The composition's maps start at 1.1.0 (Composition::check): the first of each is the
    // part's start, the rest its changes.
    let (first_tempo, tempos) = comp
        .tempo
        .split_first()
        .ok_or("composition.tempo is empty")?;
    out.tempo_bpm = first_tempo.bpm;
    let (first_meter, meters) = comp
        .meter
        .split_first()
        .ok_or("composition.meter is empty")?;
    out.time_sig = first_meter.sig;
    out.key = comp.key.first().map(|k| k.key);

    let mut stated = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut end_tick = 0u64;
    for (n, placement) in comp.placements.iter().enumerate() {
        let name = &placement.part;
        let field = format!("composition.placements[{n}]");
        let part = parts
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, part)| part)
            .ok_or_else(|| format!("{field} places the part {name:?}, which is not read"))?;
        let pmap = part.time_map().map_err(|e| format!("{name}: {e}"))?;
        let ptpq = part.ticks_per_quarter;
        let ptick = |at: Position| pmap.tick(at).map_err(|e| format!("{name}: {e}"));
        let part_end = part.stated_end().map_err(|e| format!("{name}: {e}"))?;
        let from = placement.from.unwrap_or(Position::bar(1));
        let to = placement.to.unwrap_or(part_end);
        let (from_t, to_t, end_t) = (ptick(from)?, ptick(to)?, ptick(part_end)?);
        if to_t > end_t {
            return Err(format!(
                "{field} plays {name} to {to}, past its end at {part_end}: a placement plays \
                 a stretch of the part"
            ));
        }
        if to_t <= from_t {
            return Err(format!(
                "{field} plays {name} from {from} to {to}, which holds nothing: `to` is after \
                 `from`"
            ));
        }
        let span = to_t - from_t;
        let repeats = u64::from(placement.repeat.unwrap_or(1));
        let start = tick_of(placement.at)?;
        let mut rounded = 0usize;
        let mut song_tick = |part_tick: u64| {
            let (t, exact) = rescale(part_tick, ptpq, tpq);
            if !exact {
                rounded += 1;
            }
            t
        };
        let placed_end = start + song_tick(span * repeats);
        end_tick = end_tick.max(placed_end);

        stated.extend(conductor_disagreements(
            song,
            &map,
            part,
            name,
            placement.at,
            start,
        ));

        let mut cut = 0usize;
        for track in &part.tracks {
            let mut placed = Track::new(format!("{name}: {}", track.name), track.channel);
            for r in 0..repeats {
                let base = r * span;
                let last = r + 1 == repeats;
                // Each repeat sets the track's program as it starts, so a part placed after
                // another on its channel sounds as it was written.
                if let Some(program) = track.program {
                    placed.controls.push(Control::Program {
                        at: map.position(start + song_tick(base)),
                        program,
                    });
                }
                for note in &track.notes {
                    let t = ptick(note.at)?;
                    if t < from_t || t >= to_t {
                        continue;
                    }
                    let len = ticks_from_beats(note.beats, ptpq).map_err(|e| e.to_string())?;
                    let held = if t + len > to_t {
                        cut += 1;
                        to_t - t
                    } else {
                        len
                    };
                    let at = start + song_tick(base + t - from_t);
                    let until = start + song_tick(base + t - from_t + held);
                    let mut note = note.clone();
                    note.at = map.position(at);
                    note.beats = beats_from_ticks(until - at, tpq);
                    placed.notes.push(note);
                }
                for control in &track.controls {
                    let t = ptick(control.at())?;
                    // A control on `to` belongs to the next repeat's start, so only the last
                    // repeat plays it.
                    if t < from_t || t > to_t || (t == to_t && !last) {
                        continue;
                    }
                    let mut control = control.clone();
                    let at = map.position(start + song_tick(base + t - from_t));
                    match &mut control {
                        Control::Cc { at: a, .. }
                        | Control::Bend { at: a, .. }
                        | Control::Program { at: a, .. } => *a = at,
                        _ => continue,
                    }
                    placed.controls.push(control);
                }
            }
            spans.push(Span {
                part: name.clone(),
                channel: track.channel,
                program: track.program,
                start,
                end: placed_end,
            });
            out.tracks.push(placed);
        }
        if cut > 0 {
            let each = cut / usize::try_from(repeats).unwrap_or(1).max(1);
            stated.push(format!(
                "{name}: {} past {to}, where the stretch {field} plays ends, and each is cut \
                 there, so a repeat never sounds over the next",
                notes(each, "note runs", "notes run")
            ));
        }
        if rounded > 0 {
            stated.push(format!(
                "{name} counts {ptpq} ticks to a quarter note and the song {tpq}: {} between \
                 the song's ticks and moved to the nearest",
                notes(rounded, "place falls", "places fall")
            ));
        }
    }

    stated.extend(shared_channels(&spans));

    // The song's end: composition.end when it says, or else the latest end of any placement.
    let end_tick = match comp.end {
        Some(end) => {
            let stated_end = tick_of(end)?;
            if end_tick > stated_end {
                let dropped = drop_past(&mut out, &map, stated_end, tpq)?;
                let mut left_out = Vec::new();
                if dropped.0 > 0 {
                    left_out.push(format!(
                        "{} not played",
                        notes(dropped.0, "note is", "notes are")
                    ));
                }
                if dropped.1 > 0 {
                    left_out.push(format!(
                        "{} cut at it",
                        notes(dropped.1, "note is", "notes are")
                    ));
                }
                if left_out.is_empty() {
                    left_out.push("nothing sounds past it".to_owned());
                }
                stated.push(format!(
                    "the song ends at {end} (composition.end), and what is placed plays on to \
                     {}: {}",
                    map.position(end_tick),
                    left_out.join(", and ")
                ));
            }
            stated_end
        }
        None => end_tick,
    };

    for change in tempos {
        if tick_of(change.at)? <= end_tick {
            out.tempo_changes.push(TempoChange {
                at: change.at,
                bpm: change.bpm,
            });
        }
    }
    for change in meters {
        if tick_of(change.at)? <= end_tick {
            out.time_changes.push(TimeChange {
                at: change.at,
                time_sig: change.sig,
            });
        }
    }
    for change in comp.key.iter().skip(1) {
        if tick_of(change.at)? <= end_tick {
            out.key_changes.push(KeyChange {
                at: change.at,
                key: change.key,
            });
        }
    }
    out.end = Some(map.position(end_tick));
    Ok(Arranged { part: out, stated })
}

/// `n` things, in the singular or plural: "1 note is", "3 notes are".
fn notes(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// `tick` counted at `from` ticks to a quarter note, at `to`: rounded to the nearest, and
/// whether it was exact.
fn rescale(tick: u64, from: u16, to: u16) -> (u64, bool) {
    if from == to {
        return (tick, true);
    }
    let (from, to) = (u128::from(from), u128::from(to));
    let scaled = u128::from(tick) * to;
    let exact = scaled % from == 0;
    let rounded = (scaled + from / 2) / from;
    (u64::try_from(rounded).unwrap_or(u64::MAX), exact)
}

/// Where a part's own tempo, meter or key differs from the song's where it is placed, and each
/// change of its own the song's maps play over.
fn conductor_disagreements(
    song: &Song,
    map: &TimeMap,
    part: &Part,
    name: &Name,
    at: Position,
    start: u64,
) -> Vec<String> {
    let comp = &song.composition;
    let mut out = Vec::new();
    let in_force = |ats: &mut dyn Iterator<Item = Position>| -> Option<usize> {
        ats.enumerate()
            .filter(|(_, p)| map.tick(*p).is_ok_and(|t| t <= start))
            .map(|(i, _)| i)
            .last()
    };
    if let Some(i) = in_force(&mut comp.tempo.iter().map(|c| c.at)) {
        let bpm = comp.tempo[i].bpm;
        if (part.tempo_bpm - bpm).abs() > 1e-6 {
            out.push(format!(
                "{name} is at {} bpm, and the song is at {} bpm at {at}: it plays at the \
                 song's tempo",
                part.tempo_bpm, bpm
            ));
        }
    }
    if let Some(first) = part.tempo_changes.first() {
        out.push(format!(
            "{name} changes its own tempo ({}, the first at {} to {} bpm): the song's tempo map \
             plays instead",
            notes(part.tempo_changes.len(), "change", "changes"),
            first.at,
            first.bpm
        ));
    }
    let sig = map.sig_at(start);
    if part.time_sig != sig {
        out.push(format!(
            "{name} is in {}, and the song is in {sig} at {at}: its notes play beat for beat, \
             so its bar lines fall off the song's",
            part.time_sig
        ));
    }
    if let Some(first) = part.time_changes.first() {
        out.push(format!(
            "{name} changes its own meter ({}, the first at {} to {}): the song's meter map \
             is the one its bars are read on",
            notes(part.time_changes.len(), "change", "changes"),
            first.at,
            first.time_sig
        ));
    }
    if let (Some(key), Some(i)) = (part.key, in_force(&mut comp.key.iter().map(|c| c.at))) {
        let song_key = comp.key[i].key;
        if key != song_key {
            out.push(format!(
                "{name} is in {key}, and the song is in {song_key} at {at}: no note is changed"
            ));
        }
    }
    if let Some(first) = part.key_changes.first() {
        out.push(format!(
            "{name} changes its own key ({}, the first at {} to {}): no note is changed",
            notes(part.key_changes.len(), "change", "changes"),
            first.at,
            first.key
        ));
    }
    out
}

/// Two parts that sound on one channel at once with different programs: one program is heard
/// for both, so it is said, naming them.
fn shared_channels(spans: &[Span]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in spans.iter().enumerate() {
        for b in &spans[i + 1..] {
            if a.part != b.part
                && a.channel == b.channel
                && a.program != b.program
                && a.start < b.end
                && b.start < a.end
            {
                let program =
                    |p: Option<u8>| p.map_or_else(|| "none".to_owned(), |p| p.to_string());
                let line = format!(
                    "{} and {} both play on channel {} at once, with programs {} and {}: the \
                     one set last is heard for both",
                    a.part,
                    b.part,
                    a.channel,
                    program(a.program),
                    program(b.program)
                );
                if !out.contains(&line) {
                    out.push(line);
                }
            }
        }
    }
    out
}

/// Drops every note starting at or after `end`, cuts each one sounding past it, and drops each
/// control after it. Returns how many notes were dropped and how many cut.
fn drop_past(out: &mut Part, map: &TimeMap, end: u64, tpq: u16) -> Result<(usize, usize), String> {
    let (mut gone, mut cut) = (0, 0);
    for track in &mut out.tracks {
        let before = track.notes.len();
        let mut kept = Vec::with_capacity(before);
        for mut note in std::mem::take(&mut track.notes) {
            let at = map.tick(note.at).map_err(|e| e.to_string())?;
            if at >= end {
                continue;
            }
            let len = ticks_from_beats(note.beats, tpq).map_err(|e| e.to_string())?;
            if at + len > end {
                note.beats = beats_from_ticks(end - at, tpq);
                cut += 1;
            }
            kept.push(note);
        }
        gone += before - kept.len();
        track.notes = kept;
        let mut controls = Vec::new();
        for control in std::mem::take(&mut track.controls) {
            if map.tick(control.at()).map_err(|e| e.to_string())? <= end {
                controls.push(control);
            }
        }
        track.controls = controls;
    }
    Ok((gone, cut))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use duet_core::{Part, Position};
    use duet_play::Schedule;
    use duet_schemas::Song;
    use duet_schemas::project::Name;
    use duet_schemas::song::{Composition, Placement, TempoAt};

    use super::arrange;

    fn name(s: &str) -> Name {
        s.parse().expect("a name")
    }

    fn pos(s: &str) -> Position {
        s.parse().expect("a position")
    }

    fn song(composition: Composition) -> Song {
        Song::new(
            "01a1f16d-dc03-7162-8f59-e240692e579e"
                .parse()
                .expect("an id"),
            "Porch",
            composition,
        )
    }

    /// A one-bar groove in 4/4 at its own 120 bpm: a kick on beats 1 and 3, a hat on 2 and 4.
    fn kit() -> Part {
        Part::from_text(
            "tempo 120\ntime 4/4\nend 2.1.0\n\n[track \"Kit\" ch10]\n\
             1.1.0 C2 0.25 100\n1.2.0 F#2 0.25 70\n1.3.0 C2 0.25 100\n1.4.0 F#2 0.25 70\n",
        )
        .expect("a part")
    }

    fn placed(part: &str, at: &str, repeat: Option<u32>) -> Placement {
        let mut placement = Placement::new(name(part), pos(at));
        placement.repeat = repeat;
        placement
    }

    /// Note-ons of a schedule, as (tick, seconds, key).
    fn ons(schedule: &Schedule) -> Vec<(u64, Duration, u8)> {
        schedule
            .events()
            .iter()
            .filter(|e| e.message.kind() == 0x90)
            .map(|e| (e.tick, e.at, e.message.bytes()[1]))
            .collect()
    }

    /// The spec's witness: a part placed at bar 9 with `repeat: 2` sounds its notes at bars 9
    /// and 10, at the composition's tempo, not its own.
    #[test]
    fn a_part_placed_at_bar_9_twice_sounds_at_bars_9_and_10_at_the_songs_tempo() {
        let mut comp = Composition::new(90.0, "4/4".parse().expect("a meter"), None);
        comp.placements = vec![placed("kit", "9.1.0", Some(2))];
        let arranged = arrange(&song(comp), &[(name("kit"), kit())]).expect("arranged");

        let at: Vec<String> = arranged.part.tracks[0]
            .notes
            .iter()
            .map(|n| n.at.to_string())
            .collect();
        assert_eq!(
            at,
            [
                "9.1.0", "9.2.0", "9.3.0", "9.4.0", "10.1.0", "10.2.0", "10.3.0", "10.4.0"
            ]
        );
        assert_eq!(arranged.part.end, Some(pos("11.1.0")));
        assert_eq!(arranged.part.tempo_bpm, 90.0);

        let schedule = Schedule::from_part(&arranged.part).expect("a schedule");
        let ons = ons(&schedule);
        // At 90 bpm a beat is 666,667 µs as MIDI stores it, so bar 9 (32 beats in) starts at
        // 21.333 s; at the kit's own 120 bpm it would start at 16 s.
        let beat = Duration::from_micros(666_667);
        let want: Vec<(u64, Duration)> = (32u32..40)
            .map(|b| (u64::from(b) * 480, beat * b))
            .collect();
        assert_eq!(
            ons.iter()
                .map(|(tick, at, _)| (*tick, *at))
                .collect::<Vec<_>>(),
            want
        );
        assert_eq!(
            ons.iter().map(|(.., key)| *key).collect::<Vec<_>>(),
            [36, 42, 36, 42, 36, 42, 36, 42]
        );
        // Nothing sounds before bar 9.
        assert!(
            schedule
                .events()
                .iter()
                .filter(|e| e.message.kind() == 0x90)
                .all(|e| e.tick >= 32 * 480)
        );
        assert_eq!(schedule.end(), beat * 40);
        // The kit's own 120 bpm disagrees with the song's 90, and is said.
        assert_eq!(
            arranged.stated,
            ["kit is at 120 bpm, and the song is at 90 bpm at 9.1.0: it plays at the song's tempo"]
        );
    }

    /// A tempo change in the composition's map changes when a placed part's notes sound, and
    /// not where they fall on the bars.
    #[test]
    fn the_composition_tempo_map_is_the_clock() {
        let mut comp = Composition::new(120.0, "4/4".parse().expect("a meter"), None);
        comp.tempo.push(TempoAt::new(pos("2.1.0"), 60.0));
        comp.placements = vec![placed("kit", "1.1.0", Some(2))];
        let arranged = arrange(&song(comp), &[(name("kit"), kit())]).expect("arranged");
        assert!(arranged.stated.is_empty(), "{:?}", arranged.stated);
        let schedule = Schedule::from_part(&arranged.part).expect("a schedule");
        let secs: Vec<f64> = ons(&schedule)
            .iter()
            .map(|(_, at, _)| at.as_secs_f64())
            .collect();
        // Bar 1 at 120 bpm, half a second a beat; bar 2 at 60 bpm, a second a beat.
        let want = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0];
        for (got, want) in secs.iter().zip(want) {
            assert!((got - want).abs() < 1e-6, "{secs:?}");
        }
    }

    /// Layered parts each sound at their own placement; `from` and `to` play a stretch, and a
    /// note running past `to` is cut there and said.
    #[test]
    fn layers_and_stretches_land_where_placed() {
        let mut comp = Composition::new(120.0, "3/4".parse().expect("a meter"), None);
        let mut bass = placed("bass", "3.1.0", Some(2));
        bass.from = Some(pos("2.1.0"));
        bass.to = Some(pos("3.1.0"));
        comp.placements = vec![placed("melody", "1.1.0", None), bass];
        let melody = Part::from_text(
            "tempo 120\ntime 3/4\n\n[track \"Voice\" ch1 prog52]\n1.1.0 G4 3 90\n2.1.0 A4 3 90\n",
        )
        .expect("melody");
        let bass_part = Part::from_text(
            "tempo 120\ntime 3/4\n\n[track \"Bass\" ch2 prog32]\n\
             1.1.0 G2 3 80\n2.1.0 D2 1 80\n2.3.0 E2 2 80\n",
        )
        .expect("bass");
        let arranged = arrange(
            &song(comp),
            &[(name("melody"), melody), (name("bass"), bass_part)],
        )
        .expect("arranged");
        let notes = |i: usize| -> Vec<(String, f64)> {
            arranged.part.tracks[i]
                .notes
                .iter()
                .map(|n| (n.at.to_string(), n.beats))
                .collect()
        };
        assert_eq!(notes(0), [("1.1.0".into(), 3.0), ("2.1.0".into(), 3.0)]);
        assert_eq!(
            notes(1),
            [
                ("3.1.0".into(), 1.0),
                ("3.3.0".into(), 1.0),
                ("4.1.0".into(), 1.0),
                ("4.3.0".into(), 1.0)
            ]
        );
        assert_eq!(arranged.part.end, Some(pos("5.1.0")));
        assert_eq!(arranged.part.tracks[1].name, "bass: Bass");
        assert_eq!(
            arranged.stated,
            [
                "bass: 1 note runs past 3.1.0, where the stretch composition.placements[1] plays \
              ends, and each is cut there, so a repeat never sounds over the next"
            ]
        );
    }

    /// A part's own meter and key that differ from the song's are said, and the part is played
    /// beat for beat, unchanged.
    #[test]
    fn a_meter_or_key_that_disagrees_is_said_and_nothing_is_changed() {
        let mut comp = Composition::new(
            92.0,
            "3/4".parse().expect("a meter"),
            Some("G minor".parse().expect("a key")),
        );
        comp.placements = vec![placed("bass", "9.1.0", None)];
        let bass = Part::from_text(
            "tempo 92\ntime 4/4\nkey A minor\n\n[track \"Bass\" ch2]\n1.1.0 A2 4 80\n",
        )
        .expect("bass");
        let arranged = arrange(&song(comp), &[(name("bass"), bass)]).expect("arranged");
        assert_eq!(
            arranged.stated,
            [
                "bass is in 4/4, and the song is in 3/4 at 9.1.0: its notes play beat for beat, \
                 so its bar lines fall off the song's",
                "bass is in A minor, and the song is in G minor at 9.1.0: no note is changed",
            ]
        );
        let note = &arranged.part.tracks[0].notes[0];
        assert_eq!((note.at.to_string(), note.beats), ("9.1.0".into(), 4.0));
        // Four beats of bass in 3/4: it ends a beat into bar 10.
        assert_eq!(arranged.part.end, Some(pos("10.2.0")));
    }

    /// Two parts sounding on one channel at once with different programs are said; the stated
    /// end of the song leaves out what lies past it, and says so.
    #[test]
    fn a_shared_channel_and_the_songs_stated_end_are_said() {
        let mut comp = Composition::new(120.0, "4/4".parse().expect("a meter"), None);
        comp.placements = vec![placed("a", "1.1.0", Some(4)), placed("b", "2.1.0", None)];
        comp.end = Some(pos("3.1.0"));
        let a = Part::from_text("tempo 120\ntime 4/4\n\n[track \"A\" ch1 prog0]\n1.1.0 C4 4 80\n")
            .expect("a");
        let b = Part::from_text("tempo 120\ntime 4/4\n\n[track \"B\" ch1 prog40]\n1.1.0 E4 4 80\n")
            .expect("b");
        let arranged = arrange(&song(comp), &[(name("a"), a), (name("b"), b)]).expect("arranged");
        assert_eq!(
            arranged.stated,
            [
                "a and b both play on channel 1 at once, with programs 0 and 40: the one set \
                 last is heard for both",
                "the song ends at 3.1.0 (composition.end), and what is placed plays on to \
                 5.1.0: 2 notes are not played",
            ]
        );
        assert_eq!(arranged.part.end, Some(pos("3.1.0")));
        assert_eq!(arranged.part.tracks[0].notes.len(), 2);
        Schedule::from_part(&arranged.part).expect("it plays");
    }

    /// A song that places nothing has nothing to play, and says so.
    #[test]
    fn a_song_that_places_nothing_is_refused() {
        let comp = Composition::new(120.0, "4/4".parse().expect("a meter"), None);
        let err = arrange(&song(comp), &[]).expect_err("nothing placed");
        assert!(err.starts_with("the song places no part"), "{err}");
    }
}
