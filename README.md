# making-music

Tooling for writing and editing music together with an LLM.

LLMs can't read MIDI, and the text formats that do exist either quantize (ABC) or speak
in ticks and note numbers (midicsv). The tool here is **duet**. Its first format,
**notefmt**, turns a MIDI file into one readable note per line, in bars, beats and note
names, and turns it back into MIDI without losing a tick. Its second, the **drum grid**,
writes a drum part as one row per drum and one column per step.

The workspace is split by domain:

| Crate | Holds |
|---|---|
| `duet-core` | the `Song` every format converts through, quantizing, and the notefmt text |
| `duet-midi` | reading and writing Standard MIDI Files |
| `duet-drums` | the drum grid, its lane maps, and grid ⇄ `Song` |
| `duet-cli` | the `duet` binary |

## notefmt

```
# notefmt v1: positions are bar.beat.tick, lengths are quarter notes
title "Sketch in D minor"
tpq 480
tempo 92
time 4/4
key D minor

[track "Piano" ch1 prog0]
1.1.0     cc64  127
1.1.0     D4    1      80
1.2.0     F4    0.5    72
1.2.240   A4    0.5    75
3.1.0     D4    0.333  80
3.1.160   E4    0.333  76

[track "Bass" ch2 prog33]
1.1.0     D2    2      90
1.3.0     A1    2      84
```

The full example is [samples/sketch.txt](samples/sketch.txt).

### Install and run

```sh
cargo install --path crates/duet-cli

duet convert take.mid take.txt      # MIDI to text
duet convert take.txt take.mid      # text back to MIDI
duet convert take.mid take.json     # or JSON, or .toml
duet song show take.mid             # print as text
duet song show take.mid --to json
duet song quantize take.mid take-q.mid --grid 1/16   # a copy snapped to sixteenths
```

Formats come from the file extension (`.mid`/`.midi`, `.json`, `.toml`, anything else is
text). Use `-` for standard input or output, with `--from` and `--to` to name the format.

### The text format

**Header**, before the first track. Every line is optional; the defaults are 480 ticks
per quarter, 120 bpm and 4/4. A header line after a `[track]` line is an error: tempo, meter and
key are the whole song's, so they are never written inside a track.

| Line | Meaning |
| --- | --- |
| `title "Name"` | Song name: the MIDI sequence name (see below) |
| `tpq 480` | Ticks per quarter note: the file's resolution |
| `tempo 92` | Tempo at the start, in quarter notes per minute |
| `time 6/8` | Time signature at the start |
| `key D minor` | Key at the start (`Dm`, `Bb major`, `Bb` also work) |
| `tempo 96 at 9.1.0` | Tempo change |
| `time 3/4 at 17.1.0` | Time signature change |
| `key A minor at 33.1.0` | Key change |

**Tracks** start with `[track "Name" ch1 prog0]`: a name, a MIDI channel from 1 to 16 and
optionally the General MIDI program (0 to 127) at the start. Then one event per line:

| Line | Meaning |
| --- | --- |
| `3.2.240  F#4  0.5  80` | Note: position, pitch, length, velocity (1 to 127) |
| `5.1.0  cc64  127` | Controller: number and value (64 is sustain, 1 the mod wheel) |
| `5.3.0  bend  -2048` | Pitch bend, -8192 to 8191, 0 is none |
| `9.1.0  prog  5` | Program change later in the track |

- **Positions** are `bar.beat.tick`, as a DAW shows them. Bars and beats count from 1. A
  beat is the time signature's denominator (a quarter in 4/4, an eighth in 6/8), and the
  tick is the offset into the beat in the file's own ticks: at 480 tpq, `1.2.240` is an
  eighth note after beat 2. `1.2` means `1.2.0`, and offsets past the end of a beat carry
  on (`1.1.960` in 4/4 is read as `1.3.0`).
- **Pitches** are scientific pitch names with middle C (MIDI 60) as `C4`. Sharps and flats
  both work (`F#3`, `Bb2`). When reading MIDI, black keys are spelled the way the scale of
  the key in force where the note starts spells them, so D minor gives `Bb` and `C#`, and
  after `key A major at 9.1.0` the same keys are `A#` and `C#`.
- **Lengths** are in quarter notes, whatever the time signature: `1`, `0.5`, `1.5`, or a
  fraction like `1/3` for a triplet.
