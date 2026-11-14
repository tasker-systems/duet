# making-music

Tooling for writing and editing music together with an LLM.

LLMs can't read MIDI, and the text formats that do exist either quantize (ABC) or speak
in ticks and note numbers (midicsv). The first tool here, **notefmt**, turns a MIDI file
into one readable note per line, in bars, beats and note names, and turns it back into
MIDI without losing a tick.

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
cargo install --path crates/notefmt

notefmt convert take.mid take.txt      # MIDI to text
notefmt convert take.txt take.mid      # text back to MIDI
notefmt convert take.mid take.json     # or JSON, or .toml
notefmt show take.mid                  # print as text
notefmt show take.mid --to json
```

Formats come from the file extension (`.mid`/`.midi`, `.json`, `.toml`, anything else is
text). Use `-` for standard input or output, with `--from` and `--to` to name the format.

### The text format

**Header**, before the first track. Every line is optional; the defaults are 480 ticks
per quarter, 120 bpm and 4/4.

| Line | Meaning |
| --- | --- |
| `title "Name"` | Song name (the MIDI file's first track name) |
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
  both work (`F#3`, `Bb2`). When reading MIDI, black keys are spelled the way the opening
  key's scale spells them, so D minor gives `Bb` and `C#`.
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
files are always written as format 1, with tempo, meter and key in the first track.

There is no quantizing: played timing comes through as it was played. A quantize option is
a likely next step.

### As a library

```rust
use notefmt::Song;

let song = Song::from_midi(&std::fs::read("take.mid")?)?;
let text = song.to_text();
let json = serde_json::to_string_pretty(&song)?;
let back = Song::from_text(&text)?.to_midi()?;
```

`Song`, `Track`, `Note` and `Control` derive serde's `Serialize` and `Deserialize`, so any
serde format works. Positions, pitches, keys and time signatures serialize as the same
strings the text format uses.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

The round-trip tests in `crates/notefmt/tests` generate MIDI files with humanized timing,
sustain pedal, pitch bend, tempo and meter changes (including one in the middle of a bar),
overlapping notes, and a format 0 file with several channels, at 96, 384, 480 and 960 ticks
per quarter, and check every kept event survives MIDI to text to MIDI unchanged.
