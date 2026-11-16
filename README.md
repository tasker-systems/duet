# duet

Tooling for writing and editing music together with an LLM.

LLMs can't read MIDI, and the text formats that do exist either quantize (ABC) or speak
in ticks and note numbers (midicsv). **duet** is the tool for that. Its first format,
**notefmt**, turns a MIDI file into one readable note per line, in bars, beats and note
names, and turns it back into MIDI without losing a tick. Its second, the **drum grid**,
writes a drum part as one row per drum and one column per step. It also reads and writes
**ABC**, the shared score format, through the part of ABC a song can hold exactly, and
refuses the rest by name.

The workspace is split by domain:

| Crate | Holds |
|---|---|
| `duet-core` | the `Song` every format converts through, quantizing, and the notefmt text |
| `duet-midi` | reading and writing Standard MIDI Files |
| `duet-drums` | the drum grid, its lane maps, and grid ⇄ `Song` |
| `duet-abc` | a subset of ABC 2.1 ⇄ `Song`, and the corpus that checks it against abc2midi |
| `duet-play` | a `Song` as timed MIDI events, and a SoundFont synth that renders them |
| `duet-dsp` | audio analysis, samples to events: notes, hits, tempo, meter, key; no I/O |
| `duet-capture` | decoding a take, writing one as FLAC, and assembling `duet-dsp`'s events into a `Song` and score |
| `duet-schemas` | documents: the envelope, versioned kinds, `.duet` and `.duet.jsonl`, the version policy |
| `duet-cli` | the `duet` binary |

## How the formats fit together

**ABC is a score, and notefmt is a performance.** They are different kinds of text, and the
`Song` sits between them. Every format converts through one in-memory `Song`: notes at
bar.beat.tick positions with exact tick lengths, velocities, channels, programs, controls,
and tempo, meter and key changes. notefmt is the text form that shows a `Song` exactly. The
drum grid and ABC are readable, grid-shaped views of part of one, for discussing it.

```mermaid
flowchart LR
    MIDI["MIDI (.mid)"] <-->|lossless| Song(("Song"))
    Song <-->|lossless| TXT["notefmt (.notes)<br/>a performance, exact"]
    Song <-->|lossless| DOC["a song document (.duet)<br/>the Song, versioned"]
    Song <-->|"a drum track on its grid"| DRUMS[".drums<br/>a drum part"]
    Song <-->|"a score-shaped song"| ABC[".abc<br/>a melodic score"]
    Q["duet song quantize<br/>(an explicit step)"] -.->|"makes a played take score-shaped"| Song
```

- **notefmt holds whatever MIDI holds that duet keeps.** A note can start 10 ticks late, last
  0.98 of a beat, have velocity 103, or overlap another note; a track can sit on channel 10,
  with sustain pedal and pitch bend. Nothing has to fit a grid. That is what a sung or tapped
  idea looks like after capture, and what a take from a DAW looks like. MIDI ⇄ notefmt never
  loses anything duet keeps.
- **ABC holds a score.** Lengths are fractions of a unit note, bars are full, pitches are
  spelled against a key, and velocity is one of eight dynamics. ABC also carries notation a
  performance has no use for: repeats, slurs, beaming, annotations. It is the format people
  and LLMs already read and talk about as music. ABC → `Song` → ABC keeps every note of the
  subset duet reads; notation-only marks are listed and dropped, and repeats come back written
  out.
- **Writing ABC works only for a song that is already score-shaped.** A played take is
  refused by name, naming the note, and never snapped quietly. Quantizing is a step you
  choose: `duet song quantize`, then `duet abc write`.
- **The drum grid is the same idea for drums**, which ABC refuses. ABC covers pitched lines.

| | notefmt `.notes` | `.drums` | `.abc` |
|---|---|---|---|
| What it is | a performance, exact | a drum part on a step grid | a melodic score |
| Unit | ticks | steps | fractions of a note |
| Velocity | any, 1 to 127 | three levels as text (any in a drum-grid `.duet`) | eight dynamics |
| Lossless with `Song` | yes | for what fits the grid | for the subset |
| For | capture, DAWs, exact edits, diffs | discussing a groove | discussing and sharing a tune |

notefmt is also where anything is checked exactly. The ABC tests compare in it: abc2midi's
reading of each corpus tune is committed as notefmt text. That is how abc2midi's
undocumented habit shows: every note one tick late and one tick short (`1.1.1  C4  0.998`),
which ABC itself could not express.

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

The full example is [samples/sketch.notes](samples/sketch.notes).

### Install and run

```sh
cargo install --path crates/duet-cli

duet convert take.mid take.duet                  # MIDI to a song document
duet convert take.duet take.mid                  # and back
duet view take.mid > take.notes                  # MIDI to notefmt text, a view
duet apply take.notes -o take.duet               # text to a document
duet song show take.duet                         # print as text
duet song quantize take.mid take-q.mid --grid 1/16   # a copy snapped to sixteenths
```