- Lines starting with `#` are comments, and so is anything after a word that starts with `#`.

### Exact round-trips

Reading a MIDI file and writing it back keeps every note to the tick: start, length, pitch,
channel and velocity. Lengths and tempos are written as the shortest decimal that converts
back to exactly the same ticks and microseconds (a triplet eighth at 480 tpq is 160 ticks,
written `0.333`), so nothing drifts however many times a file goes through text.

Kept: notes, tempo changes, time signature changes, key signatures, track names, program
changes, controllers and pitch bend.

Dropped: aftertouch, SysEx, markers, lyrics and other text events, and note-off velocity.
A MIDI track that mixes channels (as format 0 files do) becomes one track per channel, and
files are always written as format 1, with the title, tempo, meter and key in the first track.

The title is the sequence name, which MIDI keeps as a track name: the name of a format 1
file's first track when that track holds no notes, or of the only track in a format 0 file.
A format 0 file's tracks have no names of their own, so they are named `Channel 1`,
`Channel 10` and so on.

Names are text. A track name or title whose bytes are not UTF-8 (an old file written in
Latin-1, say) is read with each invalid byte replaced by `�` (U+FFFD), so that name is not
written back byte for byte. Notes and everything else above are unaffected.

One thing MIDI cannot hold: a note that starts after another note of the same pitch and
channel and ends before it. MIDI has no note ids, so its note-off would end the outer note
instead. Converting such a song to MIDI is an error that names both notes. Notes that only
overlap, or that start together, are kept exactly.

Nothing is quantized unless you ask: played timing comes through as it was played.

### Quantizing

`duet song quantize` (or `Song::quantized` in the library) writes a copy with every note's
start and end moved to the nearest point of a grid. The input is never changed.

- The grid is written `1/16` for sixteenths or `1/8t` for triplet eighths. A grid whose step
  is not a whole number of the file's ticks is refused, naming the grid and the resolution.
- The grid restarts at every bar line, so bar lines are always on it. In a bar cut short by
  a time signature change, the last step before the bar line is short.
- A tick exactly halfway between two grid points goes to the later one.
- A note that would collapse to zero length gets one step instead, so no played note
  disappears. A note that was already zero length stays that way.
- Pitch, velocity, track and note order are kept. Controls, tempo, key and time signature
  changes are not moved.
- Two notes of the same pitch can end up one inside the other. Writing those to MIDI is
  refused by name, as above; quantize to text to see and fix them.

### As a library

```rust
use duet_core::Song;
use duet_midi::Midi; // adds Song::from_midi and Song::to_midi

let song = Song::from_midi(&std::fs::read("take.mid")?)?;
let text = song.to_text();
let json = serde_json::to_string_pretty(&song)?;
let back = Song::from_text(&text)?.to_midi()?;
```

`Song`, `Track`, `Note` and `Control` derive serde's `Serialize` and `Deserialize`, so any
serde format works. Positions, pitches, keys and time signatures serialize as the same
strings the text format uses.

## Drum grids

```
# notefmt drums v1: one row per lane, one column per step, | at bar lines
title "Groove in D"
tpq 480
tempo 96
time 4/4
grid 1/16
map gm

[bars 1-2]
#         1 . . . 2 . . . 3 . . . 4 . . .   1 . . . 2 . . . 3 . . . 4 . . .
crash   | X . . . . . . . . . . . . . . . | . . . . . . . . . . . . . . . . |
hh      | x . x . x . x . x . x . x . x . | x . x . x . x . x . x . x . . . |
hh-open | . . . . . . . . . . . . . . . . | . . . . . . . . . . . . . . x . |
snare   | . . . . X . . . . . . g X . . . | . . . . X . . . . . . . X . g . |
kick    | X . . . . . . X . . X . . . . . | X . . . . . . X . . X . . X . . |
```

That is [samples/groove.drums](samples/groove.drums). A drum grid is a matrix: for each
bar, one velocity per lane and step, 0 for no hit. `.drums` files are its text form, and
`.drums.json` and `.drums.toml` its exact form, the matrix itself.

