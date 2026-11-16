# `duet capture`: a gesture read where it arrives, and an ornament shorter than any note

**Date:** 2026-11-16. **Status:** **Ruled (owner, 2026-11-16).** Questions 1 to 5 are ruled, as
recorded in the owner's Claude Doc "duet: ornament and tuning rulings".

Two things are not ruled:
- **Four checks by ear** that the owner still owes (temper task
  `01a1e4a5-ce43-7159-86d5-9ea18dc5731f`). Each gates a build step, as *Gates* below says.
- **One question:** an ornament in the middle of a held note, against "a dip is two notes"
  (question 2). It is marked **Open** where it falls.

**Task:** temper `01a1e0bc-6af4-73b7-8b86-fee8db27a620`. It comes from the Blood Sport listening
(temper `01a1e077-f1cc-7252-87f9-2601aacf5356`).

**Builds on:** [the performance and its reading](2026-11-15-performance-and-reading-design.md). Three
parts of it matter here:
- the residual (question 3);
- the #28 ruling, "a slide belongs to the note it arrives at";
- the note HMM, which has been the default reader since #50.

The format changes follow the version policy in [duet documents](2026-11-15-duet-documents-design.md)
(question 3).

**Grounding:**
- The code at `d0fd967`.
- `crates/duet-capture/corpus/wayfaring-stranger.flac` (CC BY 4.0), read with both readers. The
  times below are seconds into that take, on its estimated tuning (A4 = 438.3 Hz, −7 cents).
- The Blood Sport moments are named, but nothing measured on that take is quoted here. Its licence
  keeps them out of git.

## What the take shows

"Wayfaring Stranger" is sung in a southern gospel inflection. The HMM reads 31 notes and the rules
read 38. The rules add nine notes:
- eight are held approaches (table 1);
- one is a turn inside a note (table 2).

The rules also miss two notes the HMM reads (13.22 s C4 and 20.18 s D3). At 4.79 s they keep an
approach from above (A3) and drop the F3 it arrives at. Five of the rules' ten between-keys notes are
among the nine they add.

**1. A held approach, then the arrival.** The voice holds a key a step below for 0.05–0.1 s, often
overshoots, then settles. This happens eight times.

| time (s) | approach, held | arrives on | rules | HMM |
|---|---|---|---|---|
| 2.28 | G#3, 0.07 s; overshoots to B3 −40 | A#3 | G#3, A#3 | A#3 |
| 3.77 | G#3, 0.05 s | A#3 | G#3, A#3 | A#3 |
| 5.17–5.52 | wobbles C3–D3–C#3–D3–E3 | D#3 | C#3 (between keys), D#3 | D#3, with a 0.2 s scoop |
| 9.07 | C3 +30, 0.08 s; overshoots to E3 −30 | D#3 | C3 (between keys), D#3 | D#3 |
| 12.03 | A3 +20, 0.09 s; overshoots to B3 −45 | A#3 | A3 (between keys), A#3 | A#3 |
| 13.84 | G#3, 0.06 s, after a leap up from C#3 | A#3 | G#3, A#3 | A#3 |
| 14.87 | F#3, 0.07 s; overshoots to A3 −25 | G#3 | F#3 (between keys), G#3 | G#3 |
| **18.56** | **F3 −35, 0.1 s** | **F#3, held 0.12 s** | **F3, F#3** | **F#3** |

- **18.56 s has the shape of Blood Sport's opening.** A flat F3 is held about as long as the F#3 it
  arrives at. At Blood Sport's opening the owner hears one F#3. Nobody has listened to 18.56 s yet
  (task D).
- **The HMM names the arrival every time.** It states the approach only as a long scoop, or as drift
  and rest. That is also how it states Blood Sport's turn at 33.3 s.

**2. Ornaments inside a held note.** Most of them move less than a semitone.

| time (s) | what the voice does | rules | HMM |
|---|---|---|---|
| 1.42–1.61 | turns on D#3: down to D3 +30, up to E3 −30, back | adds an E3 (between keys) | one D#3, rest 41c |
| 9.55–9.79 | dips from D#3 to C#3 +40 and back | two D#3 | two D#3 |
| 16.40–16.55 | rises from F#3 to G3 −25 and back | two F#3 | two F#3 |
| 19.27, 20.05 | leans up to E3 on D#3 | none | none |

**3. What the formats carry today:**
- `duet-abc` refuses every decoration except the dynamics, and refuses grace notes.
- `song` 1.0 holds only notes and controls.
- MIDI holds only notes and bends.

## 1. Where an ornament lives

**Ruled: as a part of the note's residual. It is measured, not classified.**

