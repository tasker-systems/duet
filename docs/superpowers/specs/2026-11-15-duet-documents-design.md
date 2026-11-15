# duet documents: design

This designs how [the decision that duet's files are projections of versioned data
structures](#the-decision-this-builds) is built. That decision is settled and is not reopened here.
It answers the eight questions of the design task, names the build tasks in order, and holds the
performance layer's file until its envelope exists.

It takes in the two amendments the owner made to the decision on 2026-11-15, while this design was
being written: `.notes` and `.drums` are no longer file extensions, and a view reads back into a
document as a working surface. Questions 5 and 7 answer to them.

Every answer below is **Proposed**. Each open question is marked **For the owner to rule**, with a
recommendation. All of them are gathered in [For the owner to rule](#for-the-owner-to-rule).

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

- **`Song` can become `song` 1.0 unchanged.** The body of a song document is byte for byte what
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
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/schemas/song/1.0.json",
  "duet": {
    "kind": "song",
    "version": "1.0",
    "tool": { "name": "duet", "version": "0.3.0" }
  },
  "body": { "ticks_per_quarter": 480, "tempo_bpm": 92.0, "time_sig": "4/4", "tracks": [] }
}
```

- **`duet.kind`:** the kind's name, lowercase with hyphens: `song`, `drum-grid`, `capture-analysis`,
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

- **The body is the type, unchanged.** `song` 1.0's body is today's `Song` JSON, with no added keys.
  Top-level keys would put `duet` beside `tracks`, so the body would no longer be the type.
- **Unknown fields can be refused.** Top-level keys would need `#[serde(flatten)]`, and serde cannot
  combine `flatten` with `deny_unknown_fields`. The version policy needs the latter.
- **The reader dispatches before it parses the body.** It reads `duet`, picks the kind, and only then
  parses `body` as that kind's type. serde writes struct fields in order, so `duet` is always before
  `body` in a file duet wrote.

**Where the `$id`s live. For the owner to rule.**

- **Options:**
  - (a) a URL in this repository: `https://raw.githubusercontent.com/tasker-systems/duet/main/schemas/<kind>/<major>.<minor>.json`;
  - (b) a URL on a domain the owner holds;
  - (c) a repository path with no URL, such as `duet:schemas/song/1.0.json`.
- **Recommendation: (a).** It resolves today with nothing to host, and editors can fetch it. A
  committed schema file is never edited once its version ships ([question 4](#4-the-schema-gate)),
  so the `main` URL is stable. Moving to (b) later is a change to the `$schema` value only, which a
  reader can accept both ways.

**A stream's lines.** A `.duet.jsonl` file holds one JSON value per line.

- **The first line** is the envelope with a `head` in place of `body`: the stream's own header, such
  as the performance's rate, hop, first frame and unit.

  ```json
  {"$schema":"…/schemas/performance/1.0.json","duet":{"kind":"performance","version":"1.0","tool":{"name":"duet","version":"0.3.0"}},"head":{"rate":22050,"hop":256,"first_frame":0,"unit":"absolute-cents"}}
  ```
- **Every later line** is one record of the kind, and nothing else. It validates against the kind's
  record schema (`$defs/Record` of the same file) on its own, without the envelope or any other line.
  So a stream can be appended to, cut at any line, or read as it is written.
- **A stream kind has two parts in its schema:** `Head` and `Record`.

## 2. The kinds, and their v1 schemas

Each kind is a Rust type that serializes as its body, and implements one trait from `duet-core`:

```rust
pub trait Kind: Serialize + DeserializeOwned + JsonSchema {
    const NAME: &'static str;        // "song"
    const VERSION: Version;          // 1.0
    /// Reads a body written at an older minor of this major. The default parses it as the
    /// current type, which is right while every minor is additive.
    fn read_minor(minor: u16, body: serde_json::Value) -> Result<Self>;
}
```

| Kind | v1 | Body type | Change from today |
|---|---|---|---|
| `song` | 1.0 | `duet_core::Song` | **None on write.** Read gains `deny_unknown_fields`. |
| `drum-grid` | 1.0 | `duet_drums::DrumGrid` | The velocity matrix becomes one tensor. `Bar.bar` goes. |
| `capture-analysis` | 1.0 | `duet_capture::Analysis` | Gains a reader. Absent values are omitted, not `null`. `tempo_alternatives` is named. |
| `performance` | 1.0, a stream | new `duet_dsp` types, `PerformanceHead` and `FrameBlock` | New: the performance design's question 1, in this envelope. |
| `project` | named, deferred | none yet | Waits for the project work. |
| lane map | **not a kind** | | Config. |
| `duet.toml` | **not a kind** | | Config. |

### song 1.0

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
- **A written song always states its end.** Every writer already does this (`io::with_end`), so a song
  document's `end` is always present, though the type keeps it optional.

### drum-grid 1.0

Today: `bars: [{ "bar": 1, "steps": 16, "vel": [[…per step…], …per lane…] }, …]`.

**Proposed v1:**

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

**For the owner to rule: how the drum matrix is held.**

- **Options:**
  - (a) one lanes × steps tensor for the grid, with `bar_steps`;
  - (b) one lanes × steps tensor per bar;
  - (c) keep today's nested arrays, and call a rectangular nested array a tensor.
- **Recommendation: (a).** It is what the decision describes: a drum grid is a lanes × steps
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

**Proposed: adopt the decision's proposal.**

- **A reader accepts its own version and every older minor of the same major.** It parses an older
  body with the kind's `read_minor`, which is the current parser while every minor is additive.
- **A reader refuses a newer minor by name:** "groove.duet is drum-grid 1.3; this duet reads drum-grid
  up to 1.2; update duet". It does not read the fields it knows and drop the rest. Writing such a file
  back would lose those fields silently, which breaks the exact round trip.
- **A reader refuses another major by name,** pointing at the converter: "song.duet is song 2.0; this
  duet reads song 1.x; `duet convert` reads it from a duet that knows 2.0".
- **A writer writes the version it speaks.** An older minor read and written back gets the newer
  version in its envelope, and its body is unchanged, because an additive field is omitted when
  absent.
- **Unknown fields are refused** (`deny_unknown_fields`) at every version, so a body that claims 1.0
  but holds a 1.1 field is refused naming the field.

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

- **Committed under `schemas/<kind>/<major>.<minor>.json`** at the repository root, one file per kind
  and version. Each is the whole file's schema: envelope, then body (or head and record).
- **Generated by `schemars`** from the `Kind` types. `schemars` 1.2.3 is MIT. Its dependencies are
  `schemars_derive` (MIT), `dyn-clone`, `ref-cast`, `ref-cast-impl` and `serde_derive_internals` (each
  MIT OR Apache-2.0). Every licence is in `deny.toml`'s allow list, and all come from crates.io, so
  SC-DENY-1 and SC-DENY-2 pass. It is an ordinary dependency of each crate that defines a kind, not a
  feature: reading needs no schema, but an optional feature is one more thing a tier can leave off.
- **The test:** `crates/duet-documents/tests/schemas.rs`, run by `cargo test --workspace` in the
  `check` job. For each kind:
  - It generates the schema at the kind's current version.
  - **No committed file for that version:** it fails, naming the file, and says to write it with
    `DUET_BLESS_SCHEMAS=1 cargo test -p duet-documents`.
  - **A committed file whose shape differs** (compared with every `description` stripped): it fails,
    saying the schema of the kind changed at the same version, and to bump the version by
    [the rule](#3-the-version-policy).
  - **A committed file that differs only in descriptions:** it fails, and says to regenerate. No bump
    is needed.
  - **Every older committed version** must have its example document, which must still read
    ([the gate's witnesses](#witnesses)). An older schema file is never regenerated, since the code
    no longer produces it. It is frozen by review, and its example proves the reader still honours it.
- **Where the registry lives:** a new library crate, `duet-documents`. It depends on duet-core,
  duet-drums, duet-dsp and duet-capture, lists every kind, and reads any `.duet` into an `AnyDocument`
  by its kind. duet-cli's `io.rs` calls it. The generic parts (`Envelope`, `Version`, `Tensor`, the
  `Kind` trait, the version checks) live in `duet_core::doc`, since every crate that defines a kind
  already depends on duet-core.

**Its convention status.** This is CI-3 (generated artifacts are drift-checked). Its status in
`.conventions/status.toml` today names the projection's gate only. The build task changes it to:
`gated: .github/workflows/ci.yml#gate (the projected skill) and .github/workflows/ci.yml#check (the
committed schemas, crates/duet-documents/tests/schemas.rs)`. No new rule is needed.

**Validation against the schema. For the owner to rule.**

- A derived schema can describe a shape the code never sends. The five hand-serialized string types
  are where that happens here.
- **Options:**
  - (a) add `jsonschema` (MIT) as a **dev-dependency** of `duet-documents`, with default features off
    (no HTTP resolver, no TLS), and validate every example and every round-trip document against the
    committed schema;
  - (b) no validator: trust the derive and the hand impls, judged in review.
- **Recommendation: (a).** It is the test API-JSON-2 asks for in an API, applied to a file format. As
  a dev-dependency it ships in nothing. `cargo deny` checks it like any other dependency when it is
  added.

## 5. Views, rendered and read back

The owner ruled (2026-11-15) that the text forms are **views the CLI produces**, not file
extensions, and that **a view reads back into a document**. This answers how.

### The views

| View | Of kind | Renders | Lossless? |
|---|---|---|---|
| `notes` | `song` | today's notefmt text: the header, then one note or control per line, by track | **Lossless** against song 1.0 |
| `grid` | `drum-grid` | today's `.drums` text: the header, then one row per lane and one column per step | **Lossless** against drum-grid 1.0, and refuses what it cannot show |
| `grid` | `song`, one drum track | the same grid text, from one track of a song at a step | **Lossy**: shows one track's starts and velocities only |

The `grid` view of a song is the owner's case: "dump a rhythm section as a grid view". It is the
view `duet drums read` makes today, without writing a grid document.

### The commands

**Proposed:** two top-level commands beside `convert`. Views span kinds, as `convert` does, so
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

**For the owner to rule: the command names.**

- **Options:**
  - (a) `duet view` and `duet apply`;
  - (b) `duet view` and `duet view --read`;
  - (c) `duet export --as notes` and `duet import`.
- **Recommendation: (a).** Two verbs for two directions, and `apply … --onto` says that the document
  keeps what the view does not touch. (c) collides with what `convert` already means for MIDI and ABC.

### The header line

A saved view has no extension of its own, so its first line names it:

```text
# duet notes v1 of song 1.0 from 3f9a1c0d5e2b7a41
# duet grid v1 of drum-grid 1.0 from 9c04e1b2a7d35f60
# duet grid v1 of song 1.0 track "Drums" step 1/16 from 0b7e33c19a5d2f84
```

- **`notes v1`, `grid v1`:** the view and its syntax version. A change of syntax bumps it.
- **`of song 1.0`:** the kind and the schema version the view renders.
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
- **It is today's line,** `# notefmt v1: …` or `# notefmt drums v1: …`: read as `notes v1 of song 1.0`
  or `grid v1 of drum-grid 1.0`, with no digest. Every file written before this design still applies.
- **It is missing:** `apply` asks for `--as notes|grid`, and reads the text at the current versions with
  no digest check. A view written from scratch by hand or by an agent has no header, and is read as
  leniently as today (`hand_written_text_is_read_leniently` in both round-trip files). A partial grid
  view with no header also needs `--track` and `--grid` to be placed.

### What each view drops

- **`notes` of a song: nothing in the body.**
  - It shows every field of song 1.0: the conductor, the end, every track with its channel and
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

- **ABC and MIDI import into a song document** and export from one. Their readers and writers are
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
| `<x>.notes` (and `.txt`, read as notefmt) | a song document, `<x>.duet`. Its text is the `notes` view. |
| `<x>.drums` | a drum-grid document, `<x>.duet`. Its text is the `grid` view. |
| `<x>.json`, `<x>.toml` (a song) | a song document, `<x>.duet` |
| `<x>.drums.json`, `<x>.drums.toml` | a drum-grid document, `<x>.duet` |
| capture's `<stem>.notes` (the unquantized performance) | `<stem>.duet`, a song document |
| capture's `<stem>.drums` (a rhythm take's score) | `<stem>.grid.duet`, a drum-grid document |
| capture's `<stem>.abc` (a melody take's score) | unchanged: ABC is an outside format |
| `<stem>.capture.json` | `<stem>.capture.duet`, a `capture-analysis` document |
| `<stem>.performance.jsonl` (designed, not yet written) | `<stem>.performance.duet.jsonl`, a `performance` stream |
| `.mid`, `.abc` | unchanged |

- **The dispatch in `duet-cli/src/io.rs`:**
  - **`.duet` and `.duet.jsonl` are read by their envelope.** The kind decides what the file is, so a
    song command given a drum-grid document refuses it, naming the kind: "beat.duet is a drum-grid;
    duet play reads a song; duet drums render makes one".
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
  - **`.json` and `.toml` song files** already hold a song 1.0 body, and `.drums.json` and
    `.drums.toml` the old grid shape. These are the only names that need a reader of their own.

**For the owner to rule: the old JSON and TOML files.**

- **Options:**
  - (a) `duet convert` keeps **reading** `.json`, `.toml`, `.drums.json` and `.drums.toml` as legacy
    inputs for one minor release, with a stderr note naming `duet convert <file> <x>.duet`. Nothing
    writes them. The readers are removed after that release;
  - (b) cut them now, readers and writers both.
- **Recommendation: (a)**, or (b) if no such file on the owner's machine matters. A bare song `.json`
  already is a song 1.0 body, so reading it costs one branch. The old grid shape needs one conversion.
- **The question only the owner can answer:** are there `.json`, `.toml` or `.drums.*` files on the
  owner's machine that matter?

## 8. TOML

**Proposed: TOML for config only.**

- **`duet.toml`, `duet.local.toml` and lane-map tables stay TOML.** They are written by hand, and
  TOML is good at that.
- **No document is TOML.** TOML has no `null`, and the performance's pitch column needs one for an
  unvoiced frame. A container that holds some kinds and not others would make the kind decide the
  container, which is what the envelope is for.
- **A song or grid in TOML** is reachable only as a legacy input ([question 7](#7-todays-names-and-readers)).

## Witnesses

Every round-trip test in `crates/duet-midi/tests/roundtrip.rs` and `crates/duet-drums/tests/roundtrip.rs`
keeps passing unchanged. These are the new ones, each named for the task that adds it.

- **The envelope and the policy** (`duet_core::doc` unit tests):
  - an envelope round-trips exactly;
  - a newer minor is refused, naming the file's version and the newest this reader knows;
  - another major is refused, naming the converter;
  - an unknown field in a body is refused, naming the field;
  - a `$schema` that disagrees with `kind` and `version` is refused;
  - a tensor whose `data` length is not the product of its `shape`, or whose `dtype` is wrong, is
    refused naming both.
- **Exact floats:** random `f64` and `f32` bit patterns (finite) write and read back to the same bits
  through serde_json, with `float_roundtrip` on. It fails without it.
- **song 1.0** (in `crates/duet-midi/tests/roundtrip.rs`, as CLAUDE.md requires):
  - every song the file already builds round-trips through a song document exactly;
  - a song document's body is byte-identical to what `.json` wrote before this change;
  - the `notes` view witnesses of [question 5](#witnesses-for-the-views).
- **drum-grid 1.0** (in `crates/duet-drums/tests/roundtrip.rs`):
  - the random grids of `the_drum_grid_round_trips_under_its_map` round-trip through a document
    exactly;
  - the `grid` view witnesses of [question 5](#witnesses-for-the-views), including the partial view over a song;
  - a grid in today's `.drums.json` shape reads to the same grid as its document (if the legacy reader
    is kept).
- **The gate** (`crates/duet-documents/tests/schemas.rs`):
  - its compare function, given a schema with a field added and the same version, fails and names
    the kind;
  - given a description-only change, it asks to regenerate and asks for no bump;
  - every committed example document (`schemas/<kind>/examples/<version>.duet`) reads with the current
    reader, and validates against its version's schema (if the validator is ruled in).
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

Each is filed once the owner has ruled. Each names its witness, and none is built before its
predecessor unless it says so.

1. **The document core.** `duet_core::doc`: `Envelope`, `Version`, `Tool`, `From`, `Tensor<T>`, the
   `Kind` trait, `.duet` and `.duet.jsonl` framing, and the version policy's errors. `serde_json`'s
   `float_roundtrip` on. Witnesses: the envelope and the policy, and exact floats.
2. **The schema gate, with song 1.0.** `schemars` added; hand `JsonSchema` impls with patterns for
   `Position`, `TimeSig`, `Key`, `Grid` and `Pitch`; ranges stated; `deny_unknown_fields` on the song
   types; the `duet-documents` crate with its registry; `schemas/song/1.0.json` and its example
   committed; the gate test; CI-3's status updated. Witnesses: the gate, and song 1.0 in
   `crates/duet-midi/tests/roundtrip.rs`.
3. **The CLI reads and writes documents.** `io.rs` dispatches `.duet` by envelope, refuses a wrong
   kind by name, sniffs standard input, and writes `.duet`. Nothing reads or writes `.notes` or
   `.drums` by name any more. The old JSON and TOML names as ruled. Witnesses: the kind, standard
   input and `duet play` tests in `crates/duet-e2e`.
4. **drum-grid 1.0.** The tensor, `bar_steps`, `Bar.bar` gone, `schemas/drum-grid/1.0.json`, and the
   legacy grid reader if ruled. Witnesses: drum-grid 1.0 in `crates/duet-drums/tests/roundtrip.rs`.
5. **Views, rendered and read back.** `duet view` and `duet apply` (or the names ruled), the header
   line with its digest, the cell-by-cell merge onto a source, the envelope's `statements` rendered
   as comments, and what each view drops stated in module docs and README. Needs 4 for the grid view of
   a grid. Witnesses: [question 5's](#witnesses-for-the-views), and the `view` and `apply` tests in
   `crates/duet-e2e`.
6. **Outside formats record where a song came from.** `from` filled by ABC and MIDI import, kept on
   write-back. Witness: the ABC slur test.
7. **capture's outputs become documents.** `capture-analysis` 1.0: `Deserialize` on the analysis
   types, absent values omitted, `tempo_alternatives` named, and its schema committed. Capture writes
   `<stem>.duet`, `<stem>.grid.duet` or `<stem>.abc`, and `<stem>.capture.duet`. What it inferred
   moves from `#` comments into `statements`. Witnesses: an analysis document round-trips exactly
   through its reader and validates against its schema; a capture with an estimated tempo has that
   statement in its song document, and `duet view` shows it as a comment.
8. **performance 1.0.** The performance design's task 2, written as `<stem>.performance.duet.jsonl`.
   Witnesses: that design's question 1, plus: the first line is the envelope, and every later line
   validates against the record schema on its own.

Tasks 4, 6 and 7 need 3 and can be built in any order. Task 5 needs 3 and 4. Task 8 needs only 1 and 2, so the held performance
task can start before the CLI work lands.

## Amendments to the other designs

Each gets a pointer where its file names change, and nothing else:

- **[The capture design](2026-11-14-capture-design.md):** `<stem>.notes` becomes `<stem>.duet`, a
  song document. `<stem>.drums` becomes `<stem>.grid.duet`, a drum-grid document. `<stem>.capture.json`
  becomes `<stem>.capture.duet`, a `capture-analysis` document. The `#` comment statements move to the
  envelope's `statements`, and every view shows them.
- **[The performance and reading design](2026-11-15-performance-and-reading-design.md):**
  `<stem>.performance.jsonl` becomes `<stem>.performance.duet.jsonl`, a `performance` stream. Its
  header is the envelope's `head`, and its lines are records.

## For the owner to rule

1. **Where the schemas' `$id`s live.** (a) a raw GitHub URL in this repository, (b) a URL on a domain
   the owner holds, or (c) a repository path with no URL. **Recommended: (a)**, since it resolves today
   with nothing to host and editors can fetch it.
2. **The version policy.** Readers accept older minors and refuse a newer minor or another major by
   name; unknown fields are refused; writers write the version they speak. Or, readers ignore unknown
   fields. **Recommended: refuse**, since ignoring them loses data on write-back.
3. **How the drum matrix is held.** (a) one lanes × steps tensor for the whole grid with each bar's step
   count, (b) a tensor per bar, or (c) today's nested arrays. **Recommended: (a)**, the decision's
   shape; (c) is free but is not a declared tensor.
4. **The old `.json`, `.toml`, `.drums.json` and `.drums.toml` files.** (a) `duet convert` reads them
   as legacy inputs for one minor release, and nothing writes them; (b) cut them now.
   **Recommended: (a)**, or (b) if no such file on the owner's machine matters. Are there any? Old
   `.notes` and `.drums` files need no converter: `duet apply` reads them as views.
5. **The view commands' names.** (a) `duet view` and `duet apply --onto | -o`; (b) `duet view` and
   `duet view --read`; (c) `duet export --as` and `duet import`. **Recommended: (a)**: two verbs for
   two directions, and (c) collides with what `convert` means for MIDI and ABC.
6. **TOML.** Config only, or also a document container. **Recommended: config only**, since it has no
   `null` and the envelope, not the container, says what a file is.
7. **A JSON Schema validator as a dev-dependency** (`jsonschema`, MIT, default features off) to prove
   documents match their committed schemas. **Recommended: yes**, since the five hand-serialized types
   are exactly where a derived schema would lie.

Also proposed, and not a ruling unless the owner wants one: a new `duet-documents` crate for the kind
registry and the gate; the view header's source digest, which makes `apply --onto` refuse a source
changed since rendering; merging a view onto its source cell by cell rather than by ids; `capture-analysis` omitting absent values rather than writing `null`; and
`tempo_alternatives` named `{half, double}`.

## Not decided here, and why

- **The project kind:** named, and constrained to live inside `.duet/`. What it holds is the project
  work's question.
- **A schema for `duet.toml`:** an editor convenience, not a contract. Deferred until someone wants it.
- **Seconds beside frames in capture events:** the capture design promised them, and the file has frames
  and the rate. Whether to add them is a minor of `capture-analysis`, decided when the comparison tool
  needs it.
- **A domain for the schemas:** only if the owner rules (b) on question 1.