**A file's envelope means the same to every command.** A `.duet` file is a duet document,
read by its envelope: the kind it names decides what it is, never the file's name, so a
command that reads a song refuses a document of another kind, naming the kind. `.mid`
(`.midi`, `.smf`) and `.abc` are the outside formats, read and written by their names. No
other name means a format, and `--from` and `--to` name one (`duet`, `midi` or `abc`). notefmt
text and a drum grid's text are [views](#views-duet-view-and-duet-apply) of a document: only
`duet view` writes them and only `duet apply` reads them. Any other command refuses a `.notes`,
`.drums` or `.txt` name, or any name that is not a format's, pointing at `duet apply <file> -o
<new.duet>`. `.json`, `.toml`, `.drums.json` and `.drums.toml` are no longer read or written: a
song document's body is what a song's `.json` held. Use `-` for standard input or output.
Standard input is read by what it holds: a document when its first non-blank byte is `{`, MIDI
when it starts with `MThd`, and ABC with `--from abc`; text there is refused, pointing at
`duet apply - -o -`. Standard output is a document unless `--to` names another format (`abc
render` writes MIDI, and `song show` prints notefmt text), so one command pipes into the next:

```sh
duet apply sketch.notes -o - | duet play -       # play a view's text, no file in between
```

`convert` reads and writes [drum-grid documents](#drum-grids) too; writing one from a song needs
`--grid`.

**A document says where its song came from.** A song imported from ABC or MIDI records it in
the envelope's `from`: `{"format": "midi"}`, or for ABC the tune's `X:` number and every
notation-only construct the reader read but did not carry (a slur, an annotation), each at its
line and column, as `duet abc check` lists them. The provenance follows the song: a `.duet`
read and written again keeps its `from` unchanged, and so does `song quantize`. Going out to
ABC or MIDI and in again replaces it, since that import is the song's new provenance. A
document `duet apply -o` makes from a view has none.

### Views: `duet view` and `duet apply`

The text forms are **views** of a document, not files of their own. `duet view` renders one,
and `duet apply` reads an edited one back: a working surface a person or an agent edits until
it plays as intended.

```sh
duet view take.duet > take.notes                          # a song's notes view
duet view beat.duet -o beat.drums                         # a drum grid's grid view
duet view take.duet --as grid --grid 1/16 > kit.drums     # one drum track of a song, as a grid
duet apply take.notes --onto take.duet                    # read the edited view back
duet apply kit.drums --onto take.duet                     # only the drum track changes
duet apply old.notes -o old.duet                          # a lossless view into a new document
```

| View | Of | Drops |
|---|---|---|
| `notes` | a song | nothing: every field of song 1.0, normalized, with its end stated |
| `grid` | a drum grid | nothing, while every velocity is 0 or one of its three levels |
| `grid` | one drum track of a song, at a step | everything but that track's starts and velocities |

Every view drops the envelope's `tool` and `from`, and shows its `statements` as `# ` comments
under the header. Comments added to a view are not read back. A velocity off the levels is
refused, naming its lane, bar and step, unless `--snap-levels` shows it snapped.

**The header line** names the view and the document it came from, so a view reads back without
a file name: `# duet notes v1 of song 1.0 from 3f9a1c0d5e2b7a41`, or `# duet grid v1 of song 1.0
track "Drums" step 1/16 from …` for a partial view. The digest is a 64-bit FNV-1a of the source
document's body: `apply --onto` refuses a document changed since the view was rendered, naming
both digests, unless `--force`. A newer view version or a schema version this duet does not read
is refused by name. Today's `# notefmt v1` and `# notefmt drums v1` lines read as `notes v1 of
song 1.0` and `grid v1 of drum-grid 1.0`, so every `.notes` and `.drums` file applies. A view
with no header needs `--as notes` or `--as grid`.

**Applying onto the source** goes cell by cell, with no ids. A lossless view replaces the body
and keeps the source's envelope (`from` and `statements`). A grid view of a song changes only the
drum track it names: an unchanged cell keeps its note exactly, its length and velocity (a
velocity shown snapped included); a changed level sets that note's velocity; a new hit is a note
one step long; an emptied cell removes the note. A conductor line (tempo, meter, key, end), the
step or the channel changed in a partial view, and a lane or bar added or removed there, are
refused by name: change the conductor in the notes view, which shows the whole song. A lossy view
makes no document of its own: `-o` refuses it, naming `--onto`.

**`view` and `apply` are the one way to text.** `--from text`, `--to text`, `--from drums` and
`--to drums` were removed: clap refuses each, pointing here. A one-shot pipes instead:
`duet apply x.notes -o - | duet play -`. `duet song show` still prints a song as notefmt text,
with no header, for reading.

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
| `key D minor` | Key at the start (`Dm`, `Bb major`, `Bb` also work), or a mode: `key D dorian` |
| `tempo 96 at 9.1.0` | Tempo change |
| `time 3/4 at 17.1.0` | Time signature change |
| `key A minor at 33.1.0` | Key change |
| `end 41.1.0` | Where the song ends, when it says (see below) |

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

### Where a song ends

A song names its end with `end`, a position like any other: `end 9.1.0` is the bar line
before bar 9, and `end 8.3.0` stops halfway through bar 8, for a pickup ending. Every format
keeps it: the header line in text and drum grids, `end` in a song document's body, and MIDI's
end of track, where every track ends.

**Every file `duet` writes states its end**, so a file shared between people means the same
on every machine, whatever defaults each has set.

- A note may end exactly on the end. Anything past it is refused, naming it and the end:
  `track 'Piano': the D4 at 8.4.0, which ends at 9.1.240, lies past the song's end at 9.1.0`.
  That goes for notes, controls and tempo, meter or key changes, when reading, writing MIDI
  and quantizing. Nothing is cut to fit.
- A song that names no end gets the default: the bar line after the last bar holding a
  note or control, and never before a tempo, meter or key change. `duet.toml` can change
  the default (see [Configuration](#configuration)). A file written from it states that
  end, so it reads the same wherever it goes.
- Reading MIDI, the file's end (its latest end of track) becomes the song's when some
  track's end of track comes after the event before it, or when it lies past every event
  kept. A file whose tracks each end exactly at their last event cannot say whether it means
  an end there or none, so it reads as naming none and the default applies; the music is
  the same either way.

### Exact round-trips

Reading a MIDI file and writing it back keeps every note to the tick: start, length, pitch,
channel and velocity. Lengths and tempos are written as the shortest decimal that converts
back to exactly the same ticks and microseconds (a triplet eighth at 480 tpq is 160 ticks,
written `0.333`), so nothing drifts however many times a file goes through text.

Kept: notes, tempo changes, time signature changes, key signatures, track names, program
changes, controllers, pitch bend, and the file's end (above). A key in a mode other than
major or minor (dorian, phrygian, lydian, mixolydian, locrian) is written as its key
signature plus a `duet:key D dorian` text event, which is read back; other programs see the
signature.

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
let back = Song::from_text(&text)?.to_midi()?;

// A song document (.duet), through duet-schemas:
use duet_schemas::{Document, Tool};
let doc = Document::new(Tool::new("my-tool", "1.0"), song).write()?;
let song = Document::<Song>::read(&doc)?.body;
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

That is [samples/groove.drums](samples/groove.drums). A drum grid is a matrix: one
velocity per lane and step over the whole grid, 0 for no hit, with each bar's step count.
That is its text, the drum grid's `grid` view. Its document is a drum-grid `.duet`, which holds
the matrix itself: `bar_steps` and one lanes × steps tensor, `vel`, so it keeps any velocity
from 1 to 127.

```sh
duet apply groove.drums -o groove.duet           # the grid's text to a drum-grid document
duet drums render groove.duet groove.mid         # to MIDI (or a song .duet)
duet drums read take.mid take.duet --grid 1/16   # a song's drum track to a drum-grid document
duet view take.duet --as grid > take.drums       # and its text, to read or edit
duet drums snap take.duet snapped.duet           # velocities snapped to g, x and X
duet drums lanes                                 # the lanes and their keys
```

The grid side of each `duet drums` command (`render`'s input, `read`'s output, `snap`'s both)
is a drum-grid document: a `.duet`, read by its envelope, or `-` for one on standard input or
output. Any other name is refused rather than misread: a grid's text (`.drums`, or any name that
is not a format's), pointing at `duet apply`; a song's (`.mid`, `.abc`); a stream
(`.duet.jsonl`); or `.json` and `.toml`. A song document there is refused, naming its kind. The
song side is a song's, as for every other command.

**`drums snap` stays.** `duet view --snap-levels` shows a grid snapped, but `duet apply --onto`
then keeps each velocity exact wherever its level is unchanged, by design, so snapping through
views needs `view --snap-levels -o - | apply - -o new.duet`, two commands and a pipe that drop
the count of what moved. `drums snap` is the one step that commits snapped velocities to a
document and says how many moved.

`duet convert` goes between grids and every other format as well, by the same code:
reading a grid it writes what `duet drums render` writes, and writing one from a song it
takes `duet drums read`'s `--grid`, `--track`, `--bars` and `--snap-levels`, and writes and
refuses what `duet drums read` does.

```sh
duet convert groove.duet groove.mid                            # the same as drums render
duet convert take.mid take.duet --grid 1/16                    # the same as drums read
duet convert groove.duet copy.duet                             # a straight copy
duet convert groove.duet sixteenths.duet --grid 1/16           # re-stepped
```

A `.duet` output keeps what was read: a drum-grid document read is written as one, and so is a
grid made from a song with `--grid`. A song is written as a song document; `duet drums render` makes a song of a grid.

Writing a grid from a song without `--grid` is refused, naming it: the step is never
guessed. From a grid to a grid, with no `--grid`, the grid is copied as it is, read and
written as `drums snap` does but without snapping: a lane with no hits, the grid's own
`vel` levels and any velocity survive; `--snap-levels` snaps them. With `--grid`, the grid goes through a
song at that step, as `drums render` then `drums read` would, so lanes with no hits are
dropped. A grid flag that would do nothing (`--track` or `--bars` without `--grid` from a
grid, or any grid flag for an output that is not a grid) is refused too, so no flag is
silently ignored.

**Header.** The notefmt header lines (title, tpq, tempo, time, key and their `at`
changes) mean what they mean in notefmt, plus:

| Line | Meaning |
| --- | --- |
| `grid 1/16` | One step (required): `1/16`, `1/8`, `1/8t` for triplet eighths, and so on |
| `map gm` | The lane map the rows are named from; `gm` is the default and the only one |
| `lane shaker-2 82` | Adds a lane, or moves a map lane to another key |
| `channel 10` | The MIDI channel, when it is not the map's (10) |
| `vel g 40 x 90 X 120` | The velocities the cells stand for, when they are not these |
| `end 3.1.0` | Where the grid stops: the bar line after its last bar, or a step inside that bar |

The writer always states the end. An `end` inside the last bar must be on a step, and the
cells from that step on must be rests. An `end` that contradicts the bars, before the last
bar or past it, is refused by name, and so is a tempo, meter or key change past the end.

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
then hats, toms, snare, kick and percussion. A user's or project's `duet.toml` can add lanes, move them and give them aliases (see
[Configuration](#configuration)).

**To and from a song.** Rendering gives one track named `Drums` on the grid's channel:
each hit is a note on its lane's key, one step long, with its velocity. Reading a song
takes its drum track (the one on channel 10, or the one `--track` names) and keeps only
note starts and velocities: lengths, controls and other tracks are left out, and `duet`
says how many. Nothing is moved to fit. A note off the grid (quantize it first, with
`duet song quantize`), on a key no lane plays, or on the same lane and step as another is
refused by name. The grid runs to `--bars` when it is given, or else to the song's end, the
one it names or its default (a mid-bar end, which must be on a step, becomes the grid's
`end`). `--bars` never cuts anything: a hit or a tempo, meter or key change past it is
refused by name.

Rendering gives the song the grid's end, so trailing empty bars and a mid-bar end survive.

Grid → song → grid gives back the same grid, through MIDI too, with no bar count asked for,
trailing empty bars and a mid-bar end included, when its lanes are in the map's row order
and each is played at least once, as the writer leaves them.

**Velocities.** The text holds three levels. A `grid` view of a velocity that is not one of
them is refused, naming the lane, bar and step, unless `--snap-levels` shows it snapped; a
drum-grid `.duet` keeps it, and `--snap-levels` on `duet drums read` (or `duet drums snap`)
snaps it to the nearest level for good. A velocity
exactly halfway between two levels goes to the louder one. Nothing snaps unless you ask.

## ABC

```sh
duet abc check tunes.abc                      # what each tune holds that a song does not carry
duet abc render tunes.abc tune.mid --tune 2   # one tune as MIDI (or .duet)
duet abc write take.duet take.abc --unit 1/4  # a song as ABC, X:1
duet convert tune.abc tune.mid                # .abc works both ways, for a file of one tune
```

`duet-abc` reads a subset of [ABC 2.1](https://abcnotation.com/wiki/abc:standard:v2.1):
header fields, notes, octaves, accidentals, lengths, rests and multi-bar rests, bar lines,
ties, broken rhythm, tuplets up to `(9`, chords, repeats and endings (unfolded), inline
`[M:]`, `[L:]`, `[Q:]` and `[K:]`, keys with a mode, voices, `%%MIDI program` and
`%%MIDI channel`, the dynamics `!ppp!` to `!fff!`, and several tunes per file. A pickup is
a short first bar of its own time signature, so its notes keep abc2midi's ticks exactly.

**Notation only.** Slurs, annotations, information fields such as `C:` and `N:`, voice
properties other than the name, other `%%` directives and the spacer `y` are read and not
carried into the song. `duet abc check` lists each with its line, and `render` says how many
there were.

**Refused by name.** Everything else is refused with its line and column and the name of
the construct: grace notes, decorations other than dynamics, chord symbols, lyrics, parts,
`K:none`, clefs and transposition, `%%MIDI` commands other than program and channel, and
the rest of the table in `duet_abc::REFUSED`. Two constructs are refused because the
standard and abc2midi read them differently: a bare `Q:120`, which the standard reads as
unit lengths per minute and abc2midi as quarters (write `Q:1/4=120`), and a tune with no
`M:`, which is free meter in the standard and 4/4 in abc2midi. A bar of the wrong length is
an error naming the bar; abc2midi plays it as written, without a warning.

**One place duet parts from both.** A `:|` with no `|:` goes back to the start of the tune,
in the standard and in abc2midi, so a pickup is replayed there (abc2midi warns of the short
bar). A song cannot hold a short bar in its middle, so duet goes back to the first full bar
after the pickup instead. Write `|:` after the pickup and all three agree.

**Writing.** Nothing is quantized. A note off the 1/64-note grid that is not in a triplet
group (three equal slots filling whole beats, each note one or two of them) is refused,
naming its track and position (quantize it first with `duet song quantize`). So are a velocity that is not a dynamic's level
(`--dynamics nearest` snaps it, and the ABC says so in a comment), a tempo that is not a
whole bpm, a track on channel 10 (drums are the drum grid's), and notes that overlap
without starting and ending together. ABC needs a key, so a song with none is written in C
major. The ABC written is canonical: reading it and writing it again gives the same text.

**Where it comes from.** `duet-abc` was written from the ABC 2.1 standard and from
abcMIDI's documentation of how abc2midi plays what the standard leaves open. No abcMIDI
source file was opened, and none is copied. abc2midi is only ever run as a separate
program, on a developer's machine, to check this crate's reading (see Development).

## Playing a song

```sh
duet play take.duet                        # through the built-in synth, to the speakers
duet play take.mid --soundfont GM.sf2      # with a SoundFont of your choosing
duet play take.duet --port "IAC Driver"    # to a MIDI port: Logic, a synth, a DAW
duet apply take.notes -o - | duet play -   # a view's text, through apply
```

- **The built-in synth** sounds the song with a General MIDI SoundFont (`.sf2`): every note
  at its position, length, pitch, channel, program and velocity, with its controls and tempo
  changes. Channel 10 is drums. It needs nothing else running.
- **The SoundFont** is `--soundfont`, else `[play] soundfont` in duet.toml, else the one duet
  installs: `default.sf2` in the data directory (`~/.local/share/duet` on macOS and Linux, or
  `$XDG_DATA_HOME/duet`; `%APPDATA%\duet\data` on Windows; or whatever `DUET_DATA_DIR` names),
  or in `share/duet` beside the binary's directory. With none, `duet play` fails saying where
  it looked; it never plays silence.
- **`--port`** sends the same events, at the same times, to a MIDI port instead, with no
  SoundFont. Give the port's name, or a part only one port's name holds, in any case. On
  macOS a port exists only while the app that owns it runs, such as Logic, or the IAC Driver
  once it is enabled in Audio MIDI Setup. An unknown name lists the ports there are. If
  playing stops early, on Ctrl-C or a failed send, duet turns off every note it left
  sounding and the sustain pedal on each channel it used.
- **What plays is the song's MIDI file**, exactly as `duet convert take.duet take.mid` writes
  it, with every track merged. At one moment, notes ending come before programs and controls,
  and those before notes starting, so one track's note never cuts off another's. MIDI cannot
  say which of two overlapping notes on one key a note-off ends, so the key is held until the
  last of them ends: each note sounds at least to its own end. The synth renders in blocks of
  64 samples, so an event sounds within 1.5 ms of its time.

## Recording a take

```sh
duet record idea.flac                           # from the default input, until Ctrl-C
duet record idea.flac --seconds 30              # stop after 30 seconds
duet record idea.flac --device scarlett         # an input by its name, or part of it
duet capture melody idea.flac                   # then capture it like any take
```

`duet record` keeps a take from the machine's microphone as a FLAC file, which `duet capture`
reads like any other recording. FLAC is lossless: the take is the samples the device
delivered, at its own rate and channels, stored at 24 bits from a float or 32-bit device and
16 from a 16-bit one. Nothing is resampled, mixed or turned up. A 24-bit mono take is about 5 MB
a minute.

- **The file must be new and named `.flac`.** A take never replaces a file; a path that exists,
  another extension, or a directory that does not exist is refused before any device is
  opened.
- **`--device <name>`** picks an input by its name, or a part only one input's name holds,
  ignoring case. A name that matches none lists the inputs there are.
- **`--seconds <n>`** stops the take after that long. Without it, Ctrl-C stops it; either way
  the file is finished and kept.
- **What it says on standard error.** The device, rate, channels and depth as it starts; the
  file and its length when it ends; and anything the owner should know about the take: input
  lost (when, and how many milliseconds, written as silence so what follows keeps its time),
  overruns the device reported, samples at full scale (the take may be clipped), and a take
  silent throughout.
- **A take that ends early is kept.** The file is written a frame at a time, so a device that
  disconnects, or a process that is killed, leaves a file that reads up to its last whole
  frame.
- **On macOS**, the terminal app needs microphone access (System Settings, Privacy & Security,
  Microphone). Without it the system records silence rather than refusing, and `duet record`
  says the take is silent throughout.

## Capturing an idea

```sh
duet capture melody idea.m4a                    # idea.duet, idea.abc, idea.capture.duet
duet capture rhythm tap.wav --lane hh           # a tapped take: every hit in one lane
duet capture rhythm beatbox.flac --tempo 96     # a beatboxed take: kick, snare and hat
duet capture melody idea.m4a --key Eb --downbeat 0.4 -o scratch/idea
```

A recording of a sung, hummed or single-note played melody, or of a tapped or beatboxed
rhythm, becomes the text the rest of duet reads. Both commands take one recording and these
options; everything about the timing that is left out is estimated and stated.

- **`--tempo <bpm>`** is in quarter notes per minute, used exactly. A melody's must be a whole
  number, because its ABC states the tempo as `Q:1/4=<n>`; a fractional one is refused before
  analysis starts. A rhythm accepts any tempo.
- **`--meter <m>`** is the meter, such as `4/4` or `6/8`, used exactly.
- **`--downbeat <seconds>`** is where bar 1 starts, in seconds from the start of the recording.
- **`--tuning <a4>`** is a melody's tuning: A4 in Hz (`440`, `442`) or in cents from 440 Hz
  (`+44c`, `-12c`), used exactly, up to a key either way. A rhythm take's hits have no pitch,
  and `--tuning` is refused for one.
- **`--reader hmm|rules|basic-pitch`** is what chooses a melody's notes from its performance:
  `hmm`, the note HMM (the default); `rules`, the segmentation rules; or `basic-pitch`,
  Spotify's Basic Pitch. A reader other than the HMM is stated on standard error when chosen
  (`reader: the segmentation rules`). See *Readers* below. A rhythm take's hits are not
  read as notes, and `--reader` is refused for one.
- **`--key <key>`** is a melody's key as ABC's `K:` names it (`Eb`, `Bm`, `F#m`), used exactly.
  A rhythm take has no key, and `--key` is refused for one.
- **`--grid <step>`** is the step the score is quantized to, such as `1/16` (the default) or
  `1/8t`. The `.duet` is never quantized.
- **`--lane <name>`** (rhythm) puts every hit in that lane, such as `hh` or `kick`. It is how a
  tapped take is read; without it a rhythm take is read as beatboxed. Lane names resolve
  through the configured lane map, so aliases from `duet.toml` apply. A melody's notes are
  pitched, not in lanes, and `--lane` is refused for one.
- **`-o`, `--output <stem>`** is the output files' stem. Without it, the input's name without
  its extension, beside the input.

**Files written.** Three, and a melody's performance, all beside each other, and none ever
overwrites a file that exists (the whole capture is refused before anything is written). Once
the take is analysed, the `.duet` and the `.capture.duet` are always written; if the score then
cannot be, that is reported after them, and they stay:

- `STEM.duet`: the exact performance, unquantized, at the detected times, as a song document.
  Quantizing never replaces it. Every duet command reads it by its envelope: `duet song show
  STEM.duet` prints it as notefmt text.
- `STEM.abc` for a melody, or `STEM.grid.duet` for a rhythm: the score, the `.duet` quantized
  to `--grid`. A rhythm's score is a drum-grid document, which keeps every velocity exactly;
  `duet drums render STEM.grid.duet` reads it by its envelope.
- `STEM.capture.duet`: the analysis, a `capture-analysis` document that reads back. Every onset, note and hit, in sample frames at the
  stated rate (22 050 Hz; divide by it for seconds); the timing (tempo, its half and double,
  the measured beat period, meter, downbeat, each marked given or estimated, with a
  confidence); a melody's key, with its pitch-class histogram and every key's score; the tempo
  from each beat to the next, so drift is a series and not a feeling; and how far quantizing
  moved each note, in milliseconds and in grid steps; and a melody's `reading` (since
  `capture-analysis` 1.1): the tuning its notes were read on, and each note's key, `centre` and
  whether it is between keys; and since 1.2 its residual, what the notes leave of the
  performance: per note its centre, drift (cents a second), vibrato (rate and depth), the slide
  in and the fall off (cents and milliseconds) and the rest; per take the centres' histogram
  and their slope over the take, the voiced frames read and unread, and the **loss** as its
  terms (pitch, unread, notes), never as one number; since 1.3 its `reader`, which
  reader chose the notes (`rules` or `hmm`) and the cost per note the loss counts; and since
  1.4 the loss priced, in nats, as the note HMM decodes: a fourth term, `voicing` (what reading
  frames costs that their voicing says are not sung), `unread` costing each frame left out by
  its voicing (about 2 at 0.9, nothing for a breath), and `pitch` measuring a slide as slide,
  not as though held on the key it arrives at; and since 1.5 an estimated tuning's confidence
  measured above a flat spread: 0 when the take's held pitch spreads evenly across the hundred
  cents, 1 when all of it sits within 15 cents of the estimate. Below 0.5 the tuning is stated
  weak, and capture suggests giving it with `--tuning`. Since 1.6 a note's residual states its
  `excursions`, the ornaments inside it as measured: each a run of at least 3 frames (35 ms)
  held at least 50 cents from where the note is held, as its `start`, `end` and `cents` from
  the note's key. A held approach before the note's held pitch is one too. They are never
  named in the file: a grace, a turn or a trill is worked out from them when something is
  written. A note with none writes none. Since 1.7 the reader may be `basic-pitch`.
- `STEM.performance.duet.jsonl`, for a melody: every pitch frame, before any note is read from
  it, as a `performance` stream. The first line is the envelope with its head (the rate, the hop,
  the first frame and the unit); each later line is about a second of frames as the columns
  `pitch` (absolute cents, `1200·log2(hz/440) + 6900`, so 6900.0 is A440, to 0.1 cent, `null`
  when unvoiced), `voicing` and `level`. It is written as the frames settle, and kept with the
  `.duet` and `.capture.duet`. A three-minute take's is about 370 KB.

**Formats.** WAV, FLAC, MP3, Ogg Vorbis and AAC in M4A are read, a packet at a time, so a long
take is never held in memory whole. A damaged stretch that cannot be decoded is heard as silence
of its true length, so every note after it keeps its time. A file that is not audio, or uses a
codec duet cannot decode, is refused naming the file and the codec. HE-AAC is refused by name
when the file's header declares it. Known limit: HE-AAC signalled only inside the audio data, with a plain
AAC-LC header, decodes as AAC-LC at half bandwidth without a refusal. Catching it would mean
comparing the declared and decoded sample rates, which capture does not do.

**What is estimated, and how it is said.** The tempo, the meter, the downbeat and a melody's
tuning and key are each given or estimated. A given value is used exactly and never reported as an
estimate. An estimate is applied and stated, on standard error, in the `.duet`'s envelope as
its `statements`, and in the score: in a `.grid.duet`'s envelope, or as `%` comment lines in
ABC. Each says what you would reach for if it is wrong: the half and double tempo, the relative major or minor, and in a
compound meter the felt beat. A weak estimate is marked `weak`; a meter that cannot be told
apart is applied as 4/4 and says so. For the synthetic melody in the reference corpus:

```
$ duet capture melody synthetic-melody.flac
tempo: estimated 121 bpm (half 60.5, double 242), confidence 0.53
meter: estimated 4/4, confidence 0.14, weak
downbeat: estimated at 0.255 s, confidence 0.14, weak
tuning: estimated A4 = 440.2 Hz (+1 cent), spread 2 cents, confidence 1.00
notes: 8 read, 0 between keys
key: estimated C major (relative A minor), confidence 0.88
lead-in: none
velocities: 8 snapped to the nearest dynamic's level in the .abc; the .duet keeps them exact
```

The take was made at 120 bpm, so this estimate is a beat a minute fast. A tempo's
confidence is the lesser of how many onsets sit on its beat and how clearly it beat its half
and double, so a take that could as well be read at half the tempo is marked weak.

The tempo is in quarter notes per minute always. The estimate is rounded to a whole bpm for
the score, but **notes are placed by the beat period actually measured**, not by the rounded
tempo, so none drifts off its beat over a long take; playback runs a fraction of a percent off
the recording's real time. A given tempo is used exactly for both. The key changes how notes
are spelled and the signature, never a pitch.

**Tuning.** A melody's notes are read on its own tuning, not on A440, so a take sung 44 cents
sharp reads the keys it would in tune. The tuning is estimated from the performance: every
voiced frame whose pitch holds steady for 50 ms gives its distance from the nearest key, and the
30-cent window where most of the singing sits (weighted by how voiced each frame is) is where
A4 is. Its confidence is how far the share of the singing within 15 cents of it stands above
what a flat spread across the hundred cents would put there (0.31): 0 for a take whose held
pitch wanders evenly, 1 when all of it is that near. Below 0.5 it is stated weak, and capture
suggests giving the tuning with `--tuning`. Its spread is how widely the take strays from it. Each note is the key nearest its held pitch on that tuning; one
held more than 35 cents from its key is **between keys**, counted on the `notes:` line and
marked in the analysis's `reading`, with every note's `centre`, its distance from its key in
cents. The key is estimated after, from those notes, and never changes one.

**Readers.** Three readers choose a melody's notes from its performance, on its tuning, and all
are judged by the same loss. The **rules** (`--reader rules`) segment the pitch where it leaves a
held note and holds somewhere new, or at an onset. The **note HMM** (the default) decodes,
frame by frame, whether the voice is sliding into a key (attack), holding it (stable) or silent,
by fixed-lag Viterbi a second behind. A note is a key's held frames with the slide into them; a
note may follow another with no breath (legato), and a dip that leaves a pitch and comes back
is two notes. A slide that never holds is no note, and its frames are left unread, which costs
what leaving singing out costs by its voicing; so a short note, or one held far from its key,
is read, while a breath is left out. Every note costs the same, `c` = 4 nats (the negative log probability of leaving a
held note): raising it merges notes, lowering it splits them. Vibrato of half a semitone either
way with a slow drift stays one note at `c` and at `c/2`; a semitone step held 0.5 s is two.
Known limits: a glide slower than about 8 semitones a second (15 at `c/2`) is read as a short
note on each key it passes, where the rules read a few fewer; the HMM leaves the last, weakly
voiced frames of a decaying note unread, where the rules read them, and the loss charges
nothing for them, since their voicing says they are not sung.

**Basic Pitch** (`--reader basic-pitch`) is an outside reader: Spotify's
[Basic Pitch](https://github.com/spotify/basic-pitch) note model, run in Rust through
[basic-pitch-rs](https://github.com/AThevon/basic-pitch-rs), an unofficial port, on the take's
own samples. It says where each note starts and ends; the performance then gives each note its
key, centre and residual on the take's tuning, as for the other readers, so Basic Pitch's own
A440 keys are only counted, never used. It is polyphonic and a reading is not: where its notes
overlap, the louder is kept whole and a quieter one cut back to where it no longer overlaps, or
dropped when less than half of it would be left. Capture states how many notes it found, cut
back, dropped and keyed differently (`basic-pitch: …`). Its model, Spotify's `nmp.onnx` from
basic-pitch v0.4.0 (230 kB, Apache-2.0), is built into duet and its SHA-256 checked by the
tests; its licence and Spotify's NOTICE are in `crates/duet-dsp/models/basic-pitch/`. Nothing is
downloaded. The reader is the `basic-pitch` feature, on by default; `--no-default-features`
builds duet without it. Known limits: Basic Pitch takes no tuning and knows nothing of where a
note settles, so a whole note is taken as held, its scoop in read as drift; and its notes reach
into frames the performance hears as unvoiced, which the loss's `voicing` term charges.

**Rhythm.** Without `--lane`, each hit is sorted by sound into `kick`, `snare` or `hh` (a closed
hat). With `--lane`, every hit goes to that lane. Known limit: a hit has one lane. A kick and a
hat sounded together, or within about 50 ms of each other, are heard as one hit and get one
lane. Takes of different lanes are meant to be put together later; for that, give each the same
`--tempo`, `--meter` and `--downbeat` so they line up.

**Before the downbeat.** A take rarely starts on its first bar line, and no note is dropped
from the score for coming before it. Notes before the downbeat that fit a short bar become a
pickup bar, followed by a change of meter at bar 2. Otherwise bar 1 (or more) is whole bars of
rests holding them. A drum grid always gets the rest bars, because grids number bars from 1
with no gaps. When quantizing leaves nothing in that lead-in, it is removed from the score and
the score says so. Each case is stated (`lead-in: none`, `a pickup of 1/4`,
`1 bar of rests`, `removed from the score`). The `.duet` always keeps the true time.

**Notes that share a step.** A quick run or a flam can put two notes, or two hits on one lane,
on the same step of the grid. The score cannot hold both, so the grid's step is halved (1/16,
then 1/32, then 1/64) until none do, and the grid used is stated:
`grid: 1/16 refined to 1/32, so no two notes share a step`. A `--grid` you give is refined the
same way. What a 1/64 still cannot part is merged into one, each merge stated: a melody keeps
the longest note's pitch, and a rhythm the strongest hit. The `.duet` keeps every note, and the
analysis gives the grid used, which the quantize offsets are counted in.

**Velocity.** A hit's or note's velocity is its strength relative to the take's strongest
(20 to 127). ABC holds only its dynamics' levels, so a melody's score snaps each velocity to
the nearest and states the count snapped. A rhythm's `.grid.duet` holds any velocity, so
nothing is snapped. The `.duet` keeps the raw values.

**Comparing a reading with the singing.** `duet capture compare` plots and plays a melody's
reading against its performance, so you can see and hear where the notes read depart from what
was sung. It reads the recording and what `duet capture melody` wrote at its stem:

```sh
duet capture compare idea.m4a                       # idea.compare.svg and idea.compare.wav
duet capture compare idea.m4a --from 33 --to 38     # five seconds of it: idea.compare.33.0-38.0.svg, .wav
duet capture compare idea.m4a -o scratch/idea       # the stem it was captured to
duet capture compare idea.m4a --reading hmm.capture.duet   # another reading of the take
```

- `STEM.compare.svg`: the performance's pitch in cents over time; the reading's notes as bars
  at their keys on the take's tuning (orange when between keys); the residual, the gap between
  them, shaded; and the onsets as ticks under it.
- `STEM.compare.wav`: the take in the left channel, as the analysis heard it (one channel at
  22 050 Hz, so it lines up with the reading sample for sample), and the reading in the right as
  a sine at the take's own tuning, gliding only through each note's slide in (its scoop, where
  the residual measured one). 32-bit float. The sine holds the note before through a gap shorter
  than 0.2 s, about a consonant or a catch-breath, so a phrase plays as one tone. It is silent
  through a gap of 0.2 s or more, a breath or a rest, so a note the reading drops is heard as
  silence, not as the note before held on. `--legato` holds it through every gap; `--gapped`
  silences it in every gap.
- On standard output, one line per note: when it starts, its key, its `centre` in cents, its
  residual's parts (where it is held, its drift in cents a second, its vibrato, scoop and fall,
  and the rest), and whether it is between keys; then the take's loss as its three terms (pitch,
  unread and notes). A reading written before `capture-analysis` 1.2 has no residual, and the
  output says so.

`--from` and `--to` (seconds) window all three, and name the files after the window
(`STEM.compare.FROM-TO.svg`, with `0.0` or `end` for a side not given), so a window never
collides with the whole take's files. `--reading` compares another analysis of the same
take: its reading is the bars and the sine, the stem's own is drawn beside it dashed, and the
outputs are named after it (`hmm.compare.svg`). Nothing is overwritten. Neither file the stem
holds names the recording it was read from, so the recording is checked only by its length
against the performance's.

**Correcting a capture.** Edit the ABC text, which is what it is for, or a rhythm's grid with
the `duet drums` commands, which read the `.grid.duet` by its envelope. To fix an
estimate, run again with `--tempo`, `--meter`, `--downbeat` or `--key` (add `-o` to keep the
first). Do not fix a wrong key by editing `K:` in the ABC: that changes every pitch written, not
only the spelling. Re-run with `--key`, which changes spelling alone.

**Not in this version.** Chords and polyphony, live input, telling tap timbres apart, and a
tempo that changes within a take (drift is reported in the analysis, not corrected).

**The reference corpus.** `crates/duet-capture/corpus` holds the recordings the capture
tests are measured against, each with a hand-checked transcription. Its
[README](crates/duet-capture/corpus/README.md) says how to add a take.

## Configuration

`duet` reads `duet.toml` and `duet.local.toml` from two places, each file over the ones
before it:

1. the user config directory: `~/.config/duet` on macOS and Linux (or `$XDG_CONFIG_HOME/duet`),
   `%APPDATA%\duet\config` on Windows, or whatever `DUET_CONFIG_DIR` names
2. a project's `.duet/` directory: the nearest one from the working directory up

```toml
[song]
end = { round-up = 4 }   # or "last-bar", the built-in

[play]
soundfont = "GeneralUser.sf2"   # for duet play; a relative path is this file's directory's

[lanes.shaker-2]         # a new lane: it needs a key; it goes in the "other" group
key = 82
aliases = ["sh2"]

[lanes.kick]             # a gm lane: only the fields given change
aliases = ["bd", "boom"]
```

- **`[song] end`** is the end a song that names none gets when a file is written.
  `"last-bar"` is the bar line after the last bar holding a note or control. `{ round-up = 4 }`
  carries that on to the next multiple of 4 bars. The end is always written into the file, so
  someone with other defaults reads the same song.
- **`[play] soundfont`** is the SoundFont `duet play` sounds songs with, over the installed
  one. `--soundfont` is over both.
- **`[lanes.<name>]`** adds a lane to the `gm` map, or changes one. The fields are `key`,
  `group` (one of the map's groups, or `other`) and `aliases`. A file never removes a
  built-in lane, so one that would put two lanes on one key is refused: move the other lane
  too. Each file must leave a valid map.
- **Config lanes are for reading.** A `.drums` row may use a lane or alias config adds, and
  `duet drums read` names notes by the configured keys. Written `.drums` files never depend on
  config: they say `map gm` and carry a `lane` line for every lane the built-in map does not
  name on that key, so they read the same with no config at all.
- **`map gm` is General MIDI on every machine.** When reading `.drums` text, a config that
  moves a gm lane's key or group does not apply: `kick` is still 36 there. The move still
  applies to `duet drums read`, where it describes your kit. `duet` says on standard error
  when a moved key doesn't apply, and `duet config show` lists it. A file's own `lane` line
  still moves a lane.
- A file that breaks a rule (bad TOML, a field `duet` does not know, two lanes on one key)
  is refused, naming the file and the rule.

```sh
duet config path   # the directories, and each file in the order it is read
duet config show   # the merged config as a duet.toml, each value with where it came from
```

## Development

Builds on the latest stable Rust (1.100 or later, edition 2024). On Linux, `duet play`'s
audio and MIDI crates need the ALSA headers: `apt install libasound2-dev pkg-config` (or your
distribution's package).

```sh
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

Beyond cargo, a developer's machine wants a few things CI does not. mise cannot install
abcmidi or FluidSynth (neither has a registry entry), so they are in the `Brewfile`:

```sh
scripts/dev-setup.sh                 # brew bundle, then fetch the pinned SoundFont
scripts/fetch-soundfont.sh           # just the SoundFont, to duet's data directory
scripts/port-check.sh                # duet play --port to FluidSynth, then check what arrived
scripts/port-check.sh --interrupt 4  # the same, with Ctrl-C after 4 seconds
scripts/capture-fixtures.sh [fixtures|corpus|all]   # regenerate duet-capture's audio (macOS)
scripts/duet-listen.sh TAKE --stem STEM [--reading hmm=OTHER.capture.duet] [--moments FILE]
                                     # hear a take against its readings, moment by moment
scripts/listen/check.sh              # duet-listen.sh's clips land at their windows
```

- **The SoundFont** is the one duet's installer will fetch, pinned in `scripts/soundfont.env`
  by commit and sha256: GeneralUser GS 2.0.3. It is never committed. `fetch-soundfont.sh` puts
  it at `default.sf2` in the data directory, so `duet play` sounds with no flags. It keeps a
  file already there with that sha256, and refuses to overwrite any other.
- **`port-check.sh`** (macOS) starts FluidSynth as a CoreMIDI port, sounding what it receives
  through the speakers and logging every event, then plays a song to it with `--port`.
  Played to the end, it passes when every note arrived and no key is left on. Interrupted, it
  passes when duet says it stopped, its last messages reset every channel it used (sustain
  off, all notes off), and no key is left on. This is the only check of `--port` against a
  real port; CI can't open one.
- **`capture-fixtures.sh`** regenerates the generated audio duet-capture's tests use: `fixtures`
  is the decode tests' one-second tones in each format, `corpus` the reference corpus's
  synthetic takes (the default), `all` both. Its encoders (`lame`, `flac`, `vorbis-tools`) are
  in the `Brewfile`; the AAC and HE-AAC files use macOS's own `afconvert`. CI uses the committed
  files, so run it only to change them.
- **`duet-listen.sh`** is for checking a reading by ear.
  - **What it builds:** clips of the whole take and of each named moment, for the stem's
    reading and for any `--reading` beside it. Each clip has the take on the left and the
    reading's sine on the right, raised 6 dB.
  - **Three versions of each clip,** each as `duet capture compare` renders it:
    - bridged, its default: the sine held through a gap under 0.2 s and silent at a longer one,
      so a dropped note is heard as a gap;
    - legato (`--legato`), with the sine held through every gap;
    - gapped (`--gapped`), with the sine silent wherever the reading has no note.
  - **Playing them:** a menu plays one clip at a time. `N` plays clip N bridged, `Nl` legato,
    `Ng` gapped, and `q` quits. While a clip plays, it shows where the clip is in the take and
    which note is being read.
  - **The moments file** is plain text, one moment per line: `FROM TO LABEL`, in seconds from
    the start of the take, with a free-text label. A line starting with `#` is a comment.
    `scripts/listen/synthetic-melody.moments` is an example.
  - **Clips of a licence-restricted take are copies of it.** Keep them, and its moments
    file, out of git.
  - **`scripts/listen/check.sh`** builds the synthetic corpus melody's clips and checks each
    one's length against its window. CI does not run it.
- **`ffmpeg`**, also in the `Brewfile`, is for converting recordings to a format capture reads.
- The Brewfile installs each tool's current version. The abc2midi oracle checks its own
  version against the corpus manifest.

CI also runs `cargo deny`, `cargo audit`, rustdoc with `-D warnings`, a build on the declared
`rust-version`, and the checks for the tasker-systems
[conventions](https://github.com/tasker-systems/conventions) baseline this repository is pinned to
(see [CLAUDE.md](CLAUDE.md) and `.conventions/`).

The round-trip tests in `crates/duet-midi/tests` generate MIDI files with humanized timing,
sustain pedal, pitch bend, tempo and meter changes (including one in the middle of a bar),
overlapping notes, and a format 0 file with several channels, at 96, 384, 480 and 960 ticks
per quarter, and check every kept event survives MIDI to text to MIDI unchanged. Those in
`crates/duet-drums/tests` generate drum grids in several meters (with a change of meter),
grids and resolutions, with any velocity, ending in empty bars or inside a bar, and check
grid → song → grid, through MIDI and drum-grid documents, and the text form all round-trip
unchanged. Every song the MIDI tests build also round-trips through a song document.
`crates/duet-schemas/tests/schemas.rs` is the schema gate. Each kind's JSON Schema, and the
envelope's, is generated from the Rust types and committed under `crates/duet-schemas/schemas/`,
with an example document beside it. The gate fails when a committed schema's shape differs from
the code at the same version (bump the kind's version), and asks only to regenerate when its
descriptions alone changed. Every committed example must still read, and validate against its
schema. To write a new or regenerated schema:

```sh
DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas
```

`crates/duet-dsp` is the analysis behind `duet capture`, tested on synthesized signals and on
a committed pYIN reading. `crates/duet-capture` decodes and assembles around it; its tests
decode a one-second tone in every format it reads and run the reference corpus through the
pipeline.
`crates/duet-abc/corpus` holds our own ABC tunes, MIT, one per construct the subset reads
plus six whole tunes, each with committed recordings of abc2midi's reading of it and of the
ABC our writer makes from it. CI compares our reading with those recordings
(`crates/duet-abc/tests/oracle.rs`). It never runs abc2midi. On a developer's machine, the
`oracle` feature also runs abc2midi and checks the recordings still match it:

```sh
brew install abcmidi                               # the version in corpus/manifest.toml
cargo test -p duet-abc --features oracle           # fails on any other abc2midi version
DUET_ABC_RECORD=1 cargo test -p duet-abc --features oracle recordings_match_live_abc2midi
```

The last line re-records, for a new corpus tune or a writer change; review its diff. When
our reading disagrees with a recording, re-recording is not the fix: either our reading of
the standard is wrong and is fixed, or abc2midi differs from the standard and the
difference is listed in the manifest with its reason. mise cannot pin abcmidi (no registry
entry, and the project publishes no release binaries), so Homebrew installs it and the
oracle checks its version against the manifest.

`crates/duet-e2e` holds the tests that drive the `duet` binary end to end. It is never
published. Each test run builds the binary with `cargo build -p duet-cli --bin duet` and runs
what that build reports, so a test never runs a stale binary (set `DUET_E2E_BIN` to run
another, such as one built from `main`). Its `golden/` holds what `duet` wrote from the
samples before a song could name an end, and a test checks that every file written now holds
the same music, event for event, and differs only by stating its end. CI runs it again, with
duet-cli's tests, under the hostile config in `crates/duet-e2e/hostile`, so no test reads a
developer's own config.