```text
NoteResidual {
  … as in capture-analysis 1.4 …
  excursions: Vec<Excursion>   // omitted when empty
}
Excursion {
  start, end: u64              // frames on the take's timeline, inside the note
  cents: f32                   // where it is held, in cents from the note's key
}
```

**What counts as an excursion:**
- It is a run of frames inside the note that holds somewhere other than the note's held pitch.
- Its frames spread no wider than a slide's frames may when they have arrived.
- It holds at least 3 frames (35 ms).
- It sits at least 50 cents from where the note is held. That is centre plus drift for an excursion
  inside the note, and the key for one before `held_from`.

**What does not count:**
- **A vibrato.** It is fitted first, as `modulation`, so its swing never counts.
- **Anything shorter than the floor of 3 frames** stays in `rest`.

**Blood Sport's "-eak" wobble is an ornament under this rule.** Its rise toward G#3 and its dip
toward F3 each hold longer than the floor (temper research `01a1e1fa-bcba-70db-82e7-4e6d4a4f1e94`).
Whether it is worth keeping is by-ear check 2, which gates the floor.

The constants are set when task A is built, on the synthetic witnesses, and named in `residual.rs`.

**The ornament's name is worked out when something is written. It is never stored.** Where an
excursion sits says what it is:
- before `held_from`, it is a **grace** (an approach);
- one inside the note, above or below, is a **mordent**;
- one above and one below, within 0.3 s, is a **turn**;
- three or more alternations are a **trill**;
- one under 70 cents is a **wobble**.

So the naming can change without a format change, and the reading holds only what was measured.

**`rest_cents` keeps its meaning.** It is still measured over every held frame, excursions included.
Taking the excursions out of it would change an existing value's meaning, which is a major version.

**The loss does not change.** It prices frames, not parts, so an excursion is not a way to lower it.

**How this fits the transformations task:**
- `excursions` is one more part of a note, beside centre, drift, modulation, slides and rest. A person
  keeps it or drops it note by note.
- `correct k` keeps excursions, as it keeps vibrato and slides: they are expression, not error.
- A performance render writes the excursions that are kept as bends. They are residual, so they
  round-trip exactly through the bend witness that task already names.

## 2. Which key a gesture takes

**Ruled: the arrival's.** #28 extends to this:

> **An approach belongs to the note it arrives at, held or not.**

- The held approach is kept as a grace excursion (question 1), so nothing sung is lost.

**The direction is ruled.** An approach is pitch that moves toward the note it reaches:
- from a step below or above;
- with no onset and no voicing dip between the two.

**The bound is not ruled, and is a named open parameter.** That parameter decides how long, or in
what shape, an approach may be held before it is a note of its own.
- **Time alone cannot set it.** Blood Sport's opening has an approach about as long as a sixteenth
  note at the take's tempo (temper research `01a1e1fa-bcba-70db-82e7-4e6d4a4f1e94`). A sung
  sixteenth is a real note, and has the same length.
- **The candidates:**
  - **the glide:** the approach keeps moving toward the arrival while it is held, where a sung note
    settles;
  - **a flat arrival:** the arrival is held below or above its key, toward the approach, as a reached
    pitch is;
  - **the length relative to the arrival,** rather than an absolute length.
- **The parameter is set after by-ear check 1,** and the set value is stated with its reason in the
  code.

**It is built as a merge pass over the reading, after decoding, not inside the HMM.**
- The pass takes two adjacent notes that the direction and the bound join. It makes them one note,
  on the later note's key, with `held_from` at the arrival. The earlier note's held span becomes a
  grace excursion.
- So the pass can be witnessed on its own, against any reading, and the decoder's prices are
  unchanged.
- It runs for both readers. The rules' held approaches (table 1) are its widest test.
- **Why it is needed:** the HMM, as of `d0fd967`, already takes in all eight approaches in table 1 as
  attack. But it reads Blood Sport's opening as two notes, F3 then F#3, with no voicing break between
  them (the temper research above).
- **What the loss does with it:** the merged note's grace frames count as slide in the loss, priced
  at `ATTACK_COST`, as the decoder prices an attack. The loss still scores any reading, merged or
  not. A merge that raises the loss is stated by the pass, not hidden.

**A real step stays two notes.** This clause comes from the held contour task. Its witness is set
where the bound is tight: two sixteenth notes at 108 bpm, a semitone apart and each on its key.

**Open: an ornament in the middle of a held note, against "a dip is two notes".** This needs a
ruling, which by-ear check 3 informs.
- On `main`, the HMM reads a synthetic turn as two notes, the same way it reads a dip: a held G#3
  that turns up to A#3 for 0.14 s, with no onset and no voicing dip.
  - The second G#3's attack takes in the turn.
  - Holding the turn's frames on G#3 costs about 8 nats a frame, against `c` = 4 once for a new note.
  - The rules read three notes: G#3, A#3, G#3.
