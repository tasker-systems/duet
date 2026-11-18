# duet documents: design

This designs how [the decision that duet's files are projections of versioned data
structures](#the-decision-this-builds) is built. That decision is settled and is not reopened here.
It answers the eight questions of the design task, names the build tasks in order, and holds the
performance layer's file until its envelope exists.

It takes in the two amendments the owner made to the decision on 2026-11-15, while this design was
being written: `.notes` and `.drums` are no longer file extensions, and a view reads back into a
document as a working surface. Questions 5 and 7 answer to them.

The owner ruled the seven open questions on 2026-11-15, each as recommended except the old JSON and
TOML files, which are cut now. Each is marked **Ruled (owner, 2026-11-15)** where it is answered, and
all seven are listed in [The owner's rulings](#the-owners-rulings). Everything else is **Proposed**.

**Amended 2026-11-17: the `song` kind is renamed `part`.** **Ruled (owner, 2026-11-17, temper
`01a1eb48-6b7a-7797-bb2b-68f335de87e9`, ruling 7).** A part is one voice, line or groove (notes, or
a drum grid's role in a song), and "song" now names the piece those parts belong to
([songs and composition](2026-11-17-songs-and-composition-design.md)). Nothing has been captured,
distributed or committed under the old name, and nothing on disk needs carrying over, so the kind
is simply `part` 1.0, its body unchanged. This design was written and built as `song` 1.0. Its
kinds table, schema paths, views table, view headers, messages and examples now say `part`;
elsewhere, in prose written before the rename, "a song" or "a song document" means a `part`
document. `Song`, the Rust type, and the CLI's `duet song` commands keep their names until the code
change that renames them; this amendment renames only the kind. See [part 1.0](#part-10).

## The decision this builds

Temper decision `01a1df20-1918-7130-b2d6-90ab20979553`, made by the owner on 2026-11-15, in short:

- Each data structure is a **kind** with a JSON Schema and an explicit version. The document is the
  canonical form.
- One **envelope** gives the kind and the schema version. The file says what it is, not its name.
- `.duet` is a JSON document. `.duet.jsonl` is a stream whose first line is the envelope. These are
  the only two extensions.
- The one-note-per-line text and the drum grid text are **views the CLI produces** from a document.
  They are not file extensions. **A view reads back into a document**, as a working surface: a person
  or an agent edits it until it plays as intended, then reads it in. Each view is declared lossless
  or names what it drops, and reading it back over its source keeps what it does not show.
- ABC and MIDI are **outside formats**. ABC gets no JSON Schema of ours.
- **Tensors** are a declared shape and number type over a flat array.
- Schemas are generated from the Rust types with `schemars` and committed. A CI check fails when a
  schema changes without a version change.

## What the Rust types serialize to today

Read from the code at `main` (4bd50df), and checked by generating `Song`'s schema with
`schemars` 1.2.3 in a scratch checkout.

| Type | Crate | Serialize / Deserialize | What it writes today |
|---|---|---|---|
| `Song` | duet-core | both | An object. `title`, `key`, `end` and the three change lists are left out when empty, never `null`. Read from `.json` and `.toml`, then normalized. |
| `Track`, `Note`, `TempoChange`, `TimeChange`, `KeyChange` | duet-core | both | Plain objects. |
| `Control` | duet-core | both | Internally tagged: `{"type":"cc","at":…,"controller":…,"value":…}`, `bend`, `program`. |
| `Position`, `TimeSig`, `Key`, `Grid` | duet-core | both, by hand (`string_serde!`) | Strings: `"1.3.240"`, `"6/8"`, `"D minor"`, `"1/16"` (or a triplet form). |
| `Pitch` | duet-core | both, by hand | A string with its spelling: `"Db4"` and `"C#4"` are both kept. |
| `DrumGrid` | duet-drums | both | The `Song` conductor fields, then `grid`, `channel` (always written, read with a default of 10), `levels` (left out when default), `lanes`, and `bars`. |
| `Bar` | duet-drums | both | `{"bar":1,"steps":16,"vel":[[…],[…]]}`: a nested array, one row per lane. |
| `Lane`, `Levels` | duet-drums | both | Plain objects. |
| `LaneMap`, `LanePatch` | duet-drums | read only, from TOML, `deny_unknown_fields` | Config, never written. |
| `Analysis`, `QuantizeOffset` | duet-capture | **write only** | An object. `key` and `grid` are written as `null` when absent. |
| `Event`, `HitLane` | duet-dsp | both | Internally tagged events with frames, not seconds; the rate is on `Analysis`. |
| `Timing`, `BeatTempo` | duet-dsp | both | `tempo_alternatives` is a tuple, written as `[half, double]`. |
| `Origin<T>` | duet-dsp | both | Externally tagged: `{"given":92}` or `{"estimated":{"value":92,"confidence":0.8}}`. |
| `KeyEstimate` | duet-dsp | **write only** | `histogram` is 12 numbers; `scores` is 24 `[key, score]` pairs. |
| `PitchFrame` | duet-dsp | neither | Not serialized yet. |
| the `duet.toml` config | duet-cli | read only, from TOML, `deny_unknown_fields` | Config, never written. |

### What that means for v1

- **`Song` can become `part` 1.0 (built as `song` 1.0) unchanged.** The body of a part document is byte for byte what
  `.json` holds today. Converting costs nothing.
- **`DrumGrid` changes in one place.** Its velocity matrix is a nested array per bar, and the decision
  makes a matrix a declared tensor over a flat array ([question 2](#2-the-kinds-and-their-v1-schemas)).
- **`Analysis` cannot be read back today.** `Analysis`, `QuantizeOffset` and `KeyEstimate` derive
  `Serialize` only, so `.capture.json` is write-only. A document kind needs a reader, so they gain
  `Deserialize`. That adds a reader; it changes no shape.
- **`Analysis` writes `null` where `Song` omits.** `key` and `grid` are written as `null`, while every
  optional `Song` field is left out. v1 picks one rule ([question 2](#2-the-kinds-and-their-v1-schemas)).

### What is surprising

- **Unknown fields are ignored today.** `Song` and `DrumGrid` do not deny unknown fields, so a `.json`
  song with a misspelled field reads without it and writes back without it. Only the config and lane
  maps deny them. The version policy needs them denied ([question 3](#3-the-version-policy)).
- **Floats are not parsed exactly.** `serde_json` is used without its `float_roundtrip` feature. Its
  default parser may land one unit in the last place off for some decimals. The writer's shortest
  form is exact, but reading it back is not guaranteed to be. Songs survive because their numbers are
  short (tick lengths such as `0.333`, tempos MIDI can store). A pitch curve in cents would not. The
  feature adds no dependency.
- **A derived schema would lie about five types.** `Position`, `TimeSig`, `Key`, `Grid` and `Pitch`
  serialize as strings by hand. A derived `JsonSchema` would describe their fields, not the string.
  Each needs a hand-written `JsonSchema` giving `type: string` and a `pattern`. This is the failure
  API-JSON-2 records in tasker-core, where a derived schema described a shape that was never sent.
- **Doc comments become the schema's `description`s.** Rewording a doc comment changes the generated
  schema, and rustdoc link syntax such as ``[default end](Song::default_end)`` ends up in it. The gate
  must tell a description change from a shape change ([question 4](#4-the-schema-gate)).
- **Ranges are wider in the schema than in the reader.** `vel` is a `u8`, so the schema allows 0 to
  255, while the reader allows 1 to 127. The schema says what parses, and the reader says what is
  valid, unless each range is stated with `#[schemars(range(…))]`.
- **The capture design says the analysis holds events "in seconds as well as frames".** It holds
  frames and the rate.
- **`.duet` already names a directory.** A project's config lives in `<project>/.duet/duet.toml`, so
  the project kind cannot be a file named `.duet` at the project root
  ([question 2](#2-the-kinds-and-their-v1-schemas), project).

## 1. The envelope

**Proposed: a wrapper, with the envelope first.**

```json
{
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/part/1.0.json",
  "duet": {
    "kind": "part",
    "version": "1.0",
    "tool": { "name": "duet", "version": "0.3.0" }
  },
  "body": { "ticks_per_quarter": 480, "tempo_bpm": 92.0, "time_sig": "4/4", "tracks": [] }
}
```

- **`duet.kind`:** the kind's name, lowercase with hyphens: `part`, `drum-grid`, `capture-analysis`,
  `performance`.
- **`duet.version`:** `"<major>.<minor>"` of the kind's schema. There is no patch level
  ([question 3](#3-the-version-policy)).
- **`duet.tool`:** the name and version of what wrote the file. It is information, never checked, and
  a reader never branches on it.
- **`duet.from`, optional:** where the body came from, when it was imported from an outside format
  ([question 6](#6-outside-formats)).
- **`duet.statements`, optional:** what the tool that wrote the body inferred or changed and must say,
  one sentence each: "tempo 92, estimated (confidence 0.81)", "velocities snapped to the grid's
  levels". Today capture writes these as `#` comments in `.notes` and `.drums`, so the files say so
  themselves (the `an-inferred-tempo-or-meter-is-stated` goal). JSON has no comments, so they move
  here, and every view renders them as comment lines under its header
  ([question 5](#5-views-rendered-and-read-back)).
- **`$schema`:** the URL of the whole file's schema. Editors that read `$schema` (VS Code, IntelliJ)
  then validate a `.duet` file as it is typed. The reader checks that it agrees with `kind` and
  `version`, and refuses a file where they disagree.
- **`body`:** the kind's value, exactly as its Rust type serializes.

**Why a wrapper rather than top-level keys:**

- **The body is the type, unchanged.** `part` 1.0's body is today's `Song` JSON, with no added keys.
  Top-level keys would put `duet` beside `tracks`, so the body would no longer be the type.
- **Unknown fields can be refused.** Top-level keys would need `#[serde(flatten)]`, and serde cannot
  combine `flatten` with `deny_unknown_fields`. The version policy needs the latter.
- **The reader dispatches before it parses the body.** It reads `duet`, picks the kind, and only then
  parses `body` as that kind's type. serde writes struct fields in order, so `duet` is always before
  `body` in a file duet wrote.

**Where the `$id`s live.** **Ruled (owner, 2026-11-15).** Option (a), a raw GitHub URL in this repository.

- **Options:**
  - (a) a URL in this repository: `https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/<kind>/<major>.<minor>.json`;
  - (b) a URL on a domain the owner holds;
  - (c) a repository path with no URL, such as `duet:crates/duet-schemas/schemas/part/1.0.json`.
- **Why (a):** it resolves today with nothing to host, and editors can fetch it. A
  committed schema file is never edited once its version ships ([question 4](#4-the-schema-gate)),
  so the `main` URL is stable. Moving to (b) later is a change to the `$schema` value only, which a
  reader can accept both ways.

**A stream's lines.** A `.duet.jsonl` file holds one JSON value per line.

- **The first line** is the envelope with a `head` in place of `body`: the stream's own header, such
  as the performance's rate, hop, first frame and unit.

  ```json
  {"$schema":"…/crates/duet-schemas/schemas/performance/1.0.json","duet":{"kind":"performance","version":"1.0","tool":{"name":"duet","version":"0.3.0"}},"head":{"rate":22050,"hop":256,"first_frame":0,"unit":"absolute-cents"}}
  ```
- **Every later line** is one record of the kind, and nothing else. It validates against the kind's
  record schema (`$defs/Record` of the same file) on its own, without the envelope or any other line.
  So a stream can be appended to, cut at any line, or read as it is written.
- **A stream kind has two parts in its schema:** `Head` and `Record`.

## 2. The kinds, and their v1 schemas

Each kind is a Rust type that serializes as its body. The `Kind` trait, defined in `duet-schemas`,
gives each one its name and current version ([where things live](#where-things-live)):

```rust
pub trait Kind: Serialize + DeserializeOwned + JsonSchema {
    const NAME: &'static str;        // "part"
    const VERSION: Version;          // 1.0
    /// Reads a body written at an older minor of this major. The default parses it as the
    /// current type, which is right while every minor is additive.
    fn read_minor(minor: u16, body: serde_json::Value) -> Result<Self>;
}
```

| Kind | v1 | Body type | Change from today |
|---|---|---|---|
| `part` (built as `song`) | 1.0 | `duet_core::Song` | **None on write.** Read gains `deny_unknown_fields`. Renamed from `song`, 2026-11-17. |
| `drum-grid` | 1.0 | `duet_drums::DrumGrid` | The velocity matrix becomes one tensor. `Bar.bar` goes. |
| `capture-analysis` | 1.0 | `duet_capture::Analysis` | Gains a reader. Absent values are omitted, not `null`. `tempo_alternatives` is named. |
| `performance` | 1.0, a stream | new `duet_dsp` types, `PerformanceHead` and `FrameBlock`, written and read only through `duet-schemas` | New: the performance design's question 1, in this envelope. |
| `project` | named, deferred | none yet | Designed in [the project design](2026-11-17-project-design.md). |
| `song` (the piece) | 1.0, **Ruled** name and version; design **Proposed** | none yet | A new kind, the song and its composition: [songs and composition](2026-11-17-songs-and-composition-design.md). |
| lane map | **not a kind** | | Config. |
| `duet.toml` | **not a kind** | | Config. |

### part 1.0

**Built as `song` 1.0** (`crates/duet-schemas/schemas/song/1.0.json`). **Renamed `part` 1.0**
(ruled, owner, 2026-11-17): the schema moves to `crates/duet-schemas/schemas/part/1.0.json`, its
example to `schemas/part/examples/1.0.duet`, and `$schema`, `duet.kind` and every message that
names the kind say `part`. The body is unchanged, so it stays 1.0. The rename is a code change
still to come (the songs design's build task 1).

- **Why the rename:** "song" is the owner's word for the piece, with its identity, intent, takes,
  snapshots and composition. What this kind holds is one voice, line or groove of it: a melody, a
  bass line, a hummed idea. Calling it a song would leave the piece without a name.
- **Nothing to carry over.** No `song` document exists anywhere that matters, so the rename has no
  compatibility path of any kind: `part` 1.0 is today's body under its new name. The name `song` is
  then free for the piece, a new kind that starts at 1.0
  ([songs and composition](2026-11-17-songs-and-composition-design.md#the-kinds-name-and-version)).
Its rules, unchanged by the rename:

- **Derives from** `Song`, `Track`, `Note`, `Control`, `TempoChange`, `TimeChange`, `KeyChange`, with
  hand-written schemas for `Position`, `TimeSig`, `Key`, `Grid` and `Pitch`. Each gives its string
  `pattern`, the one its `FromStr` reads.
- **Unchanged on write.** Every field, its name, its type and its omission rule stay as they are.
- **Stricter on read:** `#[serde(deny_unknown_fields)]` on each body type. A misspelled field is
  refused by name rather than dropped. This is a reader change, not a shape change: everything duet
  ever wrote still reads.
- **Ranges are stated** with `#[schemars(range(…))]` where the reader holds one: `vel` 1 to 127,
  `channel` 1 to 16, `program` and `controller` 0 to 127, bend −8192 to 8191, `ticks_per_quarter` 1
  to 32767. The schema then says what the reader accepts.
- **A part has a stable `id`.** **Ruled** (owner, 2026-11-17, temper `01a1eb48` ruling 13): minted
  once when the part is made, never changed by a rename, and folded into `part` 1.0 under ruling 12.
  It is a UUIDv7 in the body; its full rules are in [songs and composition](2026-11-17-songs-and-composition-design.md#ids).
- **A written part always states its end.** Every writer already does this (`io::with_end`), so a
  part document's `end` is always present, though the type keeps it optional.

#### `source`, proposed for the next free minor of part

**Superseded: built in `part` 1.0, in place.** The owner ruled on 2026-11-18 (temper
`01a1ee66-f71b-763c-8117-5198cf45aa14`) that a part states its source in `part` 1.0, changed in
place under ruling 12, with its exact shape settled in the build of songs task 2. That shape is in
[songs and composition §2](2026-11-17-songs-and-composition-design.md#2-parts), and it is
**required**, not optional. It departs from the proposal below in four ways, each for the reason
given:
- **Tagged inside, by `by`** (`{"by": "hand"}`), not outside (`{"hand": {…}}`): one spelling with
  a placement's `by`. It is still exactly one of three, and a new source is a new variant.
- **`hand` names no one.** No command knows who is at the keyboard; the journal records who did
  each act.
- **`generated` names each input by its body's digest, not by a path.** A path changes with a
  rename, and an operation's input may lie outside any song; the digest names the content. The
  operation's settings are not stated yet: that waits on the transformations task.
- **`comped` names each take by its digest, with its project name and the reader when known, and
  the comp by its name.** A reading's passes and the reader's cost are in the capture's analysis,
  which names the same take.

What follows is the proposal as it was written.

**Proposed** (the domain model's working shape: a part always states its source). An optional field
is additive, a minor (1.1, unless the ornaments work takes 1.1 first); making it required would be a
major, so the minor adds it optional and every writer fills it. Its absence means "not stated", as
in a part written before that minor.

```json
"source": { "comped": { "takes": [ { "take": "wayfaring", "digest": "sha256:9f2c…" } ],
                        "readings": [ { "reader": { "name": "hmm", "note_cost": 4.0 }, "passes": ["breathy", "rejoin"] } ],
                        "comp": { "path": "comps/melody.comp.duet", "body": "9c04e1b2a7d35f60" } } }
"source": { "generated": { "operation": "bass-from-chords", "from": [ { "path": "parts/chords.duet", "body": "3f9a1c0d5e2b7a41" } ],
                           "settings": { "register": "E1-E3" } } }
"source": { "hand": { "by": "owner" } }
```

- **`comped`:** from takes. It names each take by its project name and digest, each reading by its
  annotation (reader and passes), and the comp document when there is one, by its path inside the
  song's folder and the FNV-1a digest of its body. A capture's own notes, from one reading of one
  take with no comp, are the simple case: `comp` absent.
- **`generated`:** made by an operation. It names the operation, each input by its path inside the
  song's folder and its body digest, and the operation's settings, so it can be made again.
- **`hand`:** written by a person or an agent, through a view or by hand. `by` is `owner` or
  `claude`, as the bus's `who` spells them.
- **Exactly one of the three.** An externally tagged enum, so a new source is a new variant: a minor.
- **Imported from ABC or MIDI** is `hand`, with the import recorded where it already is, in the
  envelope's `from` ([question 6](#6-outside-formats)).

### drum-grid 1.0

Today: `bars: [{ "bar": 1, "steps": 16, "vel": [[…per step…], …per lane…] }, …]`.

**v1.** **Ruled (owner, 2026-11-15).** Option (a) below: one tensor for the whole grid, with `bar_steps`.

```json
"bar_steps": [16, 16, 12],
"vel": { "shape": [3, 44], "dtype": "u8", "data": [ … 132 numbers … ] }
```

- **`vel` is one tensor of lanes × steps for the whole grid,** row-major. A lane's row is every step
  of every bar, in order.
- **`bar_steps` gives each bar's step count,** from bar 1. Bars can differ in length when the meter
  changes, so bar lines are read from it. Its sum is the tensor's second dimension.
- **`Bar.bar` goes.** Bars are every bar from 1 in order, so the number is the index plus one. A
  stored copy can only disagree with it.
- **The rest of `DrumGrid` is unchanged:** the conductor, `grid`, `channel`, `levels` and `lanes`.

**How the drum matrix is held.** **Ruled (owner, 2026-11-15).** Option (a).

- **Options:**
  - (a) one lanes × steps tensor for the grid, with `bar_steps`;
  - (b) one lanes × steps tensor per bar;
  - (c) keep today's nested arrays, and call a rectangular nested array a tensor.
- **Why (a):** it is what the decision describes: a drum grid is a lanes × steps
  matrix. It is one shape check rather than one per bar, and a transformation over the whole grid
  (shift, mask, density) is one array operation. The legible form is the `grid` view, which is
  unchanged.
  (c) costs nothing to convert but breaks the decision's "declared shape over a flat array" for the
  first matrix it names.

### The tensor

```rust
pub struct Tensor<T> {
    pub shape: Vec<u32>,
    pub dtype: Dtype,   // "u8" | "i16" | "u32" | "f32" | "f64"
    pub data: Vec<T>,
}
```

- **Row-major,** last index fastest.
- **Checked on read:** `data.len()` equals the product of `shape`, and `dtype` names `T`. Either
  mismatch is refused, naming the tensor's field, both lengths or both types.
- **`dtype` is stated, though the Rust type fixes it,** so a reader in another language knows what the
  numbers are without the schema. Each use pins it with a `const` in the schema.
- **Floats are finite.** JSON has no NaN or infinity, so the writer refuses them, naming the field.
- **A rank-1 column is a plain JSON array.** A tensor of one dimension is its own length, and a
  `{"shape":[n], …}` wrapper adds nothing to read. The tensor type is for rank 2 and up.

### capture-analysis 1.0

- **Derives from** `Analysis`, `QuantizeOffset`, and duet-dsp's `Event`, `HitLane`, `Timing`,
  `Origin<T>`, `BeatTempo` and `KeyEstimate`.
- **Gains `Deserialize`** on `Analysis`, `QuantizeOffset` and `KeyEstimate`, so it can be read back.
  The comparison tool needs this: it reads an analysis.
- **Absent values are omitted, not written `null`:** `key` and `grid` take
  `skip_serializing_if = "Option::is_none"`, as every optional `Song` field does. One rule for every
  document: a missing field is absent. `null` is kept for a slot in a column that has no value, such
  as an unvoiced frame's pitch.
- **`tempo_alternatives` becomes `{"half": …, "double": …}`.** A positional pair says nothing about
  which is which.
- **`scores` stays as `[key, score]` pairs,** and `histogram` as 12 numbers. Both read as written.
- **Cost:** none for old files. `.capture.json` was never read, so no reader breaks.
- **What the performance design adds** (the `reading` section, the between-keys flags, the cost per
  note) arrives as minor versions of this kind.

### performance 1.0, a stream

The performance design's question 1 defines what it holds. It is born in this envelope:

- **Head:** `rate`, `hop`, `first_frame`, and `unit` = `"absolute-cents"`.
- **Record:** a block of frames as columns, `{"start":…,"pitch":[…],"voicing":[…],"level":[…]}`.
  `pitch` holds `null` for an unvoiced frame. Each column is a rank-1 array, and all three have one
  length, which the reader checks.
- **The numbers are written exactly,** in serde_json's shortest form, which reads back to the same
  `f32`. Nothing is rounded to a number of decimals.

### Project, deferred

- **Named** so the kind list is complete. Not designed here: the project work defines what a project
  holds.
- **One constraint now:** `<project>/.duet/` is already the project's config directory, so a project
  document cannot be a file named `.duet` at the root. It can live inside that directory, such as
  `<project>/.duet/project.duet`.

### Lane maps and config are not kinds

- **Lane maps** live in `duet.toml` as `[lanes.*]` tables and in the built-in `gm` map. They are read,
  never written, and do not travel on their own. A lane map becomes a kind when one does.
- **`duet.toml`** is config. It is versioned by duet's own version, and already denies unknown fields.
  A JSON Schema for it would give editors completion (taplo reads JSON Schema for TOML). That is
  deferred: it is a convenience, not a contract.

## 3. The version policy

**Ruled (owner, 2026-11-15).** Adopted as proposed.

- **A reader accepts its own version and every older minor of the same major.** It parses an older
  body with the kind's `read_minor`, which is the current parser while every minor is additive.
- **A reader refuses a newer minor by name:** "groove.duet is drum-grid 1.3; this duet reads drum-grid
  up to 1.2; update duet". It does not read the fields it knows and drop the rest. Writing such a file
  back would lose those fields silently, which breaks the exact round trip.
- **A reader refuses another major by name,** pointing at the converter: "melody.duet is part 2.0;
  this duet reads part 1.x; `duet convert` reads it from a duet that knows 2.0".
- **A writer writes the version it speaks.** An older minor read and written back gets the newer
  version in its envelope, and its body is unchanged, because an additive field is omitted when
  absent.
- **Unknown fields are refused** (`deny_unknown_fields`) at every version, so a body that claims 1.0
  but holds a 1.1 field is refused naming the field.
- **A change to the envelope's shape is a new major version of every kind.** **Ruled (owner,
  2026-11-15).** The envelope has its own committed schema, `envelope/<major>.<minor>.json`, which
  every kind's schema references for its `duet` part, but no version of its own in a file. A reader
  refuses a field it does not know, so a field added to the envelope is one every older reader
  refuses.

**The rule for classifying a change.** The gate decides that a change happened; this rule decides
which number it moves. A pull request that bumps a version states which rule it applies.

- **Minor:**
  - a new optional field, omitted when absent, whose absence means exactly what the older version
    meant;
  - a new variant of a tagged enum;
  - a range or pattern widened, so everything that read before still reads.
- **Major:** everything else. Specifically:
  - a field renamed, removed, or changed in type or unit;
  - an existing value's meaning changed, even with the same type;
  - an optional field made required, or its default changed;
  - a range or pattern narrowed;
  - a tensor's layout changed (dtype, rank, or order).
- **No version change:** a change to a `description` only. The committed schema is regenerated, and
  nothing else moves.
- **A major comes with a converter:** `read_minor` of the new major reads the old body, and
  `duet convert old.duet new.duet` writes it at the new major. Old majors' readers are kept for as
  long as their converter is.

## 4. The schema gate

**Proposed:**

- **Committed under `crates/duet-schemas/schemas/<kind>/<major>.<minor>.json`,** one file per kind and
  version, with examples beside them in `crates/duet-schemas/schemas/<kind>/examples/`. They live in
  the crate so they publish with it. Each is the whole file's schema: envelope, then body (or head and
  record). **Ruled (owner, 2026-11-15).**
- **Generated by `schemars`** from the `Kind` types. `schemars` 1.2.3 is MIT. Its dependencies are
  `schemars_derive` (MIT), `dyn-clone`, `ref-cast`, `ref-cast-impl` and `serde_derive_internals` (each
  MIT OR Apache-2.0). Every licence is in `deny.toml`'s allow list, and all come from crates.io, so
  SC-DENY-1 and SC-DENY-2 pass. It is an ordinary dependency, not a feature, of duet-schemas and of
  each crate that defines a body type, since the derives live with the types. Reading needs no schema,
  but an optional feature is one more thing a tier can leave off.
- **The test:** `crates/duet-schemas/tests/schemas.rs`, run by `cargo test --workspace` in the
  `check` job. For each kind:
  - It generates the schema at the kind's current version.
  - **No committed file for that version:** it fails, naming the file, and says to write it with
    `DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas`.
  - **A committed file whose shape differs** (compared with every `description` stripped): it fails,
    saying the schema of the kind changed at the same version, and to bump the version by
    [the rule](#3-the-version-policy).
  - **A committed file that differs only in descriptions:** it fails, and says to regenerate. No bump
    is needed.
  - **Every older committed version** must have its example document, which must still read
    ([the gate's witnesses](#witnesses)). An older schema file is never regenerated, since the code
    no longer produces it. It is frozen by review, and its example proves the reader still honours it.

### Where things live

**Ruled (owner, 2026-11-15).** A new library crate, **`duet-schemas`**, sits at the top. Its name says what it holds: the
schema definitions themselves, not just a way to read documents.

```text
duet-core
  ← duet-drums (with duet-midi), duet-dsp
  ← duet-capture (with duet-abc, duet-drums, duet-dsp)
  ← duet-schemas (duet-core, duet-drums, duet-dsp, duet-capture)
  ← duet-cli
```

- **`duet-schemas` depends on** duet-core, duet-drums, duet-dsp and duet-capture. duet-cli depends on
  it. It holds:
  - the envelope (`Envelope`, `Tool`, `From`, `statements`), `Version`, the `.duet` and `.duet.jsonl`
    framing, and the version policy's refusals;
  - the `Kind` trait, and its impls for `Song`, `DrumGrid`, `Analysis` and the performance. The trait
    is local to the crate, so these impls are allowed there;
  - the registry and `AnyDocument`, which reads any `.duet` by its kind;
  - the committed schemas and their examples, under `crates/duet-schemas/schemas/`;
  - the gate test, and the `jsonschema` dev-dependency with its validation tests.
- **What stays in the kind crates:**
  - each body type's `#[derive(JsonSchema)]`;
  - the hand-written `JsonSchema` impls for `Position`, `TimeSig`, `Key`, `Grid` and `Pitch`, in
    duet-core. The orphan rule forbids implementing schemars' trait for duet-core's types anywhere
    else;
  - `Tensor<T>`, in duet-core. It is a data type, and `DrumGrid` holds one.
- **The rule:** below `duet-schemas`, no crate reads or writes an enveloped document. The kind crates
  define and serialize their types. Putting a type in an envelope, and taking it out, happens only in
  `duet-schemas`.
  - **Today this moves nothing.** duet-cli already writes capture's files. duet-capture returns a
    `Captured` and writes no file.
  - **Later, a live performance stream** is written by whatever drives it (the CLI, later the MCP
    server), through `duet-schemas`. duet-dsp yields frame blocks and never writes the envelope.

**Its convention status.** This is CI-3 (generated artifacts are drift-checked). Its status in
`.conventions/status.toml` today names the projection's gate only. The build task changes it to:
`gated: .github/workflows/ci.yml#gate (the projected skill) and .github/workflows/ci.yml#check (the
committed schemas, crates/duet-schemas/tests/schemas.rs)`. No new rule is needed.

**Validation against the schema.** **Ruled (owner, 2026-11-15).** Option (a): `jsonschema` as a dev-dependency.

- A derived schema can describe a shape the code never sends. The five hand-serialized string types
  are where that happens here.
- **Options:**
  - (a) add `jsonschema` (MIT) as a **dev-dependency** of `duet-schemas`, with default features off
    (no HTTP resolver, no TLS), and validate every example and every round-trip document against the
    committed schema;
  - (b) no validator: trust the derive and the hand impls, judged in review.
- **Why (a):** it is the test API-JSON-2 asks for in an API, applied to a file format. As
  a dev-dependency it ships in nothing. `cargo deny` checks it like any other dependency when it is
  added.

## 5. Views, rendered and read back

The owner ruled (2026-11-15) that the text forms are **views the CLI produces**, not file
extensions, and that **a view reads back into a document**. This answers how.

### The views

| View | Of kind | Renders | Lossless? |
|---|---|---|---|
| `notes` | `part` | today's notefmt text: the header, then one note or control per line, by track | **Lossless** against part 1.0 |
| `grid` | `drum-grid` | today's `.drums` text: the header, then one row per lane and one column per step | **Lossless** against drum-grid 1.0, and refuses what it cannot show |
| `grid` | `part`, one drum track | the same grid text, from one track of a part at a step | **Lossy**: shows one track's starts and velocities only |

The `grid` view of a part is the owner's case: "dump a rhythm section as a grid view". It is the
view `duet drums read` makes today, without writing a grid document.

### The commands

**Proposed, with the names ruled below:** two top-level commands beside `convert`. Views span kinds, as `convert` does, so
neither belongs in one domain group.

- **`duet view <document> [--as notes|grid] [-o <file>]`** renders a view, to standard output unless
  `-o` names a file.
  - `--as` defaults to the kind's own view: `notes` for a song, `grid` for a drum grid.
  - `--as grid` of a song takes the flags `duet drums read` takes today: `--grid <step>` (required),
    `--track <name>`, `--bars <n>` and `--snap-levels`.
- **`duet apply <view> --onto <document>`** reads an edited view back over the document it was
  rendered from, keeping what the view does not show, and writes the document in place.
- **`duet apply <view> -o <new.duet>`** reads a view into a new document. Only a lossless view can do
  this, since a lossy one has nothing to keep the rest from. A lossy view with `-o` is refused,
  naming `--onto`.
- **Both read standard input for `-`.**
- **`convert` keeps** moving documents to and from the outside formats (MIDI, ABC) and between kinds
  (a song ↔ a drum grid). A view is not a format, so `convert` does not render one.
- **The domain groups keep their verbs over documents:** `duet drums read` makes a drum-grid document
  from a song, `duet drums render` makes a song document from a grid, `duet drums snap` snaps a grid
  document. `duet song show` becomes `duet view` of a song, and is kept as an alias for one minor
  release.

**The command names.** **Ruled (owner, 2026-11-15).** Option (a): `duet view` and `duet apply --onto`.

- **Options:**
  - (a) `duet view` and `duet apply`;
  - (b) `duet view` and `duet view --read`;
  - (c) `duet export --as notes` and `duet import`.
- **Why (a):** two verbs for two directions, and `apply … --onto` says that the document
  keeps what the view does not touch. (c) collides with what `convert` already means for MIDI and ABC.

### The header line

A saved view has no extension of its own, so its first line names it:

```text
# duet notes v1 of part 1.0 from 3f9a1c0d5e2b7a41
# duet grid v1 of drum-grid 1.0 from 9c04e1b2a7d35f60
# duet grid v1 of part 1.0 track "Drums" step 1/16 from 0b7e33c19a5d2f84
```

- **`notes v1`, `grid v1`:** the view and its syntax version. A change of syntax bumps it.
- **`of part 1.0`:** the kind and the schema version the view renders.
- **`track "Drums" step 1/16`:** for a partial view, which part of the document it shows.
- **`from <digest>`:** 16 hex digits of a 64-bit FNV-1a digest of the source document's body, as
  written. `apply --onto` checks it, and refuses when the document has changed since the view was
  rendered, naming both digests: an edit placed over a document it was not made from can land in the
  wrong place. `--force` applies it anyway, and says it did. FNV is ten lines of code, so it adds no
  dependency. It guards against mistakes, and is not a security check.
- **Then the statements,** one `# ` comment line each, from the envelope's `statements`.
- **It is a comment to every other tool.** Its grammar is fixed, and the reader parses it.

**What the reader does with it:**

- **It names a newer view version, or a newer schema version than this duet reads:** refused by name,
  under [the version policy](#3-the-version-policy).
- **It is today's line,** `# notefmt v1: …` or `# notefmt drums v1: …`: read as `notes v1 of part 1.0`
  or `grid v1 of drum-grid 1.0`, with no digest. Every file written before this design still applies.
- **It is missing:** `apply` asks for `--as notes|grid`, and reads the text at the current versions with
  no digest check. A view written from scratch by hand or by an agent has no header, and is read as
  leniently as today (`hand_written_text_is_read_leniently` in both round-trip files). A partial grid
  view with no header also needs `--track` and `--grid` to be placed.

### What each view drops

- **`notes` of a part: nothing in the body.**
  - It shows every field of part 1.0: the conductor, the end, every track with its channel and
    program, every note with its pitch's spelling, every control.
  - It shows the **normalized** body. A loose position (`1.1.960` in 4/4) or a length off the ticks is
    shown normalized. Every reader normalizes, so no document holds the loose form.
  - It always states the end, so a body with no `end` comes back with its default stated. Every writer
    states it anyway, so no document duet writes lacks one.
- **`grid` of a drum grid: nothing in the body.**
  - It shows the conductor, the end, the step, the channel, the levels, the lanes and every cell.
  - It **refuses**, naming the lane, bar and step, a velocity that is not 0 or one of the three levels.
    `--snap-levels` renders it snapped instead, and says so in a statement. It never snaps unasked
    (`off_level_velocities_are_refused_as_text_and_snapped_only_when_asked`).
- **`grid` of a song's drum track: everything but that track's starts and velocities.** Exactly:
  - every other track;
  - the drum track's note lengths, its controls and its program;
  - velocities off the levels, which it refuses unless `--snap-levels`.
  - It refuses, as `duet drums read` does today, a note off the step, a key no lane maps, two notes on
    one lane and step, or a note past `--bars`.
- **Every view drops the envelope's `tool` and `from`,** and shows the `statements` read-only.
- **Comments added to a view are not read back.** Only the header line is parsed. A note an agent
  leaves in a comment is lost on `apply`. This is said in each view's module docs.

### Reading back over the source

**Proposed: merge cell by cell, against the source document, with no ids.**

- **A lossless view onto its source** replaces the body with what the view reads, and keeps the
  source's envelope (`from` and `statements`).
- **A grid view onto a song** changes only the drum track it names, inside the bars it shows:
  - **A cell whose symbol is unchanged keeps the source note exactly,** with its length and its
    velocity. That includes a velocity off the levels shown snapped.
  - **A changed symbol** sets that note's velocity to the symbol's level, and keeps its length.
  - **A new hit** becomes a note one step long at that level, as `duet drums render` makes one today.
  - **An emptied cell** removes the note.
  - **Every other track, and the drum track's controls and program, are untouched.**
- **What cannot be placed is refused, naming it:**
  - a conductor line (tempo, time, key, end, `tpq`) that differs from the source's. In a partial view
    the conductor is shown read-only: changing the meter there would move every other track. Change
    it with the `notes` view, which shows the whole song.
  - a lane or bar added or removed in a partial view;
  - a source whose digest differs from the header's, unless `--force`.
- **Why cells and not ids:** a grid cell is already an address (a lane's key at a step), so a cell
  compared with the source's cell at the same address places every edit. Ids would put numbers into
  the view that a person must not touch, which is what the view exists to avoid.

### Witnesses for the views

These replace or extend the `.drums` and notefmt round trips in
`crates/duet-drums/tests/roundtrip.rs` and `crates/duet-midi/tests/roundtrip.rs`:

- **Lossless is the identity.** document → view → document is equal, for the `notes` view over every
  song the MIDI round-trip file builds, and for the `grid` view over the random grids of
  `the_drum_grid_round_trips_under_its_map`. view → document → view is byte-identical.
- **Silent means unchanged.** A song with three tracks, controls on its drum track, note lengths other
  than one step and a velocity off the levels: render the `grid` view of its drum track, apply it back
  unedited, and the document is equal to the source.
- **An edited cell lands where it was edited.** Edit one cell of that view (a `.` to an `x`, an `x` to
  a `.`, an `x` to an `X`), apply it, and exactly one note differs from the source, at that lane's key
  and that step, with the expected velocity. Each of the three edits is a case.
- **What cannot be placed is refused.** A changed tempo line, an added lane, and a source changed since
  rendering are each refused by name, and the document is unchanged.
- **The header is checked.** A view naming a newer view or schema version is refused by name; today's
  `# notefmt` lines and a missing header both still apply.

## 6. Outside formats

**Proposed:**

- **ABC and MIDI import into a part document** and export from one. Their readers and writers are
  unchanged. They produce and take a `Song`, and the document wraps it.
- **What `duet-abc` did not carry goes in the envelope's `from`.**

  ```json
  "from": {
    "format": "abc",
    "tune": "1",
    "notices": [ { "line": 4, "col": 12, "construct": "slur", "found": "(" } ]
  }
  ```

  - **Notices** are what the ABC reader read but did not carry: notation-only constructs such as
    slurs and annotations (`duet_abc::Notice`). Today they go to stderr and are lost.
  - **Refusals** never reach a document. A refused construct fails the read by name, so there is no
    song to put them on. If a later reader approximates instead of refusing, its approximations join
    `from` as a new optional field, which is a minor version.
  - **MIDI** gives `{"format":"midi"}`. It drops nothing duet's model holds that a notice would name.
    The MIDI round-trip tests are the proof.
- **Why the envelope, not the body:** where a song came from is a fact about this file, not about the
  music. Two documents with the same body are the same song. It is defined in the envelope now, so
  adding it costs no kind a version.
- **`from` is kept on write-back** of a `.duet` that has it, and dropped by every view (question 5).

## 7. Today's names and readers

The owner ruled (2026-11-15) that `.duet` and `.duet.jsonl` are the only extensions. **Proposed:**

| Today | Becomes |
|---|---|
| `<x>.notes` (and `.txt`, read as notefmt) | a part document, `<x>.duet`. Its text is the `notes` view. |
| `<x>.drums` | a drum-grid document, `<x>.duet`. Its text is the `grid` view. |
| `<x>.json`, `<x>.toml` (a song) | a part document, `<x>.duet` |
| `<x>.drums.json`, `<x>.drums.toml` | a drum-grid document, `<x>.duet` |
| capture's `<stem>.notes` (the unquantized performance) | `<stem>.duet`, a part document |
| capture's `<stem>.drums` (a rhythm take's score) | `<stem>.grid.duet`, a drum-grid document |
| capture's `<stem>.abc` (a melody take's score) | unchanged: ABC is an outside format |
| `<stem>.capture.json` | `<stem>.capture.duet`, a `capture-analysis` document |
| `<stem>.performance.jsonl` (designed, not yet written) | `<stem>.performance.duet.jsonl`, a `performance` stream |
| `.mid`, `.abc` | unchanged |

- **The dispatch in `duet-cli/src/io.rs`:**
  - **`.duet` and `.duet.jsonl` are read by their envelope.** The kind decides what the file is, so a
    part command given a drum-grid document refuses it, naming the kind: "beat.duet is a drum-grid;
    duet play reads a part; duet drums render makes one". The message the binary prints today still
    says "song", until the rename's code change.
  - **`.mid`, `.midi`, `.smf` and `.abc` keep their names.** They are outside formats with no envelope.
  - **A view is never read by its name.** Only `duet apply` reads one, by its header line.
  - **Standard input (`-`):** a first non-blank byte of `{` is a document, read by its envelope.
    Anything else is the command's default outside format, as today.
  - **Names are conventions.** `<stem>.grid.duet` is a reminder to people, and the reader never
    relies on it.
- **The rule this replaces:** "a file name means the same to every command" becomes "a file's
  envelope means the same to every command".
- **Retiring the old names:**
  - **`.notes` and `.drums` files need no converter.** They are views, so `duet apply old.notes -o
    new.duet` reads one, by its `# notefmt` line or by `--as`. That is what `apply` does anyway.
  - **`.json`, `.toml`, `.drums.json` and `.drums.toml` are cut now.** **Ruled (owner, 2026-11-15).** No such file on
    the owner's machine matters, so there is no legacy reader and no converter. Nothing reads or
    writes these names, and the dispatch drops them with `.notes` and `.drums`. A file with one of these
    names is refused, saying it is not a duet document.
  - **What the cut touches in the tests.** The CLI tests in `crates/duet-e2e` that use these names
    (`names.rs`, `convert_drums.rs`, `cli.rs`, `grid_copy.rs`, `play.rs`) move to `.duet` and views.
    The JSON and TOML legs of the round-trip tests in `crates/duet-midi/tests/roundtrip.rs` and
    `crates/duet-drums/tests/roundtrip.rs` round-trip the types, not a file: the JSON legs become the
    document round trips, and the TOML legs go, since no document is TOML. Each change is a test of
    the round-trip constraint changing by this ruling, named in its pull request.

## 8. TOML

**Ruled (owner, 2026-11-15).** TOML for config only.

- **`duet.toml`, `duet.local.toml` and lane-map tables stay TOML.** They are written by hand, and
  TOML is good at that.
- **No document is TOML.** TOML has no `null`, and the performance's pitch column needs one for an
  unvoiced frame. A container that holds some kinds and not others would make the kind decide the
  container, which is what the envelope is for.
- **A song or grid in TOML** is no longer read at all ([question 7](#7-todays-names-and-readers)).

## Witnesses

Every round-trip test in `crates/duet-midi/tests/roundtrip.rs` and `crates/duet-drums/tests/roundtrip.rs`
keeps passing unchanged. These are the new ones, each named for the task that adds it.

- **The envelope and the policy** (`duet-schemas` unit tests):
  - an envelope round-trips exactly;
  - a newer minor is refused, naming the file's version and the newest this reader knows;
  - another major is refused, naming the converter;
  - an unknown field in a body is refused, naming the field;
  - a `$schema` that disagrees with `kind` and `version` is refused;
  - a tensor whose `data` length is not the product of its `shape`, or whose `dtype` is wrong, is
    refused naming both.
- **Exact floats:** random `f64` and `f32` bit patterns (finite) write and read back to the same bits
  through serde_json, with `float_roundtrip` on. It fails without it.
- **part 1.0, built as song 1.0** (in `crates/duet-midi/tests/roundtrip.rs`, as CLAUDE.md requires):
  - every song the file already builds round-trips through a part document exactly;
  - a part document's body is byte-identical to what `.json` wrote before this change;
  - the `notes` view witnesses of [question 5](#witnesses-for-the-views).
- **drum-grid 1.0** (in `crates/duet-drums/tests/roundtrip.rs`):
  - the random grids of `the_drum_grid_round_trips_under_its_map` round-trip through a document
    exactly;
  - the `grid` view witnesses of [question 5](#witnesses-for-the-views), including the partial view over a song;
- **The gate** (`crates/duet-schemas/tests/schemas.rs`):
  - its compare function, given a schema with a field added and the same version, fails and names
    the kind;
  - given a description-only change, it asks to regenerate and asks for no bump;
  - every committed example document (`crates/duet-schemas/schemas/<kind>/examples/<version>.duet`) reads with the current
    reader, and validates against its version's schema with `jsonschema`.
- **Outside formats:** an ABC tune with a slur converts to a `.duet` whose `from` lists that notice at
  its line and column; converting that `.duet` back to `.abc` and in again gives the same body.
- **The CLI** (in `crates/duet-e2e`):
  - `duet apply old.notes -o x.duet`, then `duet view x.duet`, gives text identical to `old.notes`
    under a new header line;
  - `duet view` of a song `--as grid`, edited by one cell, then `duet apply --onto`, changes that one
    note in the file;
  - `apply --onto` a document changed since the view was rendered is refused, and the file is
    untouched;
  - a drum-grid `.duet` given to `duet play` is refused naming its kind;
  - a document on standard input is read by its envelope.

## Build tasks this design names

The owner has ruled, so each can be filed. Each names its witness, and none is built before its
predecessor unless it says so.

1. **The document core.** The `duet-schemas` crate, with `Envelope`, `Version`, `Tool`, `From`,
   `statements`, the `Kind` trait, `.duet` and `.duet.jsonl` framing, and the version policy's
   errors. `serde_json`'s `float_roundtrip` on. Witnesses: the envelope and the policy, and exact
   floats, as `duet-schemas` unit tests.
2. **The schema gate, with song 1.0** (built; renamed `part` 1.0 by the songs design's task 1). `schemars` added; hand `JsonSchema` impls with patterns for
   `Position`, `TimeSig`, `Key`, `Grid` and `Pitch`; ranges stated; `deny_unknown_fields` on the song
   types; the `Kind` impl for `Song` and the registry in `duet-schemas`;
   `crates/duet-schemas/schemas/song/1.0.json` and its example committed; the gate test; CI-3's status updated. Witnesses: the gate, and song 1.0 in
   `crates/duet-midi/tests/roundtrip.rs`.
3. **The CLI reads and writes documents.** `io.rs` dispatches `.duet` by envelope, refuses a wrong
   kind by name, sniffs standard input, and writes `.duet`. Nothing reads or writes `.notes` or
   `.drums` by name any more, and the `.json`, `.toml`, `.drums.json` and `.drums.toml` readers and
   writers are removed, with the e2e tests and the round-trip TOML legs that use them. Witnesses: the kind, standard
   input and `duet play` tests in `crates/duet-e2e`.
4. **drum-grid 1.0.** `Tensor<T>` in duet-core, `bar_steps`, `Bar.bar` gone, the `Kind` impl in
   `duet-schemas`, and `crates/duet-schemas/schemas/drum-grid/1.0.json` with its example committed.
   Witnesses: drum-grid 1.0 in `crates/duet-drums/tests/roundtrip.rs`.
5. **Views, rendered and read back.** `duet view` and `duet apply --onto | -o`, the header
   line with its digest, the cell-by-cell merge onto a source, the envelope's `statements` rendered
   as comments, and what each view drops stated in module docs and README. Needs 4 for the grid view of
   a grid. Witnesses: [question 5's](#witnesses-for-the-views), and the `view` and `apply` tests in
   `crates/duet-e2e`.
6. **Outside formats record where a song came from.** `from` filled by ABC and MIDI import, kept on
   write-back. Witness: the ABC slur test.
7. **capture's outputs become documents.** `capture-analysis` 1.0: `Deserialize` on the analysis
   types, absent values omitted, `tempo_alternatives` named, the `Kind` impl in `duet-schemas`, and its
   schema committed. duet-cli's capture command writes, through `duet-schemas`, `<stem>.duet`, `<stem>.grid.duet` or `<stem>.abc`, and `<stem>.capture.duet`. What it inferred
   moves from `#` comments into `statements`. Witnesses: an analysis document round-trips exactly
   through its reader and validates against its schema; a capture with an estimated tempo has that
   statement in its part document, and `duet view` shows it as a comment.
8. **performance 1.0.** The performance design's task 2. duet-dsp yields `PerformanceHead` and
   `FrameBlock`, and duet-cli writes `<stem>.performance.duet.jsonl` through `duet-schemas`.
   Witnesses: that design's question 1, plus: the first line is the envelope, and every later line
   validates against the record schema on its own.

Tasks 4, 6 and 7 need 3 and can be built in any order. Task 5 needs 3 and 4. Task 8 needs only 1 and 2, so the held performance
task can start before the CLI work lands.

## Amendments to the other designs

Each gets a pointer where its file names change, and nothing else:

- **[The capture design](2026-11-14-capture-design.md):** `<stem>.notes` becomes `<stem>.duet`, a
  part document (written as a song document before the rename). `<stem>.drums` becomes `<stem>.grid.duet`, a drum-grid document. `<stem>.capture.json`
  becomes `<stem>.capture.duet`, a `capture-analysis` document. The `#` comment statements move to the
  envelope's `statements`, and every view shows them.
- **[The performance and reading design](2026-11-15-performance-and-reading-design.md):**
  `<stem>.performance.jsonl` becomes `<stem>.performance.duet.jsonl`, a `performance` stream. Its
  header is the envelope's `head`, and its lines are records.

## The owner's rulings

All seven are **Ruled (owner, 2026-11-15)**:

1. **The schemas' `$id`s** are raw GitHub URLs in this repository:
   `https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/<kind>/<major>.<minor>.json`.
2. **The version policy** is adopted as proposed. Readers accept older minors. They refuse a newer
   minor, another major and unknown fields, each by name. Writers write the version they speak.
3. **The drum grid** holds one lanes × steps tensor for the whole grid, with `bar_steps`.
4. **Today's `.json`, `.toml`, `.drums.json` and `.drums.toml` are cut now,** with no legacy reader.
   Old `.notes` and `.drums` text still reads through `duet apply`, as views named by their header
   line.
5. **The view commands** are `duet view` and `duet apply --onto` (and `-o` for a lossless view).
6. **TOML** is for config only.
7. **`jsonschema`** is a dev-dependency, used to validate documents against their committed schemas.

Also **Ruled (owner, 2026-11-15)**: the crate is **`duet-schemas`**, at the top of the graph, holding
the envelope, the `Kind` trait and its impls, the registry, the committed schemas and the gate. Each
body type's derive, the five hand-written impls and `Tensor` stay below it. No crate below it reads or
writes an enveloped document ([where things live](#where-things-live)).

Still **Proposed**:
- the view header's FNV digest of its source, and `--force` to apply over a changed source;
- merging a view onto its source cell by cell rather than by ids;
- `capture-analysis` omitting absent values rather than writing `null`;
- `tempo_alternatives` named `{half, double}`;
- the envelope's `statements`.

## Not decided here, and why

- **The project kind:** named, and constrained to live inside `.duet/`. What it holds is
  [the project design](2026-11-17-project-design.md)'s.
- **The song kind, for the piece:** [songs and composition](2026-11-17-songs-and-composition-design.md)'s.
- **A schema for `duet.toml`:** an editor convenience, not a contract. Deferred until someone wants it.
- **Seconds beside frames in capture events:** the capture design promised them, and the file has frames
  and the rate. Whether to add them is a minor of `capture-analysis`, decided when the comparison tool
  needs it.
