# duet songs task 3: placing takes on a song's bars

The build design for songs task 3 (temper `01a1f16d-a1da-7622-b0f9-e3cbe11f4a3c`). What a
placement *is* is ruled in [the songs design, §1 and §4](2026-11-17-songs-and-composition-design.md#4-how-a-take-maps-onto-the-bars)
(temper `01a1eb48` ruling 8, project decision 6, ruling 12). This document settles what that design
left to the build: the commands, the click, where an estimate comes from, and how each witness is
shown. It restates nothing ruled there; read §1 ("The song") and §4 first.

## Intent

A take is useful to a song only once duet knows where the song's bars fall in it: comps across
takes (variants-and-comps tasks 2 to 4) pick notes by bar, and cannot until a take is placed. Today
a song names no takes at all. This task gives the song its links to takes, three ways to place one
(by the click it was recorded against, by capture's estimate, or set by the owner), and has capture
say where its reading of a placed take disagrees with the song.

**Success:** the owner records a take against the song's click and it lands on the song's bars to
the frame, with nothing to measure; a take recorded elsewhere is placed by estimate the moment it
is linked or captured, and the owner can correct it by hand; and a capture of a placed take says
plainly where it hears a different tempo, meter or key from the song.

**Goals**
- `song` 1.0 gains `takes`, each link with its optional `placed`, in place (ruling 12).
- `duet song link`, `duet song place`, and `duet record --song`.
- Capture of a linked take places an unplaced or estimated link, and states each disagreement with
  the composition at the placement.
- `take forget` and `song snapshot` read the song's `takes`.

**Non-goals**
- `duet play --click`, or playing a song's parts as a backing while recording. Considered and
  dropped by the owner (2026-11-21): the click is only for the player to keep time by.
- A take's own tempo map (a drifting take). One tempo for the whole take in 1.0 (§4).
- Unlinking a take from a song. Not asked for; a later task if wanted.
- Choosing an output device for the click. The system's default output is used.

**Anti-goals**
- The click is never written into the take. The take file holds only what the input heard.
- Capture never changes the composition, and never moves its estimate to agree with the song (§4).
- A `click` or `set` placement is never replaced by an estimate (ruling 8).

## Approaches

**How `duet record --song` keeps time** (the owner picked A, 2026-11-21):

- **A. duet plays a short tick on the audio output in the same run as it records the input.**
  The offset is worked out from the times the system stamps on each input buffer (capture) and
  output buffer (playback), so it is exact to the frame, with nothing to measure. **Picked.**
- B. The click as a MIDI part, through the SoundFont or a port. Reuses `duet play`, but a DAW
  port's latency is unknown, so the offset would be a guess.
- C. Record the click acoustically and detect it. Measures real latency, but needs speakers, and
  the click bleeds into the take.

**Reconciling drift between separate input and output devices** (the owner picked A, ruling 17):

- **A. Write the drift on the placement as `clock_ppm`,** applied wherever the take is mapped onto
  bars; the take stays what the input heard. **Picked.**
- B. Resample the take to the click's clock. No new field, but the stored take is no longer what
  was recorded, against a take's identity being its digest.

**How a take gets onto a song** (the owner picked link plus place, 2026-11-21):

- **`duet song link` adds the link; `duet song place` changes a placement.** `--for` sits on
  `link`, and "place" always means a placement the owner states. **Picked.**
- One command, `duet song place` with no flags meaning "link and estimate". One command fewer, but
  a flagless `place` quietly meaning "estimate" reads badly.

## The design

### The document: `song` 1.0's `takes`

As §1 draws it, in place under ruling 12:

```json
"takes": [
  { "take": "wayfaring", "digest": "sha256:9f2c…", "for": ["melody"],
    "placed": { "by": "estimated", "bpm": 88.6, "bar": "1.1.0", "at_secs": 1.057 } }
]
```

- **`takes`** is written always, possibly empty, and read as empty when absent, so every song
  written before this task still reads.
- **`placed`** is absent when unplaced. `by` is `click`, `estimated` or `set`. `bpm` is present for
  `estimated` and `set` and absent for `click` (the composition's tempo map is the take's tempo).
- **`clock_ppm`, on a `click` placement only** (ruling 17, temper
  `01a1fe65-a864-72ad-9543-36d81a90cb7a`): how far the take's clock ran from the click's, in parts
  per million, positive when the take's ran fast. A moment `t` seconds after `bar` on the click's
  clock falls `at_secs + t × (1 + clock_ppm / 1 000 000)` seconds into the take. Absent means no
  drift was measured or applied. Refused on `estimated` and `set`.
- **Checks a schema cannot make:** one link per take name and per digest in a song; `bar` is a
  position the composition's bars hold, written as it falls (as `Composition::check` does for every
  other position); `at_secs` is finite and at least 0; `bpm` is a tempo; `for` names each part once,
  and each is a part's name.
- **`for` may name a part not yet written.** The usual order is record, capture, then the reading
  becomes the part, so a link usually names its part before `parts/` holds it. `duet song show`
  states a `for` that names no part in `parts/`; it does not refuse the song, unlike a placement of
  a missing part. *(Builder's call, below.)*
- **An estimated `bpm` is the measured tempo,** `60 × rate / timing.beat_period_frames`, not the
  whole number `timing.tempo_bpm` states: capture places notes by the period, and a link placed by
  the rounded tempo would drift off the bars (92.4 placed as 92 is about 0.2 s off by bar 17).
- **Round trip:** a song with links and every kind of placement round-trips byte for byte, in the
  style of `crates/duet-midi/tests/roundtrip.rs` (CLAUDE.md). The schema `song/1.0.json` and the
  committed example gain `takes`.

### `duet song link <song> <take> [--for <part>]…`

- Adds the take, by its project name and digest, to the song's `takes`. `--for` may repeat.
- **On a take already linked,** `link` adds the `--for` parts given to the link, and changes nothing
  else; with no `--for`, it says the take is linked already and changes nothing. This is how a take
  linked by `record --song` or `place` gets its parts.
- **Places it `estimated` at once when the take has been captured:** `bpm` the measured tempo and
  `at_secs` the downbeat (`timing.downbeat_frame` over `rate`), from the capture's analysis; `bar`
  `1.1.0`.
  Otherwise the link is unplaced, and duet says the next capture places it.
- **Which capture:** of the analyses in `takes/<take>/` whose `take` digest is the take's, the most
  recently written. Duet names the one it used. *(Builder's call, below.)*
- Refused, naming why: a take not in the project, or a `--for` that is not a part's name. A refusal
  leaves the song unchanged.

### `duet song place <song> <take> …`

- **`--click --bar B --at S`** places it `by: click`: recorded elsewhere against a click of the
  song's tempo map. `--bpm` with `--click` is refused (the map is the tempo).
- **`--bpm`, `--bar`, `--at`, any of them, without `--click`** sets it, `by: set`. Every flag not
  given keeps the link's current value (its estimate, or a prior set), and the whole mapping is
  then `set`, as §4 rules. With no current value for a flag not given (an unplaced link), that flag
  is required, and its absence is refused by name.
- **On a take not yet linked,** it links it too (no `--for`), then places it.
- States the placement it replaced and the one it wrote.

### `duet record <take> --song <song>`

In a project only. Records the take as `duet record <take>` does today (#94: into
`takes/<take>/<take>.flac`, added to the project), and in the same run:

- **Opens the default output device and plays a click** at the composition's tempo map and meter:
  one bar of count-in in the song's first meter, then on from bar `1.1.0`, the downbeat of each
  bar accented. The click is synthesized by duet; it is never written into the take.
- **Works out where bar 1 fell in the take** from the two streams' timestamps: the playback time of
  bar 1's tick, less the capture time of the take's first frame, as a whole number of frames at the
  take's rate. That is `at_secs`.
- **Measures the drift between the two clocks** (ruling 17): how far the input's frames, as the
  input's timestamps place them, ran from the output's over the take, in parts per million.
- **Links the take to the song and writes `placed: { by: click, bar: "1.1.0", at_secs, clock_ppm }`**
  when the take is written, alongside the take's addition to the project. The drift is always
  stated; **`--no-reconcile`** leaves `clock_ppm` out of the placement.
- **Says at the start** that a click from speakers will be heard in the take, and that headphones
  keep it out. It does not refuse.
- **Warns at the start when the input and output are different devices,** that their clocks may
  drift and duet will measure it.
- **`--count-in <bars>`** is not added; one bar is fixed for this task.
- Refused before any device is opened: `--song` outside a project, a song the project does not
  have, a take name in use (as today).

### Capture of a linked take

When `duet capture melody|rhythm` reads a take that one or more songs link (by digest), for each
such song:

- **An unplaced link is placed `estimated`** from this capture, and duet says so.
- **An `estimated` link is re-placed** from this capture, and duet says what moved. *(Builder's
  call, below.)*
- **A `click` or `set` link is left as it is.** Never replaced by an estimate (ruling 8).
- **It states each disagreement** with the composition at the placement's bar, in §4's words:
  "tempo estimated 88.6 bpm; wayfaring-stranger is 92 at bar 1"; "key estimated A minor; the song
  is G minor at bar 1"; and a meter that differs, the same way. For a `set` link it also states a
  tempo that disagrees with the set `bpm`. Agreement is not stated. Tempos disagree when the
  measured tempo is more than 0.5 bpm from the song's; one near half or double the song's is said
  to be so. *(Builder's call, below.)*
- It never writes the composition, and never changes its own estimate. The statements go where
  capture's other statements go (stderr and the part's `statements`).

### Elsewhere

- **`duet take forget`** names every song whose `takes` links the take, alongside the parts comped
  from it that it names today (`take.rs` `linked_by`).
- **`duet song snapshot`** fills `snapshot.duet`'s `takes` with the song's links, by name and
  digest (spec build task 4 left it empty for this task).
- **`duet song show`** lists the song's takes and each placement.
- **The journal.** Every write of `song.duet` already journals a `doc.written` through
  `ctx.wrote`; link and place add no event kind of their own. *(Builder's call, below.)*

## Witnesses

Each is authored in the build and shown to fail against `main`'s binary (CLAUDE.md: build `main` in
a separate checkout with its own `CARGO_TARGET_DIR`, run with `DUET_E2E_BIN`).

1. **A take recorded with `--song` has its offset to the frame.**
   - The offset and drift arithmetic (buffer timestamps to frames and ppm) has unit tests with a
     fake clock: known capture and playback instants give a known whole frame count and drift.
   - **The speakers-on check** (owner, 2026-11-21): an e2e test in `crates/duet-e2e` runs
     `duet record <take> --song <song> --seconds 6` with the click played aloud, so the input
     hears it. It checks the placement is `by: click` at `1.1.0`, then finds the ticks in the take
     (duet's own onset detection) and checks bar 1's tick falls within 2 ms of `at_secs`, and a
     later bar's within 2 ms of where `at_secs` and `clock_ppm` put it. It needs a quiet room and
     an input and output device, so it is opt-in (an environment variable), and skips, saying
     why, without one, as on CI's Linux runners (as the unce sample's play check does, #122). It
     runs for real on the owner's Mac mini, and the PR states its result there.
2. **A take with no click is placed `estimated` with capture's tempo, and `duet song place --bpm`
   makes it `set`:** e2e, on a committed FLAC fixture; the estimate's `bpm` equals the analysis's
   `timing.tempo_bpm`, and after `place --bpm 90`, `by` is `set` with `bpm` 90 and the other values
   kept. A later capture leaves the `set` placement as it is.
3. **A capture of a placed take states a tempo that disagrees with the song, and changes neither:**
   e2e; the song's bytes and the analysis's tempo are unchanged, and the statement names both
   tempos.
4. **The song document round-trips with `takes` and `placed`** (round-trip test, CLAUDE.md).

## Constraints

- The format promises exact round-trips (CLAUDE.md); `takes` is additive in place under ruling 12.
- Tests that run the binary go in `crates/duet-e2e`.
- `record.rs`'s device code runs only on a machine with devices; what can be tested without one
  (the click's schedule, the offset arithmetic, every refusal) is.
- **The offset is as exact as the host's stream timestamps.** cpal does not promise that input and
  output timestamps share an origin on every host; on CoreAudio both are host time. On a host other
  than macOS, `record --song` says the offset is as the system reports it, unchecked there. The
  owner does not record through Bluetooth, whose reported latency is often wrong.

## Left to the builder

These are calls made here, for the owner to check; a sensible builder could settle each without
changing what gets built.

- **Which capture an estimate is read from** when a take has several: the most recently written
  analysis whose digest is the take's, named when used.
- **Capture re-places an `estimated` link** from each new capture, and says what moved. §4 rules
  only that a `set` mapping is never replaced; an estimate following the latest capture is read as
  allowed.
- **No journal event kind for linking or placing:** `doc.written` covers it, as for every other
  write that is not a status move.
- **The click's sound:** a short synthesized tick, the bar's downbeat higher or louder.
- **The count-in:** one bar of the song's first meter.
- **`placed.bar` from `record --song` is always `1.1.0`;** starting a recording at a later bar is
  not in this task.
- **`for` may name a part not yet in `parts/`,** stated by `song show`, never refused; `link` on a
  linked take adds `--for` parts.
- **A tempo disagreement** is more than 0.5 bpm, comparing the measured tempo (not the rounded one).