```sh
duet drums render groove.drums groove.mid        # to MIDI (or .txt, .json, .toml)
duet drums read take.mid take.drums --grid 1/16  # a song's drum track to a grid
duet drums read take.mid take.drums.json --grid 1/16  # keeping every velocity
duet drums snap take.drums.json take.drums       # velocities snapped to g, x and X
duet drums lanes                                 # the lanes and their keys
```

**Header.** The notefmt header lines (title, tpq, tempo, time, key and their `at`
changes) mean what they mean in notefmt, plus:

| Line | Meaning |
| --- | --- |
| `grid 1/16` | One step (required): `1/16`, `1/8`, `1/8t` for triplet eighths, and so on |
| `map gm` | The lane map the rows are named from; `gm` is the default and the only one |
| `lane shaker-2 82` | Adds a lane, or moves a map lane to another key |
| `channel 10` | The MIDI channel, when it is not the map's (10) |
| `vel g 40 x 90 X 120` | The velocities the cells stand for, when they are not these |

**Blocks.** `[bars 1-4]` holds rows for those bars. Blocks cover every bar in order from
bar 1, so an empty bar is written as rests, and every block lists the same lanes in the
same order. A row is a lane name, then each bar's cells between `|`s. A cell is `.` (no
hit), `g` (ghost), `x` (hit) or `X` (accent), separated by single spaces. Each bar's cell
count is checked against its meter, and a wrong count names the line, lane and bar:
`line 14: snare, bar 2 has 15 steps; 4/4 at 1/16 needs 16`. The `#` row counting beats is
a comment, written to help find "the and of 3".

**Every bar must be a whole number of steps**, so 7/8 needs a grid of `1/8` or finer, and
a grid that does not fit is refused by name.

**Lanes.** The `gm` map has a lane for every General MIDI Level 1 percussion key, 35 to
81, on channel 10, with names that are never note names: `kick` 36, `sidestick` 37,
`snare` 38, `clap` 39, `rimshot` 40, `hh` 42, `hh-pedal` 44, `hh-open` 46, `crash` 49,
`ride` 51, the toms from `tom-high` 50 down to `tom-floor` 41, and so on. `duet drums
lanes` lists them all with their aliases (`bd` for `kick`, `oh` for `hh-open`); a file may
use an alias, and the grid then uses the lane's own name. Rows are written cymbals first,
then hats, toms, snare, kick and percussion. The map is built into `duet` for now; user
and project lane files come later.

**To and from a song.** Rendering gives one track named `Drums` on the grid's channel:
each hit is a note on its lane's key, one step long, with its velocity. Reading a song
takes its drum track (the one on channel 10, or the one `--track` names) and keeps only
note starts and velocities: lengths, controls and other tracks are left out, and `duet`
says how many. Nothing is moved to fit. A note off the grid (quantize it first, with
`duet song quantize`), on a key no lane plays, or on the same lane and step as another is
refused by name. The grid runs to the last bar with a hit, or to `--bars`.

Grid → song → grid gives back the same grid, through MIDI too, when its lanes are in the
map's row order and each is played at least once, as the writer leaves them.

**Velocities.** The text holds three levels. Writing a velocity that is not one of them as
text is refused, naming the lane, bar and step; keep it in `.drums.json`, or snap it to the
nearest level with `duet drums snap` (or `--snap-levels` when reading). A velocity exactly
halfway between two levels goes to the louder one. Nothing snaps unless you ask.

## Development

Builds on the latest stable Rust (1.100 or later, edition 2024).

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
```

CI also runs `cargo deny`, `cargo audit`, rustdoc with `-D warnings`, a build on the declared
`rust-version`, and the checks for the tasker-systems
[conventions](https://github.com/tasker-systems/conventions) baseline this repository is pinned to
(see [CLAUDE.md](CLAUDE.md) and `.conventions/`).

The round-trip tests in `crates/duet-midi/tests` generate MIDI files with humanized timing,
sustain pedal, pitch bend, tempo and meter changes (including one in the middle of a bar),
overlapping notes, and a format 0 file with several channels, at 96, 384, 480 and 960 ticks
per quarter, and check every kept event survives MIDI to text to MIDI unchanged. Those in
`crates/duet-drums/tests` generate drum grids in several meters (with a change of meter),
grids and resolutions, with any velocity, and check grid → song → grid, through MIDI, JSON
and TOML, and the text form all round-trip unchanged.
