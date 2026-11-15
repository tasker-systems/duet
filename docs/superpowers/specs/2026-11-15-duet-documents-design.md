# duet documents: design

This designs how [the decision that duet's files are projections of versioned data
structures](#the-decision-this-builds) is built. That decision is settled and is not reopened here.
It answers the eight questions of the design task, names the build tasks in order, and holds the
performance layer's file until its envelope exists.

Every answer below is **Proposed**. Each open question is marked **For the owner to rule**, with a
recommendation. All of them are gathered in [For the owner to rule](#for-the-owner-to-rule).

## The decision this builds

Temper decision `01a1df20-1918-7130-b2d6-90ab20979553`, made by the owner on 2026-11-15, in short:

- Each data structure is a **kind** with a JSON Schema and an explicit version. The document is the
  canonical form.
- One **envelope** gives the kind and the schema version. The file says what it is, not its name.
- `.duet` is a JSON document. `.duet.jsonl` is a stream whose first line is the envelope.
- `.notes` and `.drums` text are **notations**: views rendered from a document and read back, each
  declared lossless or naming what it drops.
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
  (shift, mask, density) is one array operation. The legible view is `.drums`, which is unchanged.
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
    ([question 5's witnesses](#witnesses)). An older schema file is never regenerated, since the code
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

## 5. Notations as views

**Proposed: each notation states the notation version and the kind version it renders, on its first
line, and the reader checks it.**

- **`.notes`:**
  - Today: `# notefmt v1: positions are bar.beat.tick, lengths are quarter notes`, which nothing reads.
  - v1: `# notefmt v1 (song 1.0): positions are bar.beat.tick, lengths are quarter notes`.
- **`.drums`:**
  - Today: `# notefmt drums v1: one row per lane, one column per step, | at bar lines`.
  - v1: `# notefmt drums v1 (drum-grid 1.0): one row per lane, one column per step, | at bar lines`.
- **What the reader checks.** The line stays a comment to any other tool, and its form is fixed:
  - **The parenthesis names a newer version of the kind than this duet reads:** refused by name, under
    [the version policy](#3-the-version-policy).
  - **It names a newer notation version:** refused by name.
  - **The old line without the parenthesis** reads as the 1.0 of the kind. Every file written before
    this design keeps its meaning.
  - **No such line at all** reads as the current version. Hand-written text is read leniently, as it is
    today (`hand_written_text_is_read_leniently` in both round-trip files).
- **Notation versions and kind versions move separately.** A minor of the kind that the notation can
  render without a change of syntax changes only the parenthesis. A change of syntax bumps the
  notation's version.

**The declarations.** Stated in each notation's module docs and in README, and proven by
[the witnesses](#witnesses):

- **`.notes` is lossless against song 1.0.**
  - It holds every field of a song document's body: the conductor, the end, every track, note and
    control, and each pitch's spelling.
  - It holds the **normalized** body. A loose position (`1.1.960` in 4/4) or a length off the ticks is
    written in its normalized form. Every reader normalizes, so a document never holds the loose form.
  - It always states the end, so a body with no `end` reads back with its default end stated. Every
    writer states it anyway, so no document duet writes lacks one.
  - **It drops the envelope's `tool` and `from`.** They are about the file, not the song.
- **`.drums` is lossless against drum-grid 1.0, and refuses what it cannot hold.**
  - It holds the conductor, the end, the step, the channel, the levels, the lanes and every cell.
  - **It refuses**, naming the lane, bar and step, a velocity that is not 0 or one of the three
    levels. It never drops or snaps one unasked (`off_level_velocities_are_refused_as_text_and_snapped_only_when_asked`).
  - **It drops the envelope's `tool` and `from`.**

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
- **`from` is kept on write-back** of a `.duet` that has it, and dropped by every notation (question 5).

## 7. Today's names and readers

**Proposed:**

| Today | Becomes | Reading the old name |
|---|---|---|
| `<x>.json`, `<x>.toml` (a song) | `<x>.duet` (`song` 1.0) | A bare song body. See the ruling below. |
| `<x>.drums.json`, `<x>.drums.toml` | `<x>.duet` (`drum-grid` 1.0) | The old shape, converted on read. See the ruling below. |
| `<stem>.capture.json` | `<stem>.capture.duet` (`capture-analysis` 1.0) | Not read. It never had a reader. |
| `<stem>.performance.jsonl` (designed, not yet written) | `<stem>.performance.duet.jsonl` (`performance` 1.0) | Born in the envelope. |
| `.notes`, `.drums`, `.mid`, `.abc` | unchanged | Unchanged. |

- **The dispatch in `duet-cli/src/io.rs`:**
  - **`.duet` and `.duet.jsonl` are read by their envelope.** The kind decides what the file is, so a
    song command given a drum-grid document refuses it naming the kind: "beat.duet is a drum-grid; duet
    play reads a song; duet convert goes between a grid and a song".
  - **Notations and outside formats keep their names** (`.notes`, `.drums`, `.mid`, `.abc`). They have
    no envelope. A name is now a convenience for them, and the deciding fact for nothing else.
  - **Standard input (`-`):** a first non-blank byte of `{` is a document, read by its envelope.
    Anything else is the command's default format, as today.
  - **Names are conventions:** `<x>.duet` for a song or a grid, `<stem>.capture.duet` and
    `<stem>.performance.duet.jsonl` for capture's outputs. The reader never relies on them.
- **The rule this replaces:** "a file name means the same to every command" becomes "a file's
  envelope means the same to every command". The Temper decision on `.drums`, `.drums.json` and
  `.drums.toml` names is superseded in this part, as the settling decision says.

**For the owner to rule: the old JSON and TOML files.**

- **Options:**
  - (a) keep **reading** `.json`, `.toml`, `.drums.json` and `.drums.toml` as legacy inputs, with a
    note on stderr naming `duet convert <file> <x>.duet`; stop **writing** them; remove the readers
    at the next minor release of duet after this ships;
  - (b) cut them now, readers and writers both;
  - (c) keep reading and writing them indefinitely as bare bodies.
- **Recommendation: (a).** A bare song `.json` already is a `song` 1.0 body, so reading it costs one
  branch. The old grid shape needs one conversion, which drum-grid 1.0's `read_minor` would hold anyway
  if (a) of the drum ruling is taken. `duet convert` is the converter, so there is no new tool. (c)
  keeps two forms of every kind, which is what this decision ends.
- **The question only the owner can answer:** are there `.json`, `.toml` or `.drums.*` files on the
  owner's machine that matter? If none, (b) is as good as (a) and simpler.

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
  - `.notes` → document → `.notes` is byte-identical, and document → `.notes` → document is equal, over
    the same songs.
- **drum-grid 1.0** (in `crates/duet-drums/tests/roundtrip.rs`):
  - the random grids of `the_drum_grid_round_trips_under_its_map` round-trip through a document
    exactly;
  - `.drums` → document → `.drums` is byte-identical;
  - a grid in today's `.drums.json` shape reads to the same grid as its document (if the legacy reader
    is kept).
- **The gate** (`crates/duet-documents/tests/schemas.rs`):
  - its compare function, given a schema with a field added and the same version, fails and names
    the kind;
  - given a description-only change, it asks to regenerate and asks for no bump;
  - every committed example document (`schemas/<kind>/examples/<version>.duet`) reads with the current
    reader, and validates against its version's schema (if the validator is ruled in).
- **Notations:** a `.notes` or `.drums` header naming a newer kind version is refused by name; the old
  header and no header both still read.
- **Outside formats:** an ABC tune with a slur converts to a `.duet` whose `from` lists that notice at
  its line and column; converting that `.duet` back to `.abc` and in again gives the same body.
- **The CLI** (in `crates/duet-e2e`): `duet convert x.notes x.duet` then `x.duet y.notes` gives
  `y.notes` identical to `x.notes`; a drum-grid `.duet` given to `duet play` is refused naming its
  kind; a document on standard input is read by its envelope.

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
   kind by name, sniffs standard input, and writes `.duet`. The old JSON and TOML names as ruled.
   Witnesses: the CLI tests in `crates/duet-e2e`.
4. **drum-grid 1.0.** The tensor, `bar_steps`, `Bar.bar` gone, `schemas/drum-grid/1.0.json`, and the
   legacy grid reader if ruled. Witnesses: drum-grid 1.0 in `crates/duet-drums/tests/roundtrip.rs`.
5. **Notations declare what they render.** The checked header line in `.notes` and `.drums`, and the
   lossless declarations in module docs and README. Witnesses: the notation tests, and the `.notes`
   and `.drums` byte-identity tests above.
6. **Outside formats record where a song came from.** `from` filled by ABC and MIDI import, kept on
   write-back. Witness: the ABC slur test.
7. **capture-analysis 1.0.** `Deserialize` on the analysis types, absent values omitted,
   `tempo_alternatives` named, `<stem>.capture.duet` written, and its schema committed. Witnesses: an
   analysis document round-trips exactly through its reader and validates against its schema.
8. **performance 1.0.** The performance design's task 2, written as `<stem>.performance.duet.jsonl`.
   Witnesses: that design's question 1, plus: the first line is the envelope, and every later line
   validates against the record schema on its own.

Tasks 4 to 7 need 3 and can be built in any order. Task 8 needs only 1 and 2, so the held performance
task can start before the CLI work lands.

## Amendments to the other designs

Each gets a pointer where its file names change, and nothing else:

- **[The capture design](2026-11-14-capture-design.md):** `<stem>.capture.json` becomes
  `<stem>.capture.duet`, a `capture-analysis` document.
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
4. **The old `.json`, `.toml`, `.drums.json` and `.drums.toml` files.** (a) read them as legacy inputs
   for one minor release, write only `.duet`; (b) cut them now; (c) keep them indefinitely.
   **Recommended: (a)**, or (b) if no such file on the owner's machine matters. Are there any?
5. **TOML.** Config only, or also a document container. **Recommended: config only**, since it has no
   `null` and the envelope, not the container, says what a file is.
6. **A JSON Schema validator as a dev-dependency** (`jsonschema`, MIT, default features off) to prove
   documents match their committed schemas. **Recommended: yes**, since the five hand-serialized types
   are exactly where a derived schema would lie.

Also proposed, and not a ruling unless the owner wants one: a new `duet-documents` crate for the kind
registry and the gate; `capture-analysis` omitting absent values rather than writing `null`; and
`tempo_alternatives` named `{half, double}`.

## Not decided here, and why

- **The project kind:** named, and constrained to live inside `.duet/`. What it holds is the project
  work's question.
- **A schema for `duet.toml`:** an editor convenience, not a contract. Deferred until someone wants it.
- **Seconds beside frames in capture events:** the capture design promised them, and the file has frames
  and the rate. Whether to add them is a minor of `capture-analysis`, decided when the comparison tool
  needs it.
- **A domain for the schemas:** only if the owner rules (b) on question 1.
