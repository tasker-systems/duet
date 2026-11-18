# duet songs: a song, its composition, its parts, and the takes placed on its bars

**Date:** 2026-11-17. **Task:** the composition task (temper `01a1dc62-9d9f-73ae-b4ec-03b07a08e407`),
folded into the domain model by the owner's ruling on the project design (temper
`01a1eb60-07f8-77a6-8a0d-fb3947f0ba0d`, decision 10).

**Status: Proposed; songs task 2 is built** (`song` 1.0, its composition, `duet song new | show |
status`, and a part's `id` and `source`). The domain model under it is **Ruled** (owner,
2026-11-17, temper `01a1eb48-6b7a-7797-bb2b-68f335de87e9`, rulings 1 to 14, and 2026-11-18, temper
`01a1ee66-f71b-763c-8117-5198cf45aa14`, rulings 15 and 16), and each point taken from it is marked
so. Everything else is **Proposed**. What the owner has not ruled is listed in
[Open questions](#open-questions), and each place it touches says so.

**Reference pages,** drawn from the same rulings: the
[Domain Map](https://claude.ai/artifact/J7rcg5qQ1hL83EDrfchtfB) and the
[Lifecycle Map](https://claude.ai/artifact/T6QamgUyrWoEPD3UYU3b3a), beside Temper decisions
`01a1eb48` (the domain model) and `01a1eb60` (the project decisions). Where a map and this spec
disagree, the Temper decisions win.

**Builds on:**
- [duet documents](2026-11-15-duet-documents-design.md): the envelope, kinds, the version policy
  and the views. The `song` kind is renamed `part` there ([§2 of that design](2026-11-15-duet-documents-design.md#part-10)).
- [Projects](2026-11-17-project-design.md): the project as a workspace, `takes/` at its root, and
  one folder per song under `songs/`.
- [Variants and comps](2026-11-17-variants-and-comps-design.md): the take digest, readings as
  variants, and the comp, now across takes and anchored to a song's bars.
- [The bus](2026-11-17-bus-design.md): the journal, `journal.duet.jsonl`, which carries a song's
  history.

## What is ruled

**Ruled (owner, 2026-11-17, temper `01a1eb48`):**
1. **A session is a human-and-agent concept, not a duet entity.** duet does not encode it.
   Continuity is carried by the journal and the project's files.
2. **duet does DAW-like work directly on data, with no DAW interface.** That covers waveforms, note
   distributions, bends, tone and key shifts, voicings, patterns and progressions, generated chords
   and bass lines, and layering, over takes, ABC, percussion, MIDI and FLAC. The workbench is for
   seeing, listening, playing back, recording takes and working with project files. This refines the
   "not a DAW" boundary in the jobs note (section 4).
3. **A take belongs to the project** and is linked to a song and a part. It is recorded once and can
   serve more than one song or part.
4. **Comping works across takes,** anchored to the song's timeline (bars and beats) through its
   composition. Each take maps onto that timeline, usually by the click's tempo plus an offset.
5. **A song has a working version plus snapshots.** Snapshots are deliberate checkpoints; the
   working song keeps changing.
6. **The song and its composition are different things.** The **song** is the piece: its identity,
   intent, takes, parts, snapshots and history. The **composition** is how it is written down:
   sections, a tempo map, a meter map, a key map, and where each part sits on the timeline. A song
   that changes tempo and key is still one song; its composition records the changes.
7. **The `song` document kind is renamed `part`, now.** Nothing has been captured, distributed or
   committed under the old name, so there is no migration.
8. **A take's placement on the song's bars is one of three values.** `click`: recorded against the
   song's click, placed by the click plus an offset (a take from the planned Logic tap arrives this
   way). `estimated`: the default for a take with no click, from capture's tempo estimate. `set`:
   the owner and an agent settled the tempo in a working session and set it; a set placement
   replaces the estimate.
9. **A song's status is a closed, enumerated state machine:** `idea`, `sketch`, `developing`,
   `arranged`. The only moves are idea → sketch, sketch → developing, developing → arranged, and
   arranged → developing (reopen); duet refuses any other. The owner sets the status, or an agent
   does with the owner; duet never infers it. A snapshot is an event recorded against the song, not
   a status, so taking one doesn't change the song's state.
10. **There is nothing to migrate or retire.** No compatibility path from the old `song` kind:
    `part` starts at 1.0, and the new song document starts fresh at 1.0.
11. **The new song document's kind is named `song`** ("keep song as the kind name"): `song` 1.0 at
    `songs/<song>/song.duet`, holding the song and its composition.
12. **Before a first release, a format changes in place.** A change to a kind's body folds into its
    current version; the version policy applies from the first release or the first real file.
13. **A song and a part each have a stable id,** minted once when the document is made and never
    changed. The name (the folder or file stem) and the title stay free to rename. The journal's
    index lists songs and parts by id, with the name alongside, so a rename keeps the lookup of older
    segments. Its form (a UUIDv7 in the body), how a song refers to its parts, and what a copy
    keeps are ruled with it ([§1](#ids)).
14. **A song's current status lives in `song.duet`; the journal records each change.** Every move is a
    `song.status-set` event and every snapshot a `snapshot.taken` event, and `duet journal check`
    reports any disagreement between the document and the journal. It reports; it never repairs
    ([§7](#7-lifecycles)).

Rulings 12 to 14: owner, 2026-11-17, temper `01a1eb48`. Rulings 13 and 14 answer the bus design's
decisions 17 and 23, ruled on #80.

**Ruled (owner, 2026-11-18, temper `01a1ee66-f71b-763c-8117-5198cf45aa14`)**, in the owner's
words "status gates nothing, add source":
15. **A song's status gates no command.** Every command works on a song in any state; only
    `duet song status` refuses, and only a move the state machine does not draw ([§7](#7-lifecycles)).
16. **A part states its `source`,** comped, generated or by hand, in `part` 1.0, changed in place
    under ruling 12 ([§2](#2-parts)). Its exact shape was settled in the build of songs task 2.

**Agreed in the same discussion, as the model's working shape:**
- **A part is one voice, line or groove** (notes, or a drum grid). It always states its source:
  comped from takes, generated, or written by hand.
- **Operations are stated changes** over parts, songs and compositions, never overwrites: transpose,
  key shift, revoice, generate, layer, change tempo or meter, correct and retune.

**Ruled (owner, 2026-11-17, temper `01a1eb60`), from the project decisions:**
- **One folder per song:** `songs/<song>/` holds `song.duet` (the song and its composition),
  `parts/`, `comps/` and `snapshots/`.
- **Capture estimates, then compares.** It always estimates from the take, and states where it
  disagrees with the linked song's composition. A take is placed on the song's bars by the song's
  tempo map plus an offset. A value the owner gives a take is remembered for that take.

## The model in one view

```text
project                         a workspace on the owner's machine (.duet/project.duet)
├── takes                       recorded once, identified by digest, at takes/<take>/
└── songs                       one folder each, songs/<song>/
    └── song                    the piece: identity, intent, links to takes, history
        ├── composition         sections, tempo map, meter map, key map, placements
        ├── parts               one voice, line or groove each, with its source
        ├── comps               picks across takes, on the composition's bars
        └── snapshots           deliberate checkpoints of the song
```

- **A take is the project's, a part is the song's.** A take sits in `takes/` because it can serve
  several songs. A part sits in its song's `parts/` because its meaning is its place in that song.
- **A link runs from the song to the take,** never from the take to the song. The song names the
  takes it uses; the project's take entry names none of its songs. Which songs use a take is found by
  reading the songs. One place for each fact.
- **No session.** Nothing in this model records who was working, when a sitting began, or which
  agent was attached. The journal says what happened and the files say what is.

## 1. The song and its composition

**Proposed. A new kind, `song`, at `songs/<song>/song.duet`, whose body holds the song and its
composition as two parts of one document.**

```json
{
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/song/1.0.json",
  "duet": { "kind": "song", "version": "1.0", "tool": { "name": "duet", "version": "0.6.0" } },
  "body": {
    "id": "0193f2a4-7c1e-7b3a-9d10-5e2f8a6c4b01",
    "title": "Wayfaring Stranger",
    "status": "sketch",
    "intent": "slow and bare; voice first, the bass enters in the second verse",
    "takes": [
      { "take": "wayfaring", "digest": "sha256:9f2c…", "for": ["melody"],
        "placed": { "by": "estimated", "bpm": 88.6, "bar": "1.1.0", "at_secs": 1.057 } }
    ],
    "composition": {
      "ticks_per_quarter": 480,
      "tempo": [ { "at": "1.1.0", "bpm": 92.0 } ],
      "meter": [ { "at": "1.1.0", "sig": "3/4" } ],
      "key":   [ { "at": "1.1.0", "key": "G minor" }, { "at": "17.1.0", "key": "A minor" } ],
      "sections": [
        { "name": "verse 1", "at": "1.1.0" },
        { "name": "verse 2", "at": "9.1.0" },
        { "name": "lift",    "at": "17.1.0" }
      ],
      "placements": [
        { "part": "melody", "at": "1.1.0" },
        { "part": "bass",   "at": "9.1.0" },
        { "part": "kit",    "at": "9.1.0", "repeat": 2 }
      ]
    }
  }
}
```

### The song

- **`id`**, required: the song's stable id. **Ruled** (ruling 13). See [Ids](#ids).
- **`title`**, required: the song's name for people. The folder name (`wayfaring-stranger`) is its
  name for commands, with the take-name rule (lowercase with hyphens, unique in the project).
- **`intent`**, optional: what the song is meant to be, in the owner's words. duet never reads it.
- **`takes`**, possibly empty: the song's links to the project's takes (ruled: a take is linked to a
  song and a part).
  - `take` and `digest`: the take by its project name and its digest, so a renamed or re-located take
    is still known.
  - `for`: the parts this take serves in this song, by part name. Empty means linked but not yet
    serving a part.
  - `placed`, optional: how the take sits on this song's bars ([§4](#4-how-a-take-maps-onto-the-bars)).
    Absent means unplaced.
- **No `parts` list.** A part is a file in `parts/`, named by its stem. What the song plays is the
  composition's placements; a part placed nowhere is still the song's, kept for later.
- **`status`**, required: one of `idea`, `sketch`, `developing`, `arranged`. **Ruled** (ruling 9):
  a closed state machine, set by the owner or by an agent with the owner, never inferred
  ([§7](#7-lifecycles)). `duet song new` writes `idea`. A new state is a new variant of a closed
  enum: a minor, but the transitions change with it, so it comes back to the owner.
- **History is the journal's,** not the song's. Every change to a song, its composition or its parts
  is an event in the journal naming the song by id. The song document is the state now; the
  journal is how it got there; snapshots are the points the owner chose to keep.

### Ids

**Ruled** (ruling 13): a song and a part each carry an `id`, minted once and never changed.
- **What it is.** **Ruled** (owner, 2026-11-17, temper `01a1eb48` ruling 13): a UUIDv7 in its standard text form, minted by the command that makes the
  document (`duet song new`, a part written by `comp apply`, an operation, or by hand through
  `duet part new`). It sorts by creation time, and needs no registry.
- **Where it lives.** **Ruled** (owner, 2026-11-17, temper `01a1eb48` ruling 13): in the body, as `id`, beside `title` for a song and as a top-level
  body field for a part (folded into `part` 1.0 under ruling 12).
- **Renames.** Renaming a song's folder or a part's file, or changing a title, never changes the id.
  The journal's index names songs and parts by id, with the name alongside, so older segments stay
  found after a rename.
- **References.** **Ruled** (owner, 2026-11-17, temper `01a1eb48` ruling 13): inside a song, `placements` and a take's `for` name a part by its stem,
  as above, so `song.duet` stays readable by hand; `duet part rename` rewrites them in the same act
  and records it in the journal. The journal, comps and anything outside the song name it by id.
- **A copy is a new thing.** **Ruled** (owner, 2026-11-17, temper `01a1eb48` ruling 13): A snapshot keeps the ids it copied, since it is the same song at a
  point in time. Duplicating a song or a part to start a new one mints new ids.
- **Takes are unchanged:** a take's identity is its digest, as ruled in the variants design.

### The composition

**Ruled (owner, 2026-11-17):** sections, a tempo map, a meter map, a key map, and part placements.
**Proposed:** their shape.

- **Positions are `Position` strings,** `bar.beat.tick` from `1.1.0`, as a part already writes
  them, at the composition's `ticks_per_quarter`. One spelling of a place in time across every kind.
- **`tempo`:** changes in order, the first at `1.1.0`. Each holds until the next. Quarter notes per
  minute, as capture and `part` count it. A gradual change (a ritardando) is not in 1.0; it would be
  an optional `curve` on a change, a minor.
- **`meter`:** time-signature changes, the first at `1.1.0`, each on a bar line. A change off a bar
  line is refused by name.
- **`key`:** key changes, the first at `1.1.0`. The key is information for operations (a key shift,
  a generated bass line) and for the score; it changes no note.
- **`sections`:** named spans, each from its `at` to the next section's, the last to the song's end.
  Names are the owner's and need not be unique ("verse" twice is fine); a section is found by name and
  order ("verse 2"), or by its bar.
- **`placements`:** where each part sits.
  - `part`: a part's stem in `parts/`.
  - `at`: the bar position where the part's own `1.1.0` falls.
  - `repeat`, optional, default 1: how many times it plays back to back, for a groove.
  - `from` and `to`, optional: the stretch of the part that plays, in the part's own positions.
  - **Layering is several placements over the same bars.** Nothing else is needed for it.
- **The song's end** is the latest end of any placement, unless the composition states `end`.

### The composition's maps and a part's own conductor

A `part` (today's `Song`) carries its own tempo, meter and key, with their change lists. A placed part
plays on the composition's bars, so the two can disagree.

**Proposed: the composition is the clock, and the part keeps its conductor as written.**
- **Placed, a part's notes are read in bars and beats** and played at the composition's tempo map.
  Its own tempo is not applied.
- **A disagreement is stated, never fixed silently:** "bass is in 4/4; the song is in 3/4 at bar 9".
  A meter that disagrees is refused for placement unless `--force`, since every bar line would move;
  a tempo or key that disagrees is stated and placed.
- **The part is never rewritten to match.** Changing it is an operation ([§6](#6-operations-are-stated-changes)).
- Whether a placed part should drop its own conductor altogether is a question for the build: it
  would be a `part` major, so it is not proposed.

### Why one document for song and composition

The owner ruled them different things, and they are: the composition can be replaced wholesale (a
re-arrangement) while the song stays the song. They share a file because they change together, a
snapshot must hold both at one moment, and the project ruling names one file, `song.duet`, for both.
They are separate fields of the body, so a later split into two kinds moves a field and nothing else.

### The kind's name and version

**`song` 1.0, a new kind. Ruled** (rulings 10 and 11): there is nothing on disk to migrate or
retire, so the old `song` kind is simply renamed `part`, and this kind takes the name `song` and
starts fresh at 1.0, with no compatibility path between them.

## 2. Parts

**Ruled:** a part is one voice, line or groove, and always states its source.
**Ruled (owner, 2026-11-18, temper `01a1ee66`, ruling 16):** the `part` kind is today's `song` 1.0
renamed, at `songs/<song>/parts/<part>.duet`, and it states its `source` in `part` 1.0, changed in
place: `comped` (from takes, naming them and the comp when there is one), `generated` (naming the
operation and its inputs) or `hand` (written by the owner or by Claude).

**The shape, as built in songs task 2.** The body is `id`, `source`, then the part's own fields.
`source` is an object whose `by` says which:
- `{"by": "hand"}`: written by the owner or by Claude, in duet or imported from another program
  (the envelope's `from` says from what).
- `{"by": "comped", "takes": [{"digest": "sha256:…", "take": "wayfaring", "reading": "hmm"}],
  "comp": "verse"}`: at least one take, each by its digest, with its project name and the reader
  whose reading it is when known; the comp when there is one. `duet capture` writes a one-take
  comp with no comp.
- `{"by": "generated", "operation": "part quantize", "inputs": ["4693e75efa59623e"]}`: the
  operation, by its command's name, and the digest of each document it read (the FNV-1a of its
  body as compact JSON, as a view's `from` and `doc.written` name one). `part quantize` and
  `drums render` write it. An operation's settings are not stated yet: that is the
  transformations task's, with [question 1](#open-questions).

A part without `source`, or without `id`, is refused by name. A drum grid states neither yet.

- **A drum part is a `drum-grid` document** in `parts/`, placed the same way. A part is the role; the
  kind is what the file holds.
- **A part's name is its stem:** `melody`, `bass`, `kit`. Unique in its song.
- **A reading's notes become a part by being kept.** Capture writes a reading's notes beside the take
  (`takes/<take>/<take>.<reader>.duet`), derived and rebuildable. Copying one into a song's `parts/`
  is the act that makes it the song's, and it is written with `source: comped` naming the take and
  the reading. That is a one-take comp with no picks: the simple case.

## 3. Snapshots

**Ruled:** snapshots are deliberate checkpoints; the working song keeps changing.

**Proposed: a snapshot is a folder of copies, made once and never written again.**

```text
songs/wayfaring-stranger/snapshots/
  2026-11-20-first-verse/
    song.duet                 the song and composition, byte for byte as they were
    parts/melody.duet         every part the composition placed, byte for byte
    parts/bass.duet
    snapshot.duet             what the snapshot is
```

- **`duet song snapshot <song> <name> [--said TEXT]`** writes it, and an event in the journal.
  **Ruled** (ruling 9): a snapshot is an event recorded against the song, not a status, and taking
  one leaves the song's status as it was.
- **It copies,** because parts are working files that change. A snapshot that named them by digest
  would name content that no longer exists.
- **It holds what was placed,** not every file in `parts/`. A part kept for later and placed nowhere
  is not in the snapshot.
- **`snapshot.duet`** (a small `snapshot` 1.0 document) holds the name, the date, the owner's words,
  the digest of every file copied, and the takes the song linked with their digests. The takes are
  named, never copied: they are the project's and never change.
- **A snapshot is never modified.** A file inside one that differs from its stated digest is reported
  by `duet song show`, naming it.
- **Restoring** copies a snapshot's `song.duet` and parts back over the working ones, after a
  snapshot of the working song is taken, so nothing is lost by restoring. It is an operation like any
  other, stated in the journal.

## 4. How a take maps onto the bars

**Ruled (project decision 6):** capture estimates, then compares. A take is placed by the song's tempo
map plus an offset (the click case). A value the owner gives a take is remembered for that take.

### A placement and how it was made

**`placed`** says where the song's bars fall in the take: `bar` falls `at_secs` into it, and every
other bar follows from a tempo. `by` says where that tempo came from, one of three:
- **`click`:** the take was recorded against a click playing the song's tempo map, so the map is
  the take's tempo exactly. **Ruled** (ruling 8, and project decision 6: the song's tempo map plus
  an offset). A take from the planned Logic tap arrives this way.
- **`estimated`:** the take had no click, and `bpm` is capture's estimate of its tempo. **Ruled**
  (ruling 8): the default for a take with no click.
- **`set`:** the owner, or an agent with the owner, settled the tempo and set it. **Ruled** (ruling
  8): a set mapping replaces the estimate, and says so.

All three values are **Ruled (owner, 2026-11-17, temper `01a1eb48` ruling 8)**.

### Placed by the click

The usual case: the take was recorded against a click playing the song's tempo map.
- **`placed: { "by": "click", "bar": "1.1.0", "at_secs": 1.057 }`** says that the song's bar `1.1.0`
  falls 1.057 s into the take. Every other bar's time follows from the tempo map.
- **The offset is per song,** in the song's link, because one take can serve two songs at different
  bars.
- **`duet record` in a song** (`--song wayfaring-stranger`) plays the click from the composition and
  writes the placement itself: the offset is known exactly, since duet started both.
- **`duet song place <song> <take> --click --bar 9 --at 31.4`** places a take recorded elsewhere,
  against a click of the same tempo map.

### Estimate, then compare

- **Capture always estimates** tempo, meter, downbeat and key from the take, as today.
- **When the take is linked to a song,** capture compares its estimate with the composition at the
  take's placement, and states each disagreement: "tempo estimated 88.6 bpm; wayfaring-stranger is
  92 at bar 1", "key estimated A minor; the song is G minor at bar 1".
- **It never changes the composition,** and never changes its own estimate to match. Both stay as
  they are; the statement is the result.

### Values given to a take

- **A value the owner gives** (`--tempo 92`, `--meter 3/4`, `--key "G minor"`) is remembered on the
  take's entry in the project document, as `given` ([the project design](2026-11-17-project-design.md#2-the-project-document-project-10)).
- **Every later capture of that take uses it,** stated as given, as `--tempo` is today. A flag on the
  command line still wins over it.
- **It belongs to the take, not the song,** since it is a fact about the recording.

### Takes without a click

**Ruled (owner, 2026-11-17, ruling 8):** placed by an estimated tempo by default; a tempo the owner
and an agent settle and set replaces the estimate; the mapping records which.
- **Linking a take with no click to a song places it,** `by: estimated`, with capture's tempo and
  its first downbeat as `bpm` and `at_secs`. Capture states the placement, and where its tempo
  disagrees with the composition, as for any take.
- **`duet song place <song> <take> --bpm 90 --bar 1 --at 1.20`** sets it, `by: set`. Every flag not
  given keeps its estimated value, and the whole mapping is then `set`.
- **A set mapping is never replaced by a later estimate.** Capturing the take again re-estimates and
  states any disagreement with the set mapping; it does not move the placement.
- **One tempo for the whole take, in 1.0.** A take that drifts is placed by its overall tempo, and the
  drift is the capture's to state, as it is today. A tempo map of the take's own would be a minor.

## 5. Comps across takes

**Ruled:** comping works across takes, anchored to the song's bars through the composition. A one-take
comp is the simple case.

**Proposed here, and reshaped in [the variants design](2026-11-17-variants-and-comps-design.md#amended-comps-across-takes):**
- **A comp lives in its song's `comps/`,** names the part it builds, and picks from the readings of
  any placed take linked to that part.
- **A pick is stored in bars,** as a span of the composition's timeline, and found in each take
  through that take's placement. The seams rule still holds: an edge must be a seam of every reading
  the span touches, read in that take's own frames.
- **A take that is not placed cannot be picked from.** The comp refuses it, naming the take.
- **Applying a comp writes the part** it names, with `source: comped` listing the comp and every take
  it drew from.

## 6. Operations are stated changes

**Ruled:** operations are stated changes over parts, songs and compositions, never overwrites.

**Proposed: each operation names what it read, what it wrote and what it changed, and an event in the
journal records the act.**

| Operation | Over | What it states |
|---|---|---|
| transpose, key shift | a part, or the composition's key map | the interval, and every note moved |
| revoice | a part | the chords before and after |
| generate | the composition and parts (a bass line from the chords, a chord part from the melody) | the operation, its inputs by digest, and its settings |
| layer | the composition | a placement added |
| change tempo or meter | the composition's maps | the change, and every placement whose bars move |
| correct, retune | a part, through a comp's marks | each note changed, as the comp design states it |
| restore a snapshot | the song | what was replaced, and the snapshot taken first |

- **Nothing is overwritten unasked.** An operation over a part writes a new part (`-o`), or, with
  `--onto`, replaces the working file after the journal holds the change, as `duet apply --onto`
  does for a view.
- **A generated part states its source** as `generated`, naming the operation and the digest of each
  input, so it can be made again.
- **Where the change is stored** is **open** ([question 1](#open-questions)): a new part plus a step in
  the song's history (the journal), or a chain of changes carried by the part. The transformations
  task (temper `01a1df08-2b63-7658-8812-7823c97c6147`) is to settle it. Until then, every operation
  above is designed to work under the first, which needs no new field.

## 7. Lifecycles

Each is **Proposed** unless marked.

- **A take:** added (copied or named), then found or missing on this machine, then placed or unplaced
  in each song that links it. It is never removed by a command (the project design's `take forget`
  drops the entry and leaves the files). The Domain Map also draws a take's working stages
  (recorded, captured, reviewed, comped, kept, set aside, superseded); those are not ruled, and this
  spec stores none of them.
- **A part:** written (by capture, a comp, an operation or by hand), then placed or not, then changed
  by operations. Its `source` is set when it is written and changes only when an operation writes it
  again.
- **A song:** **Ruled** (ruling 9). A closed state machine, set by the owner or an agent with the
  owner, never inferred:

  ```text
  idea ──▶ sketch ──▶ developing ──▶ arranged
                          ▲              │
                          └── reopen ────┘
  ```

  - `duet song new` makes it at `idea`. `duet song status <song> <state>` moves it, and any move not
    drawn above (idea → developing, arranged → sketch, a state to itself) is refused, naming the
    current state and the moves allowed from it.
  - Each move is a `song.status-set` event in the journal, with who made it. **Ruled** (ruling 14):
    `song.duet` holds the current status and the journal holds each change. `duet journal check`
    reports a song whose status disagrees with its last `song.status-set`, naming both; it never
    picks one and rewrites the other.
  - **A snapshot is an event recorded against the song, not a status.** Taking one never changes the
    song's state, and no state requires one.
  - **Ruled (owner, 2026-11-18, temper `01a1ee66`, ruling 15):** the status gates nothing. Every
    command works on a song in any state; the status is the owner's record of where the song
    stands.
- **A comp:** **open** ([question 2](#open-questions)). The reading on the table is drafting (entries
  being made), applied (its part written from it), stale (a variant read again, a take re-placed, or
  the composition changed under its bars since it was applied) and refused (it no longer places). The
  variants design's rules give stale and refused for a re-read; a change to the composition as a cause
  of stale is new and unruled.
- **A snapshot:** made, then never changed.

## Open questions

Not ruled by the owner. Each is recorded here, not decided.

Ruled since first written, and no longer open: how a take is placed (ruling 8,
[§4](#4-how-a-take-maps-onto-the-bars)), song states (ruling 9, [§7](#7-lifecycles)), the song
kind's name and version (rulings 10 and 11, [§1](#the-kinds-name-and-version)), that the status
gates nothing (ruling 15, [§7](#7-lifecycles)), and a part's `source` (ruling 16, [§2](#2-parts)).

1. **Where an operation's change is stored:** a new part plus a step in the song's history, or a chain
   of changes carried by the part. For the transformations task ([§6](#6-operations-are-stated-changes)).
2. **The comp lifecycle** (drafting, applied, stale, refused, with stale also caused by a change to the
   composition): Claude's reading of the comp design, not ruled ([§7](#7-lifecycles)).

## What this changes in other specs

- **[duet documents](2026-11-15-duet-documents-design.md):** `song` becomes `part` 1.0, with a
  proposed `source`.
- **[Projects](2026-11-17-project-design.md):** `songs/<song>/` in the layout; `given` on a take.
- **[Variants and comps](2026-11-17-variants-and-comps-design.md):** comps across takes, on bars.
  Tasks 2 to 4 change.

## Build tasks this design names

**Proposed, not filed until the owner rules on this design.** Each names its witness.

1. **The rename:** the `song` kind becomes `part` 1.0 in code, schema path and messages. **Needs:**
   nothing. **Witness:** every round trip in `crates/duet-midi/tests/roundtrip.rs` passes under the
   new name, and the committed example is `schemas/part/examples/1.0.duet`.
2. **`song` 1.0 and the composition:** the type, `Kind`, schema and example; `duet song new | show`.
   **Needs:** 1, and the project design's task 1. **Witnesses:** a song document round-trips exactly
   and validates; a meter change off a bar line, an unknown part in a placement, and two parts of one
   name are each refused by name; `duet song status` makes each of the four allowed moves, refuses
   every other by name, and leaves the document unchanged when it refuses; `duet song new` mints an
   id, and renaming the song's folder leaves it unchanged; a part gains its `id`; a status changed
   by hand in `song.duet` is reported by `duet journal check`. **Built**, except the last
   witness, which lands with the journal follow-up (temper
   `01a1ec52-eff4-76bd-8470-33e544e7e2bf`), since no command writes `song.status-set` until it
   does. A song's `takes` and their `placed` come with task 3, in place under ruling 12.
3. **Placing takes:** `placed`, `duet song place`, `duet record --song`, and capture's comparison with
   the composition. **Needs:** 2. **Witnesses:** a take recorded with `--song` has its offset to the
   frame; a take with no click is placed `estimated` with capture's tempo, and `duet song place`
   with `--bpm` makes it `set`; a capture of a placed take states a tempo that disagrees with the
   song, and changes neither.
4. **Snapshots:** `duet song snapshot` and restore. **Needs:** 2. **Witnesses:** every copied file is
   byte for byte its source; restore takes a snapshot first; a changed file in a snapshot is reported;
   taking a snapshot leaves the song's status unchanged.
5. **Playing a song:** `duet play` of a song, the parts at their placements on the composition's
   clock, with disagreements stated. **Needs:** 2. **Witness:** a part placed at bar 9 with
   `repeat: 2` renders its notes at bars 9 and following, at the composition's tempo.

Comps across takes are the variants design's tasks 2 to 4, as amended.
