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
  output buffer (playback), so it is worked out to a whole frame from the stamps, with nothing
  of the room to measure; the stamps themselves are corrected by calibration (ruling 18) and
  leave out some drift (Constraints). **Picked.**
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
  have, a take name in use (as today), and a calibration file that cannot be read (naming its
  path).
- **Applies the pair's calibration** (ruling 18, `01a20574-7729-7112-a582-79f8c4bb5c06`): when one
  is stored for this input and output at these rates, `at_secs` is the timestamps' offset plus the
  calibration's, still a whole number of frames. It says at the start which calibration it applies
  and when it was measured. With none, it says the placement is as the devices report their
  latency, which can be off by milliseconds, and names `duet record --calibrate`. It does not
  refuse.

### `duet record --calibrate [--device <input>]`

Ruling 18: the offset between a device pair is measured, because what an ordinary microphone
reports about its latency cannot be relied on. Inside or outside a project.

- **Plays duet's click** (the same voice as `--song`) from the default output, after a lead of
  1 s: 60 bpm, 12 ticks, every one the accented downbeat sound. A placement is anchored on bar 1's
  downbeat, and an input may delay the two pitches differently (the owner's webcam: 1.3 to 1.6 ms),
  so calibration measures the sound it anchors on. It records the default input (or
  `--device`) at the same time, into memory. Nothing is written to a take.
- **Finds each tick in the recording.** It is searched for within 400 ms either side of where the
  stream timestamps (with the drift fitted, as `--song` does) put it. The tick is located at the
  loudest sample in that search, heard only when it exceeds 8 times the floor (the median magnitude
  of the recording's first 0.5 s). The heard ticks are not timed one at a time by a threshold: on
  the owner's webcam each tick arrives as a weak burst and a stronger one about 6 ms later, and
  every per-tick rule tried (a first crossing of 8 times the floor, half the peak) flips between
  the two from tick to tick. Instead:
  - **Align.** Each heard tick's segment, from 40 ms before its loudest sample to 60 ms after, is
    cross-correlated against the mean of the segments and moved to its correlation's peak (to a
    fraction of a frame), the mean is taken again, and so on until no segment moves (at most 8
    passes). Each pass's median move is taken back off every segment, so the mean stays where the
    loudest samples put it. This times each tick against the others by its whole response.
  - **Time the mean once.** The ticks' onset is the first sample of the aligned mean, from its
    segment's start to its loudest sample, whose magnitude exceeds 8 times the mean's own floor
    (the median magnitude of its first 10 ms, ahead of the tick's rise, so averaging's quieter
    noise counts).
  - **Each tick's error** is that onset placed back through the tick's alignment, less where the
    timestamps put it.

  (A first crossing anywhere in the 400 ms search is not used: on the owner's webcam a quieter
  sound precedes each tick by 45 to 400 ms, likely the input's own processing.) The segment's
  40 ms lead leaves 10 ms of floor ahead of a rise as long as 25 ms (the build-up fix round 1
  saw) and stays short of that pre-sound.
- **Stores the median error** when at least 10 of the 12 ticks are heard and their errors lie within
  2 ms of each other, for the pair, keyed by the input's and the output's names and rates. A new
  calibration of the same pair replaces the old. Each entry has the offset in ms, how many ticks
  were heard, their spread in ms, and when it was measured.
- **Says what it measured:** the pair, the offset, the ticks heard, their spread, and where it was
  written.
- **Refuses to store** when the floor is digital silence (the input heard nothing), when fewer than
  10 ticks are heard (the click may be going to headphones, or the speakers are off or too quiet),
  or when the errors spread wider than 2 ms (a noisy room, or an input whose own processing moves the sound). It says which, and stores nothing.
  Too few heard also says: if the click goes to headphones, hold one against the microphone.
- **The file** is `<user config dir>/calibration.toml` (`DUET_CONFIG_DIR`, or the platform's, as
  `duet.toml`), written whole by duet through a temporary file and a rename. duet writes it; a
  person does not need to.
- Refused before any device is opened: a take or file output with `--calibrate`, and `--calibrate`
  with `--song`, `--seconds` or `--no-reconcile`.

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

1. **A take recorded with `--song` has its offset worked out to a whole frame.**
   - The offset and drift arithmetic (buffer timestamps to frames and ppm) has unit tests with a
     fake clock: known capture and playback instants give a known whole frame count and drift.
   - **Calibration** has unit tests with no device: ticks placed in a synthetic recording at a known
     error, with noise and one tick missing, give that error within 0.05 ms; too few ticks heard,
     or a spread past 2 ms, stores nothing; the file round-trips, replaces an entry by its key, and
     misses a pair recorded at other rates.
   - **The speakers-on check** (owner, 2026-11-21; ruling 18): an e2e test in `crates/duet-e2e`
     runs `duet record --calibrate`, then `duet record <take> --song <song> --seconds 10` with the
     click played aloud, so the input
     hears it. It checks the placement is `by: click` at `1.1.0`, then finds the ticks in the take
     (duet's own onset detection) and checks bar 1's tick falls within 2 ms of `at_secs`, and a
     later bar's within 2 ms of where `at_secs` and `clock_ppm` put it. It needs a quiet room and
     an input and output device, so it is opt-in (an environment variable), and skips, saying
     why, without one, as on CI's Linux runners (as the unce sample's play check does, #122). It
     runs for real on the owner's Mac mini, and the PR states its result there.
2. **A take with no click is placed `estimated` with capture's tempo, and `duet song place --bpm`
   makes it `set`:** e2e, on a generated WAV fixture; the estimate's `bpm` is the measured tempo,
   `60 × rate / beat_period_frames` (not the rounded `timing.tempo_bpm`), and after `place --bpm 90`, `by` is `set` with `bpm` 90 and the other values
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
- **Uncalibrated, the offset is only as exact as the host's stream timestamps.** cpal does not
  promise that input and output timestamps share an origin on every host; on CoreAudio both are
  host time. On a host other than macOS, `record --song` says the offset is as the system reports
  it, unchecked there. On the owner's Mac mini the ticks are heard a constant 7.6 to 8.5 ms before
  where the timestamps put them: cpal 0.18.2 counts each side's buffer and safety offset twice and leaves out the
  output stream's latency, and the webcam input under-reports its own. Ruling 18 corrects this by
  calibration; there is no coreaudio-rs dependency and no `unsafe`. The owner does not record
  through Bluetooth, whose reported latency is often wrong; calibration's 400 ms search covers it.
- **`clock_ppm` leaves out part of the drift, so a placement holds near the start of a take.**
  `clock_ppm` is the drift the host's stamps show. On the owner's Mac mini, with the built-in
  output and a USB input, the ticks heard in a take drift about 25 ppm more than the stamps do
  (their errors rise about 0.026 ms a second, in calibration and in takes alike), and `clock_ppm`
  reads about +4.3, so about 25 ppm goes unmeasured. A reader that maps a take with `at_secs` and
  `clock_ppm` stays within 2 ms of a heard tick for roughly the first 80 s, and is about 4.5 ms off
  at 3 minutes. The speakers-on witness checks only bar 3, about 5 s in, so it cannot see this.
  Nothing yet reads `clock_ppm` to place notes. A follow-up (measure the drift from the heard
  ticks, or check a late bar of a 60 to 120 s take) is filed before comps rely on a late bar.

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
  linked take adds `--for` parts. `link` on a linked take that is not placed also places it from
  the latest capture, if the take has one, and says so, as `link` does for a new link; a placed
  link is left as it is.
- **`--bar` reads a bare number as that bar's first beat:** `--bar 9` is `9.1.0`, as the songs
  design writes it.
- **`song place --at` (or `--bar`) on a `click` placement is refused**, since a click placement
  has no bpm to keep and a `set` needs one: the refusal names `--click --bar B --at S`, which moves
  it and keeps it a click placement, and `--bpm`, which sets it.
- **A tempo disagreement** is more than 0.5 bpm, comparing the measured tempo (not the rounded one).
- **Calibration's numbers** (ruling 18 leaves them open): 60 bpm, 12 ticks, a 1 s lead, a 400 ms
  search, 10 ticks heard and a 2 ms spread to store, keyed by device names and rates.
- **The placement does not record whether it was calibrated.** `record --song` says it; the song
  document is unchanged. A later task can add it if a reader needs it.