- Blood Sport's turn read as one G#3 only because it sits at the note's start, where the attack takes
  it in.
- In "Wayfaring Stranger", both readers split the notes at 9.55 s and 16.40 s.
- The owner's ruling "a dip is two notes" covers a pitch that goes away and comes back. A turn does
  exactly that.
- **Options:**
  - **(a)** keep the ruling: an ornament in the middle of a note re-attacks it. Its excursion is then
    measured in the second note's slide, and is a grace there;
  - **(b)** narrow it: a departure that returns, with no onset and no voicing dip and held less than
    about 0.15 s, stays inside the note as an excursion. This changes the decoder, and is task E.
- **Recommended: (b).** The dip the ruling came from had a voiced consonant between two syllables,
  so a voicing drop or an onset marks it. A turn has neither.
- By-ear check 3, at 9.55 s and 16.40 s, tests the choice on a real take.

## 3. What each output writes

**Ruled: the score stays the melody, for now.**
- **`.duet` (the notes view) and `.abc`:** the reading's notes only, as today. No song change.
- **`.capture.duet`:** the excursions, in the reading.
- **`--render performance`** (transformations, task 7): the residual as bends, with the excursions
  kept and the ones dropped left out.
- **MIDI:** notes, plus bends in a performance render.
- **`compare`:**
  - stdout gains an excursions column;
  - the SVG marks each excursion;
  - the sine can play the ornaments (question 4).

**Named for later, and not filed until the owner asks: ornaments in the score.**
- `song` 1.1 gains an optional `Note.ornament`.
- The notes view gains a syntax for it.
- `duet-abc` writes and reads `~`, `T`, `M`, `P` and `{g}`.
- MIDI writes an ornament as bends only.
- Each of these is in *Formats* below.

## 4. How it is judged by ear

**Ruled, with two additions** (the Blood Sport moments and the sixteenth-note step).

**Moments:**
- **`crates/duet-capture/corpus/wayfaring-stranger.moments`** names the moments in both tables, with
  the take's licence.
  - Every listening tool can then cite the moments.
  - It is committed: the take is CC BY 4.0, and the file states the attribution.
- **Blood Sport's moments** (the opening, the turn, "-eak", the trill) are for local listening only.
  They stay in its untracked `fixtures/` and are never committed.

**`compare` plays a moment three ways:**
- **melody:** the keys only;
- **steps:** the keys, with each excursion played at its key, held for its frames;
- **bends:** the keys plus the residual.

**The owner rules each moment** through `duet-listen`. Does "steps" sound like what was sung? Is it
an ornament or a note? The rulings are recorded in Temper.

**Synthetic witnesses:** each is named under its build task.

## 5. The held contour task

**Ruled: cancelled, pointing at this task.** That task is temper
`01a1dd79-a389-7162-a2dc-bbf89b3382cf`.

Its moments are covered elsewhere:

| its moment | where it went |
|---|---|
| Blood Sport 33.3 s, the turn | question 1 |
| the opening after the dip | question 2 |
| 6.4 s and 11.4 s, between keys | the tuning work and the transformations task (snap to key, per-note correction) |

Its one live clause, "real semitone steps, quick and then held, are still two notes", is task B's
sixteenth-note witness.

## 6. Blood Sport's trill

**Ruled: nothing to change.** The HMM reads the trill (9.3–11.0 s) as five notes, each long enough
to be melody (temper research `01a1e1fa-bcba-70db-82e7-4e6d4a4f1e94`). So a trill held that long is
notes, not one ornament.

## Gates: the owner's checks by ear

These checks are tracked in temper task `01a1e4a5-ce43-7159-86d5-9ea18dc5731f`.

| check | what the owner listens for | what it gates |
|---|---|---|
| 1 | Blood Sport 0.56–0.82 s: one F#3? And is the G#3 after it a sung note? | task B's bound, and its opening-shape witness |
| 2 | Blood Sport "-eak", 34.66–34.93 s: an ornament worth keeping, or roughness? | task A's floor, which is 3 frames until then |
| 3 | "Wayfaring Stranger" 9.55 s and 16.40 s: does a new syllable start where both readers split the note? | the open question in question 2, and task E |
| 4 | Blood Sport's tuning | the tuning task, not this design |

## Build tasks

Each task is filed by the control plane. None is built before the one it needs.

**A. Excursions in the residual.**
- **Needs:** nothing. **Gate:** check 2 may move the floor after the build. The floor is a named
  constant, so moving it is a one-line change.
