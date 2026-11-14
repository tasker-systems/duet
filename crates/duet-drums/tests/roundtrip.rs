//! Round-trips of drum grids: grid → song → grid under a lane map, through MIDI and the
//! exact forms, text → grid → text, and the refusals that keep every conversion exact.

use duet_core::{Grid, Note, Position, Song, TimeChange, TimeSig, Track};
use duet_drums::{Bar, DrumGrid, Lane, LaneMap, Levels};
use duet_midi::Midi;

/// A small deterministic generator, so every run tests the same grids.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The shapes the generated grids take: resolution, meter, an optional meter change at
/// bar 3, the step, and the bar count.
fn shapes() -> Vec<(u16, &'static str, Option<&'static str>, &'static str, u32)> {
    vec![
        (480, "4/4", None, "1/16", 4),
        (96, "4/4", None, "1/16", 2),
        (960, "4/4", Some("3/4"), "1/8t", 5),
        (480, "6/8", None, "1/16", 3),
        (480, "7/8", Some("4/4"), "1/8", 4),
        (192, "5/4", None, "1/8", 2),
    ]
}

/// A grid under `map` with random velocities 1 to 127 (not only the text levels), on
/// lanes taken in the map's row order, each hit at least once.
fn random_grid(
    rng: &mut Lcg,
    map: &LaneMap,
    shape: (u16, &str, Option<&str>, &str, u32),
) -> DrumGrid {
    let (tpq, sig, change, step, bars) = shape;
    let mut conductor = Song {
        ticks_per_quarter: tpq,
        tempo_bpm: 96.0,
        time_sig: sig.parse().expect("meter"),
        ..Song::default()
    };
    if let Some(c) = change {
        conductor.time_changes.push(TimeChange {
            at: Position::bar(3),
            time_sig: c.parse().expect("meter"),
        });
    }
    let mut grid = DrumGrid::with_conductor(&conductor, step.parse().expect("grid"));
    grid.lanes = map
        .lanes()
        .filter(|_| rng.below(4) == 0)
        .take(8)
        .map(|l| Lane {
            name: l.name.clone(),
            key: l.key,
        })
        .collect();
    if grid.lanes.is_empty() {
        let kick = map.lookup("kick").expect("gm has a kick");
        grid.lanes.push(Lane {
            name: kick.name.clone(),
            key: kick.key,
        });
    }
    // Build the bars with the library's own shapes: an empty grid of `bars` bars, read
    // back from a song with no notes, gives each bar's step count.
    let empty = DrumGrid::from_song(
        &Song {
            tracks: vec![Track::new("Drums", 10)],
            ..conductor.clone()
        },
        grid.grid,
        map,
        None,
        Some(bars),
    )
    .expect("an empty drum track fits any whole-step grid");
    grid.bars = empty
        .bars
        .iter()
        .map(|b| Bar {
            bar: b.bar,
            steps: b.steps,
            vel: grid
                .lanes
                .iter()
                .map(|_| {
                    (0..b.steps)
                        .map(|_| {
                            if rng.below(3) == 0 {
                                1 + rng.below(127) as u8
                            } else {
                                0
                            }
                        })
                        .collect()
                })
                .collect(),
        })
        .collect();
    // Every lane is played at least once: its first step of bar 1.
    for row in &mut grid.bars[0].vel {
        if row.iter().all(|v| *v == 0) {
            row[0] = 100;
        }
    }
    grid.check().expect("a generated grid is valid");
    grid
}

