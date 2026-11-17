# duet: one take read several ways, and a choice between the readings per moment

**Date:** 2026-11-17. **Task:** temper `01a1e77f-3b11-72df-98a6-3414d95e3183`.

**Status:** the direction is **Ruled (owner, 2026-11-17)**: five rulings, quoted in
[The rulings this builds](#the-rulings-this-builds) and not reopened here. This spec designs how
they are built. Everything else is **Proposed**. Where a question needs the owner, it is marked
**Owner question**, with options and a recommendation, and all of them are listed in
[Questions for the owner](#questions-for-the-owner).

**Builds on:**
- [duet documents](2026-11-15-duet-documents-design.md): the envelope, kinds, the version policy,
  the views, and the FNV-1a digest in a view's header.
- [The performance and its reading](2026-11-15-performance-and-reading-design.md): the reading,
  its residual, and the transformations (question 5).
- [Arrivals and ornaments](2026-11-16-arrivals-and-ornaments-design.md): excursions, and the
  reading's passes (rejoin, breathy).
- The decision "Intent is recorded beside the capture, never over it" (temper
  `01a1e524-486a-77dd-ac84-de2e665729dc`, with its "Clarified" section).
- The transformations task's "Ruled" section (temper `01a1df08-2b63-7658-8812-7823c97c6147`):
  per-note keep-or-correct marks.
- The held-approach build task (temper `01a1e77f-02fb-73bc-8c08-e1c3243db9a0`), being built in
  parallel. It takes `capture-analysis` 1.11 and adds a pass, which this design names and does not
  touch.
- The jobs-and-boundaries note (temper `01a1e57c-f9c8-72e5-8a9b-6a4e5874bfdc`), §7: the bus is the
  project's session record, a `.duet.jsonl` stream, and no server is presumed.

**Grounding:** the code at `main` (`672f429`, `capture-analysis` 1.10), and
`crates/duet-capture/corpus/wayfaring-stranger.flac` (CC BY 4.0) read by `duet capture melody
--reader rules` and `--reader hmm` with nothing given. Times are seconds into that take.

## The rulings this builds

**Ruled (owner, 2026-11-17)**, from the task's `rulings`:

1. **Each reading stays its own `capture-analysis` document,** with a variant annotation. That is
   additive. There is no new "take" kind yet: listing a take's variants belongs to the project or
   session record (`duet init` task `01a1e007-8747-77ee-8333-dd462d6d01eb`, workbench task
   `01a1e563-22e6-70e6-a7ce-8b38e93b3091`).
2. **The annotation lives in the body, not the envelope.** It holds the take's audio digest with
   its rate and length, since the filename is never authoritative; the reader, as `reader` does
   today, thresholds included; and the passes that ran, in order.
3. **One way to state intent: a new `choices` kind, 1.0.** It holds span choices and the
   transformations task's per-note keep-or-correct marks. Applying it writes a new reading that
   states every change. The variants stay untouched.
4. **The score comes from the applied choices:** a base variant (the HMM by default) plus the
   chosen spans. An unchosen moment takes the base, and that is stated.
5. **The workbench:** the review sheet's verdicts write choice entries, the session record logs the
   act, and the choices document is the durable state.

## What the take shows

The two readers of Wayfaring Stranger disagree in short stretches, and agree on a boundary either
side of each. Call a frame where no note of either reading is sounding across it a **seam**: each
reading has a note boundary there, or no note. On this take (22.1 s, 22 050 Hz, mono) the two readings have 42
seams.

The smallest stretch between two seams around each moment the owner ruled on (task D, temper
`01a1e4f9-0dc3-70e4-b0d8-49e065c07148`) holds that moment's disagreement and little else:

| Moment | Between seams | HMM reads | Rules read |
|---|---|---|---|
| 1.42 | 1.057–1.643 | D#3 | D#3, E3 |
| 2.28 | 2.154–3.582 | A#3 | G#3, A#3 |
| 3.77 | 3.605–4.162 | A#3 | G#3, A#3 |
| 5.17 | 5.103–5.730 | D#3 | C#3, D#3 |
| 9.07 | 8.911–10.315 | D#3, C3 | C3, D#3, C3 |
| 12.03 | 11.952–13.357 | A#3, C4 | A3, A#3 |
| 13.84 | 13.636–14.356 | A#3 | G#3, A#3 |
| 14.87 | 14.843–15.459 | G#3 | F#3, G#3 |
| 18.56 | 16.771–18.884 | G#3, F#3 | G#3, F3, F#3 |
| 20.05 | 19.644–20.114 | D#3 | D#3 |

- **A choice at a moment can be cut at seams, with no note shortened or lengthened.** Inside a
  stretch, each reading's notes start and end inside it. Taking one reading's notes there in place
  of the other's moves no boundary.
- **A small tolerance would not help much.** Letting a cut fall within 25 or 50 ms of a note's
  boundary shortens only the 18.56 stretch (to 18.49–18.88). Everywhere else the stretch is the
  same. So this design cuts at seams exactly ([question 3](#3-a-span-and-where-it-may-be-cut)).
- **What the owner ruled on maps to one stretch each:** "2.28 s: rules" takes the rules' G#3 and
  A#3 in place of the HMM's one A#3.
- **These are today's readings.** Once the held-approach pass lands (1.11), the HMM reads 2.28,
  3.77, 5.17, 9.07, 12.03, 13.84, 14.87 and 18.56 as notes too, and the stretches will move. The
  witnesses below are written against the readings as they are when each task is built.

## 1. The variant annotation

**Proposed. `capture-analysis` 1.12, or the next free minor, additive.** 1.11 is the held-approach
pass's.

The annotation is three things, two of them new:

```json
"body": {
  "rate": 22050,
  "take": {
    "digest": "sha256:9f2c…64 hex…",
    "rate": 22050,
    "channels": 1,
    "frames": 488040
  },
  "events": [ … ],
  …
  "reading": {
    "tuning": { … },
    "reader": { "name": "hmm", "note_cost": 4.0 },
    "passes": ["breathy", "rejoin", "held-approach"],
    "notes": [ … ]
  }
}
```

- **`take`**, on `Analysis`: the audio the analysis was read from. It is optional, and absent in an
  older document, meaning "not stated". It is on the analysis, not the reading, because a rhythm
  take has one too and has no reading.
  - `digest`: see [question 2](#2-the-take-digest).
  - `rate`, `channels` and `frames`: the source's own sample rate, channel count and length in
    frames, as decoded. These are the rate and length the ruling asks for. `channels` is added
    because a length in frames does not say how many samples it is.
- **`reader`**, unchanged: the name, the cost per note, and Basic Pitch's thresholds.
- **`passes`**, on `Reading`: each interpretive pass that ran over the decoded notes, by name, in
  the order it ran. It is optional, and absent in an older document, meaning "not stated".
  - Today's names: `breathy` and `rejoin`. The held-approach pass adds its own, `held-approach`
    unless its task names it otherwise, in the place its task gives it in [`interpret`].
  - **A pass that ran and changed nothing is listed.** That is the point: `rejoin` listed with no
    `rejoins` means the rejoin ran and found none, where a 1.11 reading's empty `rejoins` cannot say
    whether it ran.
  - **[`interpret`] writes it,** each pass appending its name as it runs, so it holds under every
    reader and cannot drift from what ran.
  - Each name is a variant of a `Pass` enum, written in kebab case, so a new pass is a new variant:
    a minor. Its parameters are compile-time constants today, so a name is enough. If a pass ever
    takes a parameter, it gains an optional field in a minor, as `reader.thresholds` did in 1.10.

**What makes two readings variants of one take.** The same `take.digest`. Two readings are the
**same variant** when their `take`, `reader` and `passes` are all equal. A choices document names a
variant by these three, never by a path ([question 4](#4-the-choices-kind)).

**An older reading is not a variant.** A document from before this minor has no `take` and no
`passes`. A choices document refuses to name it, saying to capture the take again. Guessing its
passes from its version would be a guess.

**Output names are conventions.** Two readings of one take written side by side need different
stems, such as `wayfaring.hmm.capture.duet` and `wayfaring.rules.capture.duet`. Nothing reads the
name.

## 2. The take digest

**Proposed: SHA-256 over the decoded samples, not the file's bytes.**

- **What is hashed, exactly:** the ASCII bytes `duet-take-1`, then the rate as a little-endian
  `u32`, the channel count as a little-endian `u16`, then every sample as `FileSource` yields it,
  interleaved and in order, each as the little-endian bits of its `f32`.
- **It is written** as `"sha256:"` and 64 lowercase hex digits.
- **It is computed as the take is read,** a chunk at a time, beside `Prepare`. So it obeys the
  stream rule, and a live take later can be digested the same way.

**What it survives, and what it does not:**

| Change to the file | Same digest? |
|---|---|
| FLAC re-encoded at another compression level or block size | yes |
| Tags or pictures added, changed or removed | yes |
| FLAC to WAV, AIFF or ALAC at the same bit depth, and back | yes |
| The same samples at a wider bit depth (16 to 24 bits) | yes, if Symphonia's conversion to `f32` scales by a power of two (task 1 checks it) |
| A lossy re-encode (MP3, AAC, Ogg) of the take | no: those are other samples |
| Trimmed, resampled, gain changed, downmixed | no: another take |
| A lossy file decoded by a later Symphonia that rounds differently | no: a stated limit |

**Owner question 1: what the digest is over.**
- **Options:**
  - (a) the decoded samples, as above;
  - (b) the file's bytes;
  - (c) both: the samples as identity, and the bytes kept beside them, to find a file quickly.
- **Recommendation: (a).** It survives every re-encoding that keeps the take's samples, which is
  what the ruling asks of it. (b) breaks on a tag edit, and a phone's share sheet or a DAW export
  rewrites tags. (c) adds a field that only helps searching, and searching belongs to the project
  record.
- **What (a) costs:**
  - **One hash pass during capture,** over samples that are being decoded anyway. A three-minute
    stereo take at 44.1 kHz is 64 MB of samples, about a tenth of a second of SHA-256 beside
    seconds of pitch tracking.
  - **A dependency moves.** `sha2` 0.10.9 (MIT OR Apache-2.0) is already in the lock file, as a
    dev-dependency of duet-dsp. It becomes a normal dependency of the crate that holds the
    digester. Its licence is in `deny.toml`'s allow list.
  - **Checking that a file is a given take means decoding all of it.** Nothing in this design does
    that on the way to applying choices: every check is between documents, which already hold the
    digest.
  - **A lossy take's digest depends on its decoder.** The owner's phone takes are AAC in M4A. If a
    later Symphonia decodes one differently, its new readings are another take, and a choices
    document over the old ones is refused by name, not misapplied. Capturing again from the same
    file fixes it.

**Owner question 2: whether the performance stream states its take too.** Applying choices
recomputes a folded note's residual and the take's loss from the performance's frames
([question 6](#6-applying-choices)). Today a performance stream does not say which take it is of,
and `compare` checks it by rate and length.
- **Options:**
  - (a) `performance` 1.1, additive: the head gains the same optional `take`, written by the same
    pass;
  - (b) no change: apply checks the performance against the variants by rate, hop and length, as
    `compare` does.
- **Recommendation: (a).** It is the same digest, computed once, and makes the check exact. A 1.0
  stream still reads, with the length check.

## 3. A span, and where it may be cut

**Proposed.**

- **A span** is a stretch of the take, `from` to `to`, and the variant chosen in it.
- **Both edges are seams** of the base and the chosen variant: frames where no note of either is
  sounding across. A seam is a note boundary of one of them, or a frame inside no note of either.
- **So a span takes whole notes.** Inside it, every note of the chosen variant that starts there
  replaces every note of the base that starts there. No note crosses an edge, so nothing is
  shortened, lengthened or overlapped.
- **A choice is asked for at a moment, and stored as its span.** "2.28 s: rules" finds the nearest
  seam at or before 2.28 and the nearest after it, and stores 2.154–3.582 with `at` 2.28. The span
  is what is applied. `at` records where it was asked.
- **A span may choose the base.** "12.03 s: HMM", the base, is stored like any other span. It
  differs from an unchosen moment only in what is stated: "chosen" against "the base, where nothing
  was chosen".
- **Spans never overlap.** A choice asked inside an existing span is refused, naming it, unless it
  is asked to replace it.

**Owner question 3: where a span may be cut.**
- **Options:**
  - (a) at seams only, exactly;
  - (b) within a tolerance of a seam, such as 25 ms, moving the boundary of whichever note crosses
    the edge, and stating the move.
- **Recommendation: (a).** On Wayfaring, (b) changes one moment of ten
  ([What the take shows](#what-the-take-shows)), and it would make every splice an edit of a
  measured boundary. A long stretch between seams is shown and stated, so the owner sees what a
  choice covers before making it.

## 4. The choices kind

**Proposed. `choices` 1.0, new.** A `.duet` document. Its type lives beside the reading in
duet-dsp, and its `Kind` impl in duet-schemas, as for every kind.

```json
{
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/choices/1.0.json",
  "duet": { "kind": "choices", "version": "1.0", "tool": { "name": "duet", "version": "0.4.0" } },
  "body": {
    "take": { "digest": "sha256:9f2c…", "rate": 22050, "channels": 1, "frames": 488040 },
    "rate": 22050,
    "variants": {
      "hmm":   { "reader": { "name": "hmm",   "note_cost": 4.0 }, "passes": ["breathy", "rejoin", "held-approach"], "reading": "3f9a1c0d5e2b7a41" },
      "rules": { "reader": { "name": "rules", "note_cost": 4.0 }, "passes": ["breathy", "rejoin", "held-approach"], "reading": "0b7e33c19a5d2f84" }
    },
    "base": "hmm",
    "spans": [
      { "from": 47496, "to": 78983, "variant": "rules", "at": 50274,
        "said": "the G#3 is a note: \"poor\" moves from G#3 to A#3" }
    ],
    "marks": [
      { "variant": "hmm", "at": 328876, "key": 54, "mark": "fold",
        "said": "a rise traced up to G#3, the real target" }
    ]
  }
}
```

- **`take`:** the take every variant reads, as the annotation states it.
- **`rate`:** the analysis rate the frames below count at, as `Analysis.rate` does.
- **`variants`:** each variant by a label the document defines, bound to its annotation.
  - `reader` and `passes` are copied from the variant's reading.
  - **`reading`** is the 64-bit FNV-1a digest of the variant's `reading` as compact JSON, the same
    function a view's `from` uses. It is not part of the variant's identity. It says which
    reading the entries were made against ([question 5](#5-when-a-variant-is-read-again)).
  - **The label is a name inside this document only.** The writer uses the reader's name (`hmm`,
    `rules`, `basic-pitch`) and asks for one when two variants share a reader.
  - Two labels bound to the same annotation are refused: they are one variant.
  - A chosen reading ([question 6](#6-applying-choices)) is refused as a variant in 1.0. Its notes
    are already choices over variants, and a choice over choices is not needed yet.
- **`base`:** the label of the variant every unchosen moment takes. The writer makes the HMM's
  reading the base unless told otherwise.
- **`spans`:** in order of `from`, not overlapping, each edge a frame on the analysis timeline.
  `at` (where it was asked) and `said` (the verdict in the owner's words) are optional.
- **`marks`:** the transformations task's per-note marks, in order of `at`.

### How a mark names a note

**Proposed: by its variant, a frame inside its held part, and the key it was read on.**

- **`variant`:** the variant the note comes from.
- **`at`:** a frame in the note's held part, `held_from` up to `end`. The writer uses the frame of
  the moment asked at, when it falls in the held part, or else the middle of it.
- **`key`:** the MIDI key the reading gave the note. It is the key as heard, never a chosen one.
- **It finds exactly one note,** of that variant, whose held part holds `at` and whose key is
  `key`. Anything else is refused, naming the mark and what that variant now reads at `at`.
- **Stable across re-reading** means: a mark survives any re-read that keeps a note on that key
  sounding at that moment, which is what a person means by "the F#3 at 14.87". If a re-read moves
  the note off the moment or onto another key, the mark is refused, never moved to a neighbour.

**Owner question 4: how a mark names a note.**
- **Options:**
  - (a) variant, a frame in the held part, and the heard key, as above;
  - (b) the note's index in its variant's reading;
  - (c) an id written into every read note.
- **Recommendation: (a).** (b) shifts whenever a pass adds or removes a note anywhere earlier, so
  the held-approach pass alone would move every later mark onto the wrong note. (c) adds a field to
  every note, and a re-read writes new ids anyway, so it is no more stable than (a) and says less
  about which note it is.

### The marks

From the transformations task's "Ruled" section, plus one this design adds:

| Mark | JSON | What it chooses |
|---|---|---|
| `keep` | `{"mark":"keep"}` | the note as heard, residual and all. It states a verdict that would otherwise be silence |
| `key` | `{"mark":"key","to":58}` | the key the note plays on. A corrected note plays with no bend |
| `drop` | `{"mark":"drop","parts":["slide","excursions"]}` | residual parts not rendered as bends. Part names are the residual's own fields |
| `fold` | `{"mark":"fold"}` | **added:** the note folded into the note it reaches, as a held approach |

- **`fold` is the held-approach ruling's "folding an approach into its target is a stated choice".**
  The note after the marked one, in the chosen reading, becomes one note from the marked note's
  start, with the target's key, centre and held part. The marked note's frames become part of its
  attack, and its residual is measured again (they show as an excursion or a slide).
- **A note may carry several marks** unless they conflict. `keep` with any other mark, two `key`
  marks, and `fold` with `key` on the same note are each refused by name.
- **A fold needs the note it reaches.** The next note must start within `MAX_GAP_SECS` of the marked
  note's end, and in the same span. Otherwise it is refused, naming the gap or the seam.
- **The take-wide shorthands stay shorthands.** "Snap to key" writes a `key` mark for each
  suggestion. Accepting every held-approach suggestion writes a `fold` mark for each. Each is stated
  as written.

## 5. When a variant is read again

A variant is read again whenever capture runs again: a new duet, a changed pass, a changed reader.
The take and the annotation may stay the same while the notes change.

**Proposed: refused loudly by default.**

- **Each variant is found by its annotation** among the readings given ([question 6](#6-applying-choices)).
  - None matches: refused, naming the label, the reader, the passes and the first 12 hex digits of
    the take digest.
  - Two match with different readings: refused, naming both files.
  - A reading of another take is never a match, whatever its name.
- **A variant whose reading digest differs from the stored one** is a re-read. It is refused,
  naming the label, both digests, and how many spans and marks were made against the old reading.
  Nothing is written.
- **`--recheck`** places every entry against the re-read variant instead:
  - a span whose edges are still seams, and a mark that still finds exactly one note, is applied,
    and stated as rechecked;
  - an entry that does not place is refused by name, and nothing is written;
  - **no entry is ever dropped.** Removing one is an edit to the choices document, made on purpose.

**Owner question 5: whether a successful `--recheck` updates the stored digests.**
- **Options:**
  - (a) yes: it rewrites only the variants' `reading` digests in the choices document, says so, and
    logs it in the session record;
  - (b) no: the document is never written by applying it, so every later apply needs `--recheck`
    until the document is edited.
- **Recommendation: (a).** Every entry was checked against the new reading, so the new digest is
  true. The entries are unchanged, and the act is recorded. Under (b), `--recheck` becomes a habit,
  and a habit stops being a check.

## 6. Applying choices

**Proposed: `duet choices apply`.** The command names are [owner question 7](#7-the-commands).

```text
duet choices apply song.choices.duet wayfaring.hmm.capture.duet wayfaring.rules.capture.duet -o wayfaring.chosen
```

- **It reads** the choices document, and the readings named after it. With `--in DIR` it looks in a
  directory instead. Either way, each file is matched by its annotation, never its name.
- **It reads the performance** the variants were read from, for folds and the loss: by `take` when
  the stream states it ([owner question 2](#2-the-take-digest)), otherwise by rate, hop and length.
- **It writes,** each refused rather than overwritten, as capture and compare do:
  - `STEM.capture.duet`: the **chosen reading**, in a `capture-analysis` document;
  - `STEM.duet`: the song, the notes view's source;
  - `STEM.abc`: the score.
- **It never writes** a variant, the performance or the choices document (except under
  [owner question 5](#5-when-a-variant-is-read-again) (a)).

### What the chosen reading holds

**`capture-analysis` 1.12, additive** (the same minor as the annotation):

- **`notes`:** the base's notes outside every span, each span's variant's notes inside it, and each
  fold made. **Every other note is byte for byte its variant's,** residual included.
- **`reader`:** a new reader name, `chosen`, with the loss's cost per note. An absent `reader`
  already means "the rules, before 1.3", so the chosen reading cannot leave it absent.
- **`passes`:** empty. No pass ran over the chosen notes. The passes that ran over each variant are
  in `choices.variants`.
- **`rejoins`, `breathy` and the held-approach pass's statements:** carried from each variant for
  the notes taken from it, so the chosen reading still says what each pass did to its notes.
- **`residual`:** the take's residual and loss, measured again over the chosen notes.
- **`choices`, new and optional:** what was applied.
  - `document`: the FNV-1a digest of the choices body applied;
  - `base`, and `variants` as the choices document gave them, so the reading describes itself;
  - `spans`: **tiling the whole take**, each `{from, to, variant, chosen}`. `chosen: false` marks
    where the base was taken because nothing was chosen. This is ruling 4, stated in the body;
  - `folds`: each fold, with the two notes as their variant read them;
  - `marks`: each `keep`, `key` and `drop` mark, by the index of the note it lands on.
- **Keys and drops stay marks.** A note's `key` stays the key as heard, and its residual stays as
  measured. The song, the score and a performance render follow the marks. Writing a chosen key into
  `key` would change what `centre` measures (its distance from the key heard), and its range
  (−50 to 50 cents). That is a change of meaning, so a major.
- **Owner question 9** asks to confirm this.

**The rest of the analysis is the base's:** `events`, `timing`, `key` and `beats`. So a choice never
moves a bar line or re-estimates the tempo. That is stated.

### What it states

Each is a statement in the envelope of the chosen reading and the song, and on stderr. The numbers
are the witness document's over today's readings:

- `notes chosen by song.choices.duet (choices 1.0, 9c04e1b2a7d35f60), over hmm, the base`
- `2.154–3.582 s: rules, chosen at 2.28 s: G#3, A#3 in place of A#3`
- `11.952–13.357 s: hmm, chosen at 12.03 s`
- `hmm, the base, where nothing was chosen: 10 stretches, 12.8 s of 22.1 s`
- `14.91 s: F#3 (hmm) folded into the G#3 after it`
- `12.07 s: A3 (hmm) plays as A#3: key chosen`
- `timing, key and beats are hmm's, unchanged`

### Witnesses for applying

In `crates/duet-dsp` for the splice and fold, `crates/duet-schemas/tests/choices.rs` for the
document, and `crates/duet-e2e` for the command:

- **No entries is the base.** A choices document with a base and no entries gives the base's notes
  exactly. One span is stated, `chosen: false`, over the whole take.
- **One span over the whole take is that variant.** Choosing rules from the first frame to the last
  gives the rules' notes exactly.
- **A span takes whole notes.** On Wayfaring, "2.28 s: rules" over the HMM stores 2.154–3.582. The
  chosen reading has a G#3 then an A#3 there. Every note outside it is byte for byte the HMM's.
- **An edge off a seam is refused,** naming the note that crosses it and the nearest seams either
  side.
- **A fold makes one note.** A synthetic approach a semitone below its target, read as two notes,
  folded: one note from the approach's start, on the target's key, with an excursion or a slide
  over the approach's frames. The two notes as read are stated in `folds`.
- **The variants are byte for byte unchanged.** The SHA-256 of every variant file, and of the
  performance and the choices document, is the same before and after `apply`.
- **A re-read is refused.** One note's `end` moved by a frame in the rules' reading: apply is
  refused naming `rules` and both digests, and nothing is written. With `--recheck`, a span whose
  edges are still seams applies and is stated as rechecked. A span whose edge now falls inside that
  note is refused by name.
- **A mark whose note is gone is refused.** A mark at a frame where that variant now reads another
  key is refused, naming what is there.
- **A reading of another take is no variant.** A reading of the synthetic melody given in place of
  the rules' is refused, naming the take digests.

## 7. Views

**One way to state intent means one kind, not one surface.** The choices document is the only
store. Views are working surfaces over it, as the documents design rules for every kind.

### The choices view, new

**Proposed: `choices v1`, a lossless view of `choices` 1.0.**

```text
# duet choices v1 of choices 1.0 from 9c04e1b2a7d35f60
take sha256:9f2c…  22050 Hz  1 ch  488040 frames  read at 22050 Hz
variant hmm    hmm   cost 4  passes breathy rejoin held-approach  reading 3f9a1c0d5e2b7a41
variant rules  rules cost 4  passes breathy rejoin held-approach  reading 0b7e33c19a5d2f84
base hmm
span 2.154 3.582 rules  at 2.280  "the G#3 is a note: \"poor\" moves from G#3 to A#3"
span 11.952 13.357 hmm  at 12.030
mark hmm 14.915 F#3 fold  "a rise traced up to G#3, the real target"
```

- **`duet view song.choices.duet`** renders it. Given the variants (`--with FILE…`), it adds each
  span's notes under each reading as `# ` comment lines, so a person sees what a span swaps. Those
  are comments, so they are not read back.
- **`duet apply view --onto song.choices.duet`** reads it back, by the header's digest as for every
  view.
- **Times are written in seconds to the millisecond.** On reading back, each resolves to the
  nearest seam (a span's edge) or held frame (a mark's `at`) within half a hop (5.8 ms). Two within
  that are refused as ambiguous, naming both, and `@FRAME` gives one exactly. So the view is
  lossless over every document the CLI writes.
- **A span or mark added in the view is checked as `apply` checks it,** against the variants given
  with `--with`. Without them, it is checked when applied.

### The notes view: marks only, never spans

**Proposed: the notes view can carry per-note marks, not span choices.**

- **Why no spans:** a song's notes view shows one reading's notes, in bars and beats. A span that
  chooses another variant swaps in notes the view does not show, so it would be an edit made
  blind. Its edges are seams in frames, which bar positions cannot state exactly.
- **Marks fit it.** The transformations ruling put them there: "Marks live in the notes view, per
  note … `duet apply` reads them back exactly. A new notes-view syntax is a new version in the
  view's header."
- **What it is, then:** `notes v2` of a chosen song carries each melody note's marks after the note,
  such as `; fold` or `; key=A#3 drop=slide`. Its header names the chosen reading's choices digest
  as well as the song's. `duet apply view --onto song.choices.duet` writes the marks back as
  `marks` entries: each note, through the chosen reading, by its variant, held frame and heard key.
  The marks are never stored in the song.

**Owner question 6: where notes-view marks are built.**
- **Options:**
  - (a) in the transformations task, which renders `key` and `drop` anyway, as an amendment: its
    marks are choices entries, and `notes v2` reads and writes them;
  - (b) here, as its own task before the transformations task.
- **Recommendation: (a).** A mark that nothing renders yet is a mark the owner cannot hear. The
  choices view carries every mark from the start, so nothing waits for (a).

## 8. The workbench

The session record's design belongs to the workbench task. This states what that design needs from
choices, and decides none of its shape:

- **A verdict on the review sheet writes one entry,** through the same action the CLI uses:
  - "rules" or "hmm" on a moment is a span asked at the moment's time, with the verdict as `said`;
  - "fold" or "key" on a note is a mark;
  - "no audible difference" writes **no** entry. The moment stays unchosen and takes the base, and
    the verdict lives in the record and the review set's answers.
- **The session record logs each act:** which choices document, its body digest before and after,
  the entry, and who made it. It is a fact in the record, not the state.
- **The choices document is the state.** Replaying the record is never needed to know what was
  chosen.
- **Before a choice is made, the sheet shows the stretch it will cover** between seams, and both
  readings' notes in it.

## The witness document: the owner's task-D rulings

`crates/duet-capture/corpus/wayfaring-stranger.choices.duet`, committed with task 2. It holds no
audio, only frames and the owner's words. Base: the HMM. One entry per ruling:

| Moment | Ruling (task D) | Entry |
|---|---|---|
| 1.42 | the HMM is right | span, `hmm` |
| 2.28 | the rules are right: the G#3 is a note | span, `rules` |
| 3.77 | the rules are right | span, `rules` |
| 5.17 | the rules are right: the C#3 is a note | span, `rules` |
| 9.07 | the rules are right: C3 is a real note | span, `rules` |
| 12.03 | the HMM is closer: one A#3 with its approach | span, `hmm`; and `fold` at 12.03 once the HMM reads the A3 as a note |
| 13.84 | the rules capture the ornament right | span, `rules` |
| 14.87 | the HMM, probably: a rise traced up to G#3 | `fold` at 14.87 on the HMM's F#3 |
| 18.56 | the rules' capture is more precise | span, `rules` |
| 20.05 | the HMM is right | span, `hmm` |
| 9.55, 16.39, 16.40, 19.27 | no audible difference | none |

**After the held-approach pass,** the HMM reads the A3 at 12.03 and the F#3 at 14.87 as notes of
their own, the first with a "likely approach" suggestion. "The HMM is closer" at 12.03 then means
the A3 folded, not the HMM's span. **Owner question 8** asks to confirm it.

**What applying it reads**, the witnesses of task 2:
- one D#3 at 1.42 with its turn; G#3, A#3 at 2.28; G#3, A#3 at 3.77; C#3, D#3 at 5.17;
- C3, D#3 at 9.07; one A#3 at 12.03; G#3, A#3 at 13.84; one G#3 at 14.87;
- F3, F#3 at 18.56; and the D3 after the D#3 at 20.05;
- every note outside a span, the HMM's, byte for byte;
- the unchosen stretches stated.

## The version line

| Format | Version | Change | Task |
|---|---|---|---|
| `capture-analysis` | 1.11 | the held-approach pass's suggestion (not this design's) | the held-approach task |
| `capture-analysis` | **1.12**, or the next free minor | **additive:** optional `take` on the analysis; optional `passes` on the reading; the reader name `chosen`; optional `choices` on the reading | 1 and 2 |
| `choices` | **1.0** | **new kind**, a `.duet` document | 2 |
| `performance` | **1.1**, if owner question 2 is (a) | **additive:** the head's optional `take` | 1 |
| the `choices` view | **v1** | **new view** | 4 |
| the `notes` view | **v2** | **new syntax:** per-note marks | the transformations task, under owner question 6 (a) |
| `song`, `drum-grid`, the envelope | none | | |

Each minor follows the version policy's rule: a new optional field, omitted when absent, whose
absence means exactly what the minor before meant, and a new variant of a tagged enum. Every older
example of each kind still reads (the gate's witnesses).

## Build tasks

Each names its witness. None is built before the one it needs.

**1. The variant annotation.** `capture-analysis` 1.12 (`take`, `passes`), and `performance` 1.1
under owner question 2 (a).
- **Needs:** the held-approach task merged, so its pass is listed and the minor is free.
- **What:** a streamed SHA-256 digester fed every chunk the capture command reads; `take` on the
  analysis; `passes` written by `interpret` as each pass runs; schemas and examples committed.
- **Witnesses:**
  - The same samples written as FLAC by `TakeWriter` and as a 16-bit WAV have one digest. One
    sample changed by one step changes it.
  - Wayfaring's digest is pinned in the corpus manifest, and the committed file matches it.
  - Every reading lists its passes in the order they ran, under the rules, the HMM and Basic Pitch.
    A pass that changed nothing is still listed.
  - Every older example still reads, as one with no take or passes stated.
  - An analysis document round-trips exactly through its reader, and validates against 1.12.
  - Whether a 24-bit re-encode of 16-bit samples keeps the digest is checked, and stated either
    way in the digester's docs.

**2. `choices` 1.0, and applying it.**
- **Needs:** 1.
- **What:** the kind, its schema and example; the splice and fold in duet-dsp, beside `interpret`;
  the re-read checks; `duet choices apply`; the chosen reading's `choices` and the reader `chosen`;
  the witness document committed.
- **Witnesses:** [applying's](#witnesses-for-applying), and the witness document's.
  - A choices document round-trips exactly, and validates against its schema.
  - Overlapping spans, an unknown label, two labels for one annotation, and a chosen reading named as
    a variant are each refused by name.

**3. Writing choices from the CLI.**
- **Needs:** 2.
- **What:** `duet choices new` (from the base reading and the other variants), `duet choices choose
  --at SECS LABEL [--said TEXT]`, `duet choices mark --at SECS [fold|keep|key=…|drop=…]`, and
  `duet choices unset --at SECS`.
- **Witnesses** (in `crates/duet-e2e`):
  - On Wayfaring, `choose --at 2.28 rules` stores 2.154–3.582 with `at` 2.28. Asked again inside
    it, it is refused without `--replace`.
  - `mark --at 14.87 fold` on the HMM stores a mark on the note there, by its held frame and key.
  - `unset` removes exactly that entry, and the rest of the document is byte for byte unchanged.
  - The document written by the commands applies as the committed witness document does.

**4. The choices view.**
- **Needs:** 2.
- **What:** `choices v1`: `duet view` of a choices document, `--with` for context lines, and
  `duet apply --onto`.
- **Witnesses:**
  - document → view → document is equal, for the witness document; view → document → view is byte
    identical.
  - One edit in the view (a span's variant, a mark's kind) changes exactly that entry.
  - An ambiguous time, an edge off a seam (with `--with`) and a changed source are each refused by
    name.
  - A view naming a newer view or schema version is refused by name.

**5. Marks in the notes view, and renders that follow them.** An amendment to the transformations
task (temper `01a1df08-2b63-7658-8812-7823c97c6147`) under owner question 6 (a), not a new task.
- **Needs:** 2, 4, and that task's own needs.
- **Witnesses:** that task's per-note witnesses. Plus: a `notes v2` view applied onto a choices
  document writes `marks` entries, and the song is unchanged; and a `key` mark makes the chosen
  song's and score's note that key, while the chosen reading's `key` stays the heard one.

**6. The review sheet writes entries.** Part of the workbench design (temper
`01a1e563-22e6-70e6-a7ce-8b38e93b3091`), not filed here.
- **Witness:** the four-moment review the jobs-and-boundaries note proposes as its test, rerun with
  verdicts that write entries through task 3's action. The choices document it leaves applies, and
  the record holds one act per verdict.

## Questions for the owner

Each has a recommendation. Nothing is built on an answer before it is given.

1. **What the take digest is over** ([§2](#2-the-take-digest)): the decoded samples, the file's
   bytes, or both. *Recommended: the decoded samples.*
2. **Whether the performance stream states its take** ([§2](#2-the-take-digest)): `performance` 1.1
   with the same `take`, or checked by rate and length as today. *Recommended: 1.1.*
3. **Where a span may be cut** ([§3](#3-a-span-and-where-it-may-be-cut)): at seams exactly, or
   within a tolerance with the boundary moved and stated. *Recommended: seams exactly.*
4. **How a mark names a note** ([§4](#how-a-mark-names-a-note)): variant, held frame and heard key;
   the note's index; or an id in every note. *Recommended: variant, held frame and heard key.*
5. **Whether `--recheck` updates the stored digests** ([§5](#5-when-a-variant-is-read-again)).
   *Recommended: yes, the digests only, stated and logged.*
6. **Where notes-view marks are built** ([§7](#the-notes-view-marks-only-never-spans)): in the
   transformations task, or here. *Recommended: in the transformations task.*
7. **The command names** ([§6](#6-applying-choices)): `duet choices new|choose|mark|unset|apply`,
   or the same verbs under `duet capture`. *Recommended: a `duet choices` group,* since a choices
   document is not a capture, and `duet capture` already holds `melody`, `rhythm` and `compare`.
   Applying is `duet choices apply`, not top-level `duet apply`, which reads views.
8. **12.03 after the held-approach pass** ([the witness document](#the-witness-document-the-owners-task-d-rulings)):
   "the HMM is closer" written as `fold` on the A3, once the HMM reads it as a note.
   *Recommended: yes.*
9. **A chosen key stays a mark** ([§6](#what-the-chosen-reading-holds)): the chosen reading keeps
   the key as heard, and the song, score and renders play the chosen key. Rewriting `key` would
   change what `centre` means, which is a major. *Recommended: it stays a mark.*

## Not decided here

- **A "take" kind, and listing a take's variants:** the project and session record's job
  (ruling 1).
- **Capturing with a pass left out,** which would make variants that differ only in passes. The
  annotation can state it, and nothing asks for it yet.
- **A choice over chosen readings:** refused in 1.0.
- **`compare` checking a reading's take by digest,** in place of rate and length: a small follow-up
  once task 1 lands.
- **The session record's shape:** the workbench design's.

[`interpret`]: ../../../crates/duet-dsp/src/reading.rs