- **Witnesses:**
  - A synthetic G#3 that turns up to A#3 for 0.14 s at its start (the shape of Blood Sport's turn)
    reads as one note, with one excursion at +200 ± 10 cents, its frames within one of where it was
    synthesized. On `main` there is no excursion to read, so it fails.
  - A held approach of 0.1 s a semitone below, which the HMM takes in as attack, reads as one
    excursion before `held_from`.
  - A vibrato of ±40 cents at 6 Hz has no excursion.
  - A pitch held 2 frames, under the floor, is no excursion. One held 3 frames is.
  - A note with no ornament has none.
  - The loss is unchanged by the new part.
  - Every 1.4 example still reads.
- **Then:** once the owner has listened (task D), the "Wayfaring Stranger" listened entry gains an
  excursion count, with a tolerance.

**B. The arrival merge pass.**
- **Needs:** A, since the merged approach becomes a grace excursion.
- **Gate:** check 1. The pass and its direction witnesses are built first. The bound is set after the
  check.
- **Witnesses:**
  - **Direction.** Read the rules' reading of a synthetic approach: held 0.1 s, 35 cents flat, a
    semitone below, then the arrival held 0.12 s. On `main` it is two notes. After the pass it is one
    note on the arrival, with `held_from` at the arrival and one grace excursion.
  - **Only toward the arrival.** Two held notes whose pitch, between them, moves away from the
    second before it reaches it are not merged.
  - **Not across a break.** The same approach with an onset, or with a voicing dip, between it and
    the arrival is not merged.
  - **The bound is tight: the sixteenth-note step.** Two sixteenths at 108 bpm, G3 then G#3, each
    0.139 s on its key, then a held A#3, read as three notes. On `main` both readers read three, so
    this guards against the pass.
  - **The opening's shape,** once check 1 has ruled. An F3 held a sixteenth on its key, an F#3 held
    35 cents flat, then a G#3, reads as F#3 then G#3. No voicing break.
    - On `main` both readers read F3 then G#3: the short, flat F#3 goes into the G#3's attack. So
      this witness fails on `main`.
    - A pass can merge only notes the reader produced. So the witness's F#3 is held long enough that
      the HMM reads it, as it reads the real opening's (the temper research above).
  - **"Wayfaring Stranger" at 18.56 s** reads F#3 under both readers.
- **The pass states each merge,** on stderr and in the reading, as other statements are stated.

**C. `compare` shows and plays excursions.**
- **Needs:** A. **Gate:** none.
- It adds the stdout column, the SVG marks and the three renderings.
- **Witness:** a duet-e2e test on a synthetic turn checks two things. The stdout lists one excursion.
  In "steps", the WAV's right channel over the excursion's frames is within 1 cent of key plus
  offset.

**D. The moments, and the owner's listen.**
- **Needs:** C. **Gate:** none to build it. Checks 1 to 3 run on it.
- It commits the "Wayfaring Stranger" moments file. A draft is kept in the build notes.
- **Witness:** the owner's ruling on each moment, by ear.

**E. An ornament in the middle of a held note stays in the note.**
- **Not filed** until the open question in question 2 is ruled (b).
- **Gate:** check 3.
- **Witness:** a held G#3 that turns up to A#3 for 0.14 s in its middle, with no onset and no voicing
  dip, reads as one note with one excursion. On `main` the HMM reads two notes and the rules read
  three.

**Transformations task (temper `01a1df08-2b63-7658-8812-7823c97c6147`):** this design amends it in
two ways, and adds no task:
- `excursions` is a part a person keeps or drops per note;
- `correct k` keeps excursions.

## Formats

- **`capture-analysis`, additive (a minor): the minor after the tuning change's.** The tuning task
  takes the next minor first.
  - `NoteResidual.excursions` is a new optional field, omitted when empty. Its absence means what the
    minor before meant: no ornament stated.
  - The merge pass's statements are a new optional field of the reading in the same minor.
  - Task A adds the field; task B adds the statements.
- **`song`, `performance`, the envelope:** no change in tasks A to E.
- **The notes and grid views:** no change in tasks A to E.
- **The later ornaments-in-the-score work would add:**
  - `song` 1.1, additive: `Note.ornament`, optional;
  - a new notes-view syntax version in its header, `notes v2`;
  - a wider `duet-abc` subset. That is not a duet kind, but it needs round-trip tests in
    `crates/duet-midi/tests/roundtrip.rs` and the ABC crate.

## Not decided here

- **Whether ornaments go into the score:** named above for later, and not filed until the owner asks.
- **The arrival bound:** a named open parameter, set after check 1 (question 2).
- **An ornament in the middle of a held note:** open (question 2), informed by check 3.
