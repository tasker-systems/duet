# `duet capture disagree`: the places two readings of a take disagree, found by kind, without listening through

**Date:** 2026-11-20. **Status: Proposed.** Nothing here is ruled or built. Each of the task's open
questions is answered below with a proposal and its reasons. What needs the owner's ruling is listed
apart, in [For the owner](#for-the-owner).

**Task:** temper `01a1e156-0e70-77b5-9d45-879d660205e8`. It comes from the Blood Sport listening
(temper `01a1e077-f1cc-7252-87f9-2601aacf5356`), and its places are raw material for
[arrivals and ornaments](2026-11-16-arrivals-and-ornaments-design.md) (temper
`01a1e0bc-6af4-73b7-8b86-fee8db27a620`).

**Builds on:**
- [The variants and comps design](2026-11-17-variants-and-comps-design.md):
  - ruling 1, "each reading stays its own `capture-analysis` document";
  - the variant annotation (`take`, `reader`, `passes`, `capture-analysis` 1.13);
  - the **seam**, its §3, ruled: "a frame where no note of either reading is sounding across".
- The moments file and `duet capture review` (`crates/duet-actions/src/capture/review.rs`), which
  replaced `scripts/duet-listen.sh`. The task body still names `duet-listen`. Its moments format is
  the one `review` reads today.
- The readers, `crates/duet-dsp/src/hmm.rs` (the note HMM) and `crates/duet-dsp/src/reading.rs`
  (`read`, the rules), and their shared passes (`interpret`: breathy, rejoin, held-pitch).

**Grounding:**
- The code at `main` `8524124` (`capture-analysis` 1.13, duet 0.3.0, release build).
- `crates/duet-capture/corpus/wayfaring-stranger.flac` (CC BY 4.0), captured with nothing given,
  once with `--reader hmm` and once with `--reader rules`.
- `crates/duet-capture/corpus/synthetic-melody.flac`, read the same way with `--tempo 120`.
- A throwaway synthetic take with seven pitch shapes, read with `--tuning 440 --tempo 120`.
- The measuring scripts were throwaway Python over the two `.capture.duet` files. They are not
  committed. Their rules are the ones proposed in [question 3](#3-what-counts-as-a-split).
- The Blood Sport takes are licence-restricted. They are not in this worktree, and nothing here was
  measured on them ([Not measured](#not-measured)).

## What the take shows

**Since the held-pitch pass, the two readers mostly agree on keys.** Today's "Wayfaring Stranger" is
read as 38 notes by the HMM and 37 by the rules. The two readings make 29 stretches between seams:

| | stretches |
|---|---|
| the same notes, each boundary within 30 ms | 9 |
| the same keys in the same order, but a boundary more than 30 ms apart (up to 0.15 s) | 16 |
| **different notes: the places** | **4** |

**The four places:**

| window (s) | HMM | rules | the place |
|---|---|---|---|
| 0.91–1.79 | D#3 | D#3, E3 | **split:** the rules split the HMM's D#3 at the turn (1.42–1.61) |
| 4.64–5.21 | F3 (4.79–5.06) | A3 (4.79–4.88) | **key:** the rules keep the approach from above and leave the F3 unread |
| 11.80–13.51 | A3, A#3, C4 | A3, A#3 | **only:** the HMM's C4 at 13.22, which the rules read as no note |
| 20.03–20.46 | D3 | none | **only:** the HMM's D3 at 20.18 |

- **They are the moments the arrivals design named.** The turn at 1.42, the rules' approach from
  above at 4.79, and the C4 at 13.22 and D3 at 20.18 that the rules miss.
- **The four windows hold 3.6 s of the take's 22.1 s.** The listen they ask for is a sixth of the
  take.
- **The 16 boundary-only stretches are the readers' different note starts.** The rules start a
  note earlier than the HMM does, by up to 0.15 s. These stretches have one key, and the same key,
  under both readers.

**The synthetic melody:** both readers read the same 8 notes, and nothing is found.

**A synthetic take with seven shapes** (each a sung-like harmonic tone on G3, apart by silence):

| shape | HMM | rules | found |
|---|---|---|---|
| held 0.6 s on the G3/G#3 midpoint (+50 cents, vibrato ±8 cents at 5 Hz) | G3 +44c | G#3 −49c | **key** |
| a turn inside a held G3: up to A3, held 0.06 s, back, with no onset | G3 | G3, A3, G3 | **split** |
| a rise from a whole tone below over 0.3 s, then held | F3, F#3, G3 | F#3, G3 | **split** (of the rules' F#3) |
| held at +160c, then at 0 (two levels, the upper first) | as rules | | nothing |
| an approach from a major third above, 0.09 s | as rules | | nothing |
| sinking from +45 to −45 cents | as rules | | nothing |
| two levels near the midpoint (+55, then +40) | as rules | | nothing |

- **A key disagreement on one note comes only from the midpoint.** Both readers key a note by the
  median of its held frames (`ReadNote::held_at`). So the key differs only where the two medians
  fall either side of a midpoint. That happens where the readers hold different spans, as at 4.79,
  or where the HMM's median runs below the rules'.
- **The band is narrow.** Across sung offsets of +40 to +56 cents, the readers disagree only at
  +50 and +51. The HMM's centre runs about 6 cents below the rules' on this shape: at +50 it reads
  G3 +44.4c, and the rules read G#3 −49.4c. Why the HMM's median is lower here was not looked into.
  That is outside this task.

**The cost of a second reading is small.** On "Wayfaring Stranger" (22.1 s), a whole release-build
capture takes 86.7 ± 0.6 ms with the HMM and 80.2 ± 0.5 ms with the rules (hyperfine, 15 runs). The
performance files the two runs write are byte-identical. So the pitch track is deterministic: a
second capture of one take reads the same performance.

**Two runs of today's default capture are byte-identical, but for one field.** Two runs give the
same `.abc`, `.capture.duet`, `.performance.duet.jsonl`, stdout and stderr. In the `.duet`, the
part's `id` differs, because it is minted fresh (`fresh_id`, `crates/duet-actions/src/context.rs`).
This decides how "exactly as today" is proved (below).

## 1. Is the second reading kept?

**Proposed: yes, as its own `capture-analysis` document, the one `duet capture melody --reader
rules` already writes. The format does not change. What the disagreement yields is a moments file,
and nothing is added to `capture-analysis`.**

- **Variants ruling 1 settles the first option.** "Each reading stays its own `capture-analysis`
  document" was ruled 2026-11-17. An alternative reading inside the analysis, as a minor, would be a
  second way to hold a variant.
- **Keeping only the disagreement is not enough for the owner's next two steps:**
  - `duet capture review` shows the readings side by side at each place, and needs each reading as
    an analysis (`--reading NAME=ANALYSIS`);
  - a comp pick at a place ("4.79: HMM") needs the other reading as a variant to splice from.
- **A second capture makes a well-formed variant by construction.**
  - Its timing, key, take and passes are its own.
  - A second reading made inside the first capture would carry the first's timing and key. Those
    are estimated with the first reader's notes among their events (`capture.rs`, where the
    reading's notes extend `events`), so they would state something not measured.
- **Its cost is the pitch track run again:** about 80 ms on a 22 s take, against about 6.5 ms for
  the reader alone. It is paid only when the owner asks, never by the default capture.
- **The other way is one capture that reads both.** It would read the other reader from the same
  kept frames and write it as a second variant. It saves the pitch track, and it needs a decision
  this task does not make: whose timing and key a variant states when another reader's notes
  estimated them. **Owner question A.**

**In a project** the two readings are already in place: `takes/TAKE/TAKE.hmm.*` and
`takes/TAKE/TAKE.rules.*`, each made by `duet capture melody TAKE` with its `--reader`.

## 2. A flag, a step, or part of `compare`?

**Proposed: a step over two existing readings, `duet capture disagree`. It is not a capture flag,
and it is not part of `compare`.**

```text
duet capture disagree TAKE [-o STEM] [--reading NAME=ANALYSIS] [--moments FILE]
```

- **What it reads:**
  - the stem's own `.capture.duet`, named after its reader, as `review` names it;
  - one `--reading`, the other analysis;
  - in a project, with no `-o` and no `--reading`, the take's two readings in `takes/TAKE/`. It is
    refused, naming them, when the take has fewer or more than two.
- **What it writes:**
  - a moments file: `STEM.disagree.moments`, or `--moments FILE`. It is new, and refused if it
    exists, as every capture command refuses;
  - standard output: one line per place, then the summary line (`4 places: 1 split, 1 key, 2 only;
    16 stretches differ only in where notes start or end, by up to 0.15 s`);
  - "nothing found" when there is no place, with the boundary-only count. A moments file with no
    moment would be refused by `review`, so none is written.
- **What it refuses, before anything is written:**
  - **two readings of different takes.** Their `take.digest` differ, or either has none
    (a reading from before 1.13). Each is named.
  - **two readings on different tunings,** such as one with `--tuning` given and one estimated. A
    key is only comparable on one tuning. The refusal names both tunings, and the `--tuning` that
    makes them one.
  - **a reading named twice,** as `review` refuses it.

**Why not a capture flag:**
- The criterion "a capture run without asking behaves exactly as today" is then true by
  construction: `duet capture melody` is not touched.
- The routine use stays one melody from one reading.
- A flag would also need the second reading made inside the capture, which is owner question A.

**Why not part of `compare`:**
- `compare` renders one window of one reading, and writes an SVG and a WAV every run.
- The disagreement is a listing over the whole take, with no rendering.
- `review` already shows both readings and plays each place, so a third renderer is not needed.

**The places are found by a pure function in duet-dsp,** beside `interpret`:
`disagreements(a: &Reading, b: &Reading) -> Vec<Place>` (`crates/duet-dsp/src/disagree.rs`). The
command loads, refuses and writes; the function is unit-tested on readings built by hand.

**The seam is built here.** The comp design ruled the seam, but comp task 2 is held, so nothing
computes seams yet. This task builds `seams` in duet-dsp, where comp task 2 can use it.

## 3. What counts as a split

**Proposed:**

- **A place lies inside one stretch between seams** (variants §3, ruled). Inside a stretch, every
  note of each reading starts and ends there, so the stretch holds the whole disagreement and
  nothing else.
- **A stretch is a place when its keys differ.** A stretch whose two readings hold the same keys in
  the same order is no place, whatever its boundaries. Its count, with the largest boundary
  difference, goes in the summary line. **Owner question B.**
- **Two notes sound together** when at least half of the shorter one overlaps the longer.
- **Each note of either reading, in a place, is one of these kinds:**

  | kind | the note | what it means |
  |---|---|---|
  | `split` | sounds together with two or more of the other reading's notes, and each of those sounds together with no other note of its own reading | the other reader read it as several notes: a candidate ornament, or a slide read as notes |
  | `key` | sounds together with exactly one of the other's notes, on another key, and that note with only it | the readers disagree on which key was sung |
  | `only` | sounds together with none of the other's notes | one reader reads a note where the other reads none |
  | `other` | anything else, such as two overlapping chains | listed, so nothing is dropped; none was found on either take |

- **A place is stated once.** A split is named by the note that is split, and its pieces are not
  listed again. A key pair is named once, by the first reading's note.
- **A split always says whether the split note's own key is among its pieces.** "D#3 read as D#3,
  E3" is an ornament around a held key. "F#3 read as F3, F#3, G3" (the rise) is not.

**There is no overlap tolerance to set.** Half the shorter note is a share, not a time, so it scales
with the notes:
- the rules start notes up to 0.15 s earlier than the HMM does, and the share ignores that, where
  a fixed 30 ms tolerance classed two of the four places on "Wayfaring Stranger" (1.42 and
  13.22) as `other`;
- on both takes, a share from 0.3 to 0.7 finds the same places with the same kinds.

**There is no shortest split either.** Each piece of a split is a note of its reader, and each
reader already reads no note shorter than `SHORTEST_NOTE_SECS`, 0.05 s (`notes.rs`, used by both).
A higher floor would hide the ornaments the places are for:
- the E3 in the turn at 1.42 lasts 0.07 s;
- Blood Sport's turn A#3 lasts 0.10 s.

**The comparison is symmetric.** Swapping the two readings finds the same places with the readers'
names swapped (checked on both takes).

## 4. How the places reach the listening

**Proposed: as a moments file in today's format, which `duet capture review` reads unchanged.**

```text
# disagree: hmm (ws.capture.duet) against rules (ws-rules.capture.duet), take sha256:18148cfc…
# hmm: D#3 | rules: D#3 E3
0.91 1.79 split: hmm D#3 at 1.13 is rules D#3 E3
# hmm: F3 | rules: A3
4.64 5.21 key: hmm F3 at 4.79, rules A3
# hmm: A3 A#3 C4 | rules: A3 A#3
11.80 13.51 only: hmm C4 at 13.22
# hmm: D3 | rules: none
20.03 20.46 only: hmm D3 at 20.18
```

- **The window** is the stretch, with 0.15 s either side, kept within the take. `review` refuses a
  moment that ends past the take.
- **The kind is the label's first word,** followed by a colon. The comment line above each place
  holds both readings' notes in the stretch, as `wayfaring-stranger.moments` does by hand.
- **The format does not change.** `review` shows the label above each moment's columns. A test that
  needs the kind reads the first word. **Owner question C** asks whether the kind should be a column
  of its own: that would be a format change, additive, with its round trip.
- **Tried on `main` with the file above, unchanged:** `duet capture review
  wayfaring-stranger.flac -o ws-hmm --moments ws.disagree.moments --reading rules=ws-rules.capture.duet`
  wrote "4 moments, 2 readings (hmm, rules), 42 clips".

**So the owner's whole path is three commands:**
```sh
duet capture melody take.m4a --reader rules -o take-rules   # the second reading, when asked
duet capture disagree take.m4a --reading rules=take-rules.capture.duet
duet capture review take.m4a --moments take.disagree.moments --reading rules=take-rules.capture.duet
```
In a project, the first two are `duet capture melody TAKE --reader rules` and `duet capture disagree
TAKE`. `review` in a project reviews one reading; joining the last two is named for later
([Not decided here](#not-decided-here)).

## "Exactly as today": how the build proves it

`duet capture melody` is not changed, so the proof is that it still does what it did. It has three
parts.

1. **A golden e2e test,** in `crates/duet-e2e`, committed in the build PR's first commit, before
   any other change:
   - it captures the witness take (below) and `synthetic-melody.flac` with nothing asked. Both are
     MIT. "Wayfaring Stranger" is left to its listened corpus test: a golden of its readings would
     commit data derived from a CC BY take, with its notice to carry;
   - it compares against goldens written by `main`'s binary (`DUET_E2E_BIN`, as CLAUDE.md says),
     beside the existing ones in `crates/duet-e2e/golden/`: every file written (`.duet`, `.abc`,
     `.capture.duet`, `.performance.duet.jsonl`), byte for byte, and stdout and stderr;
   - the one masked field is the part's `body.id`, which is fresh on every run;
   - the set of files written is exactly those four, so no `.disagree.moments` appears unasked.
   - It passes on `main`, and must keep passing. It is the one witness here that does not fail on
     `main`, by design.
2. **The diff:** in `crates/duet-actions/src/capture.rs`, the build PR adds only the subcommand to
   `CaptureCommand` and its line in `run`'s dispatch. It changes nothing in `duet-dsp`'s readers or
   passes. Review checks this; no gate enforces it.
3. **The time:** hyperfine, `main`'s binary against the branch's, on "Wayfaring Stranger", 15 runs
   each. The means must be within 3σ of each other (today σ ≈ 0.6 ms of 87 ms). The numbers are
   stated in the PR body. This is not a CI gate, since a shared runner's timing is noise.

## Witnesses the build authors

**All are declared holes.** Nothing below exists yet, and each one but the golden fails on `main`
because the command and the function do not exist.

**The witness take,** committed and generated by `scripts/capture-fixtures.sh` (arithmetic, then
`flac`, byte-stable for a given `flac` version, as the corpus's synthetic takes are). It is read with
`--tuning 440` and a given tempo, so no estimate moves it. It holds:
- a held G3 with a turn in its middle: up to A3, held 0.06 s, and back, with no onset;
- a G3 held 0.6 s at +50 cents with ±8 cents of vibrato at 5 Hz, the key disagreement;
- plain held notes, on their keys, where the readers agree.

**Its fragility is stated on the test:** the key case holds only where the two medians straddle the
midpoint, +50 and +51 cents on `main`. A reader change that moves either median by a few cents ends
the disagreement. The test's failure message names both readers' centres, so the cause is visible.
Unit tests on readings built by hand carry the classification exactly, and do not depend on it.

**W1. The witness take's places** (`crates/duet-e2e`): `duet capture disagree` on its two readings
finds exactly two places: one `split`, at the turn, naming the HMM's G3 as read by the rules as G3,
A3, G3; one `key`, at the midpoint note, G3 against G#3. Nothing is found on the plain notes.

**W2. Nothing where the readers agree:** on `synthetic-melody.flac`, the command says nothing is
found and writes no moments file.

**W3. "Wayfaring Stranger":** the four places of [What the take shows](#what-the-take-shows), with
their kinds: split at 1.13, key at 4.79, only at 13.22 and at 20.18; and 16 boundary-only stretches.
They are pinned as they read when the build lands, like the manifest's listened counts. A reader
change that moves them fails by name, and is ruled by ear, not by editing the count.

**W4. The classification, on readings built by hand** (`crates/duet-dsp/src/disagree.rs` tests):
- each kind, with its edge: a note overlapping another by exactly half the shorter sounds together
  with it, and one just under does not;
- a boundary-only stretch is no place, and is counted;
- a split is stated once, its pieces not again; a key pair once;
- swapping the readings swaps the names and changes nothing else.

**W5. Refusals, before anything is written:** two readings of different takes (by digest), a
reading with no `take`, two tunings, a name given twice, and a moments file that exists. Each is
refused by name.

**W6. The file reaches the listening:** `disagree` then `review` on the witness take writes a page
of two moments with both readings, and each moment's label starts with its kind.

**W7. The default capture is unchanged:** the golden test above.

## For the owner

Each question changes what gets built.

**A. Where the second reading comes from.**
- **(a)** a second `duet capture melody --reader rules` run, kept as its own variant. The pitch
  track runs again, about 80 ms on a 22 s take. Nothing new decides a variant's timing and key.
- **(b)** one capture that reads both readers from one pitch track and writes two variants. It
  needs a ruling on whose timing and key the second variant states.
- **Recommended: (a).** Its cost is small, it is paid only when asked, and every variant stays
  measured by its own reader.

**B. Boundary-only stretches.** On "Wayfaring Stranger", 16 stretches hold the same keys under both
readers, with a boundary up to 0.15 s apart.
- **(a)** they are no place, and only their count and largest difference are stated;
- **(b)** they are places of a fourth kind, `boundary`.
- **Recommended: (a).** As places they would be 16 of 20, and the list would no longer spare a
  listen through. The task's three kinds do not include them.

**C. Where a place's kind is written.**
- **(a)** the label's first word, so the moments format does not change;
- **(b)** a column of its own, an additive change to the moments format, with `review` reading it
  and its round trip.
- **Recommended: (a).** Nothing reads the kind but a person and a test.

**D. The command's name.** `duet capture disagree` is proposed. It sits under `duet capture` because
it reads captures, as `compare` and `review` do.

**E. By ear: are these the places worth listening to?** The task's premise is that where the readers
disagree the reading is least sure. Whether the four "Wayfaring Stranger" places are where the owner
hears the reading go wrong is the owner's to judge, through `review`. Nothing here confirms it.

## Not measured

- **Blood Sport and Damocles were not read.** The task's examples (the turn's A#3 at 33.40–33.50
  inside the HMM's G#3, and the splits around 8.0 and 9.1 s) come from the 2026-11-15 listening,
  before #48 and the held-pitch pass. Whether today's readers still disagree there, and with which
  kinds, is for a local check by the owner. The takes stay out of git, and so do their numbers.
- **The owner's ear** on any place (question E).
- **The `only` kind on a synthetic take:** no shape was tried that one reader reads and the other
  does not. W3 witnesses it on "Wayfaring Stranger". A synthetic one may be added if the build finds
  one.

## Build task

**One task, medium: `duet capture disagree`.**
- **Needs:** nothing. **Gate:** questions A to D ruled.
- **What:**
  - `seams` and `disagreements` in `crates/duet-dsp/src/disagree.rs`;
  - the command in `crates/duet-actions/src/capture/disagree.rs`;
  - the witness take in `scripts/capture-fixtures.sh`;
  - the README's capture section.
- **Witnesses:** W1 to W7, with the golden test (W7) as the first commit.
- **No format changes,** under C (a). Under C (b), the moments format's change and its round trip
  are added to this task.

## Not decided here

- **`review` taking two readings of a project's take,** so the last two commands become one. In a
  project today, `review` reviews the take's one reading, and refuses a take with two without `-o`.
- **A comp pick at each place,** such as a review verdict that writes a span. That is comp task 3's
  and the workbench's.
- **What a split means for the reading.** Whether the HMM should state the turn at 1.42 as an
  excursion is the arrivals design's. The moments file records that the rise to E3 is no excursion.
- **Why the HMM's median sits lower than the rules' on a note held at the midpoint.** It was seen on
  the synthetic shape, and not looked into.