/// Witness for `the-drum-grid-round-trips-under-its-map`: for grids of many shapes under
/// the gm map, and under gm with a grid's own lanes laid over it, grid → song → grid is
/// the identity, directly and through a MIDI file, JSON and TOML; and every note plays
/// its lane's key in the map, on channel 10.
#[test]
fn the_drum_grid_round_trips_under_its_map() {
    let gm = LaneMap::gm();
    let own = [
        Lane {
            name: "shaker-2".into(),
            key: 82,
        },
        Lane {
            name: "kick".into(),
            key: 35,
        },
    ];
    let overlaid = gm.overlaid(&own).expect("a valid overlay");
    let mut rng = Lcg(0x5eed);
    let mut grids = 0;
    for map in [&gm, &overlaid] {
        for shape in shapes() {
            for _ in 0..8 {
                let grid = random_grid(&mut rng, map, shape);
                let bars = Some(grid.bars.len() as u32);
                let song = grid.to_song().expect("to song");

                let back =
                    DrumGrid::from_song(&song, grid.grid, map, None, bars).expect("from song");
                assert_eq!(back, grid, "grid → song → grid, {shape:?}");

                let midi = song.to_midi().expect("to MIDI");
                let read = Song::from_midi(&midi).expect("from MIDI");
                let back = DrumGrid::from_song(&read, grid.grid, map, None, bars)
                    .expect("from the MIDI song");
                assert_eq!(back, grid, "grid → song → MIDI → song → grid, {shape:?}");

                let json: DrumGrid =
                    serde_json::from_str(&serde_json::to_string(&grid).expect("json"))
                        .expect("json back");
                assert_eq!(json, grid, "JSON, {shape:?}");
                let toml: DrumGrid =
                    toml::from_str(&toml::to_string(&grid).expect("toml")).expect("toml back");
                assert_eq!(toml, grid, "TOML, {shape:?}");

                // On another channel, the drum track is found by name, and keeps it.
                let mut moved = grid.clone();
                moved.channel = 11;
                let back = DrumGrid::from_song(
                    &moved.to_song().expect("to song"),
                    grid.grid,
                    map,
                    Some(duet_drums::DRUM_TRACK),
                    bars,
                )
                .expect("from the named track");
                assert_eq!(back, moved, "channel 11, {shape:?}");

                let [track] = song.tracks.as_slice() else {
                    panic!("a grid is one track");
                };
                assert_eq!(track.channel, 10);
                let keys: Vec<u8> = grid
                    .lanes
                    .iter()
                    .map(|l| map.lookup(&l.name).expect("every lane is in the map").key)
                    .collect();
                assert_eq!(keys, grid.lanes.iter().map(|l| l.key).collect::<Vec<_>>());
                for note in &track.notes {
                    assert!(keys.contains(&note.pitch.midi()), "{note:?}");
                }
                let hits: usize = grid
                    .bars
                    .iter()
                    .flat_map(|b| b.vel.iter().flatten())
                    .filter(|v| **v > 0)
                    .count();
                assert_eq!(track.notes.len(), hits, "one note per hit");
                grids += 1;
            }
        }
    }
    assert_eq!(grids, 96);
}

/// Text → grid → text is the identity for text the writer wrote, and grid → text → grid
/// is the identity for any grid whose velocities are all levels, custom levels, custom
/// lanes and another channel included.
#[test]
fn the_text_form_round_trips_exactly() {
    let gm = LaneMap::gm();
    let overlaid = gm
        .overlaid(&[Lane {
            name: "shaker-2".into(),
            key: 82,
        }])
        .expect("overlay");
    let mut rng = Lcg(7);
    for (i, shape) in shapes().into_iter().enumerate() {
        let map = if i % 2 == 0 { &gm } else { &overlaid };
        let mut grid = random_grid(&mut rng, map, shape);
        if i % 3 == 0 {
            grid.levels = Levels {
                ghost: 30,
                hit: 80,
                accent: 127,
            };
            grid.channel = 11;
        }
        let grid = grid.snapped_to_levels();
        let text = grid.to_text().expect("all velocities are levels");
        let read = DrumGrid::from_text(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(read, grid, "grid → text → grid\n{text}");
        assert_eq!(read.to_text().expect("text"), text, "text → grid → text");
    }
}

#[test]
fn the_sample_groove_round_trips_through_midi() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../samples/groove.drums"
    ))
    .expect("the sample");
    let grid = DrumGrid::from_text(&text).expect("parse");
    assert_eq!(
        grid.to_text().expect("text"),
        text,
        "the sample is in written form"
    );
    let midi = grid.to_song().expect("song").to_midi().expect("midi");
    let song = Song::from_midi(&midi).expect("read midi");
    let back = DrumGrid::from_song(&song, grid.grid, &LaneMap::gm(), None, None).expect("grid");
    assert_eq!(back, grid);
}

fn drum_song(notes: &[(&str, u8, u8)]) -> Song {
    let mut track = Track::new("Kit", 10);
    for (at, key, vel) in notes {
        track.notes.push(Note {
            at: at.parse().expect("position"),
            pitch: duet_core::Pitch::new(*key).expect("key"),
            beats: 0.1,
            vel: *vel,
        });
    }
    Song {
        tracks: vec![track],
        ..Song::default()
    }
}

/// Reading a song's drum track never moves or alters a note to fit: each misfit is
/// refused, naming what does not fit, and velocities off the levels are kept exactly.
#[test]
fn reading_a_drum_track_refuses_what_does_not_fit_and_alters_nothing() {
    let gm = LaneMap::gm();
    let g16: Grid = "1/16".parse().expect("grid");
    let read =
        |song: &Song| DrumGrid::from_song(song, g16, &gm, None, None).map_err(|e| e.to_string());

    let kept = read(&drum_song(&[("1.1.0", 36, 101), ("1.2.120", 38, 7)])).expect("fits");
    assert_eq!(
        kept.bars[0].vel,
        vec![vec![0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], {
            let mut kick = vec![0; 16];
            kick[0] = 101;
            kick
        }],
        "snare above kick, velocities exact"
    );

    let cases: [(Song, &str); 4] = [
        (
            drum_song(&[("1.1.17", 36, 90)]),
            "key 36 at 1.1.17 is not on the 1/16 grid; quantize it first (duet song quantize --grid 1/16)",
        ),
        (
            drum_song(&[("1.1.0", 90, 90)]),
            "key 90 at 1.1.0 plays no lane in the gm lane map",
        ),
        (
            drum_song(&[("1.2.0", 38, 90), ("1.2.0", 38, 60)]),
            "two hits on snare at 1.2.0 (bar 1, step 5)",
        ),
        (
            Song {
                time_sig: TimeSig::new(7, 8).expect("7/8"),
                ..drum_song(&[("1.1.0", 36, 90)])
            },
            "bar 1 (7/8) is 3.5 steps of 1/4",
        ),
    ];
    for (i, (song, want)) in cases.iter().enumerate() {
        let grid: Grid = if i == 3 {
            "1/4".parse().expect("grid")
        } else {
            g16
        };
        let err = DrumGrid::from_song(song, grid, &gm, None, None)
            .expect_err(want)
            .to_string();
        assert!(err.contains(want), "{err}\nshould say: {want}");
    }

    let late = drum_song(&[("3.1.0", 36, 90)]);
    let err = DrumGrid::from_song(&late, g16, &gm, None, Some(2)).expect_err("past --bars");
    assert!(
        err.to_string().contains("a hit in bar 3, past the 2 bars"),
        "{err}"
    );

    let mut two = drum_song(&[("1.1.0", 36, 90)]);
    two.tracks.push(Track::new("Perc", 10));
    let err = read(&two).expect_err("two drum tracks");
    assert!(
        err.contains("2 tracks on channel 10 (\"Kit\", \"Perc\")"),
        "{err}"
    );
    let named = DrumGrid::from_song(&two, g16, &gm, Some("Kit"), None).expect("named");
    assert_eq!(named.lanes.len(), 1);
    let none = Song {
        tracks: vec![Track::new("Piano", 1)],
        ..Song::default()
    };
    assert!(
        read(&none)
            .expect_err("no drums")
            .contains("no track on channel 10")
    );
}

#[test]
fn off_level_velocities_are_refused_as_text_and_snapped_only_when_asked() {
    let grid = DrumGrid::from_song(
        &drum_song(&[("1.1.0", 36, 101), ("2.3.0", 38, 64)]),
        "1/8".parse().expect("grid"),
        &LaneMap::gm(),
        None,
        None,
    )
    .expect("fits");
    let err = grid.to_text().expect_err("101 is not a level").to_string();
    assert!(err.contains("kick, bar 1, step 1: velocity 101"), "{err}");

    let snapped = grid.snapped_to_levels();
    assert_eq!(
        grid.bars[0].vel[1][0], 101,
        "snapping never changes its input"
    );
    assert_eq!(snapped.bars[0].vel[1][0], 90);
    assert_eq!(
        snapped.bars[1].vel[0][4], 40,
        "64 is nearer g (40) than x (90)"
    );
    assert!(snapped.to_text().is_ok());
}

fn text_error(text: &str) -> String {
    DrumGrid::from_text(text).expect_err(text).to_string()
}

/// Every mistake in hand-written text, above all a miscounted row, is an error naming
/// the line, and where it applies the lane, the bar and both counts.
#[test]
fn text_errors_name_the_line_lane_bar_and_counts() {
    let head = "tpq 480\ntime 4/4\ngrid 1/16\n\n";
    let full = "| . . . . . . . . . . . . . . . . |";
    let cases = [
        (
            format!(
                "{head}[bars 1-2]\nsnare | . . . . X . . . . . . . X . . . | . . . . X . . . . . . . X . . |\n"
            ),
            "line 6: snare, bar 2 has 15 steps; 4/4 at 1/16 needs 16",
        ),
        (
            format!("{head}[bars 1]\nkick {full}\n[bars 2]\nsnare {full}\n"),
            "line 7: this block lists snare, but every block lists the same lanes as the first, in the same order: kick",
        ),
        (
            format!("{head}[bars 1]\nkick {full}\n[bars 3]\nkick {full}\n"),
            "line 7: this block starts at bar 3, but bar 2 comes next",
        ),
        (
            format!("{head}[bars 1]\nkick | o . . . . . . . . . . . . . . . |\n"),
            "line 6: kick, bar 1, step 1: 'o' is not a cell",
        ),
        (
            format!("{head}[bars 1]\nrim {full}\n"),
            "line 6: 'rim' is not a lane in the gm map; add a line like 'lane rim 70'",
        ),
        (
            format!("{head}[bars 1]\nkick {full}\ntempo 90\n"),
            "line 7: 'tempo' is not a row",
        ),
        (
            format!("{head}[bars 1-2]\nkick {full}\n"),
            "line 6: kick has 1 bars, but the block is bars 1-2 (2 bars)",
        ),
        (
            format!("{head}[bars 1]\nkick {full}\nkick {full}\n"),
            "line 7: kick is listed twice in this block",
        ),
        (
            "tpq 480\n\n[bars 1]\n".to_owned(),
            "line 3: a drum grid needs a grid line",
        ),
        (
            format!("{head}vel g 90 x 40\n"),
            "line 5: velocity levels g 90 x 40 X 120 must rise",
        ),
        (
            format!("{head}map tr808\n"),
            "line 5: 'tr808' is not a lane map",
        ),
    ];
    for (text, want) in cases {
        let err = text_error(&text);
        assert!(err.contains(want), "{err}\nshould say: {want}\n{text}");
    }
}

/// A hand-written file may use aliases, any lane order, comments and no count row; it
/// reads to the grid its written form would give, with lanes keeping the file's order.
#[test]
fn hand_written_text_is_read_leniently_but_exactly() {
    let text = "time 3/4\ngrid 1/8 # eighths\nvel x 100\nlane cajon 80\n\n[bars 1-2]\nBD | X . . . x . | x . . . . . |  # the kick\nhh | x x x x x x | x x x x x x |\ncajon | . . g . . . | . . . . . g |\n";
    let grid = DrumGrid::from_text(text).expect("parse");
    let names: Vec<&str> = grid.lanes.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["kick", "hh", "cajon"]);
    assert_eq!(grid.lanes[2].key, 80, "a lane line adds a lane");
    assert_eq!(grid.bars[0].vel[0], [120, 0, 0, 0, 100, 0]);
    assert_eq!(grid.levels.hit, 100);
    let written = grid.to_text().expect("text");
    assert!(written.contains("lane cajon 80\n"), "{written}");
    assert!(written.contains("vel g 40 x 100 X 120\n"), "{written}");
    assert_eq!(DrumGrid::from_text(&written).expect("again"), grid);
}
