# `duet capture`: a gesture read where it arrives, and an ornament shorter than any note

**Date:** 2026-11-16. **Status:** **Ruled (owner, 2026-11-16):** the owner adopted every
recommendation below as proposed. One measurement is still open: question 2 asks for it, and build
task B depends on it.

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
- **A wobble of 1–2 frames per pitch,** such as Blood Sport's "-eak". It stays in `rest`.

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

- The approach must be shorter than the decoder will hold as a note of its own, and must move toward
  the arrival, with no onset and no voicing dip between them.
- The held approach is kept as a grace excursion (question 1), so nothing sung is lost.

**The bound comes from the decoder's own prices,** not from a new threshold:
- Holding a stable note costs `c` plus `d²/2σ²` a frame, and an attack frame costs `ATTACK_COST`.
- So an approach is absorbed when it is shorter than about `c / (ATTACK_COST − d²/2σ²)` frames: about
  0.09 s held on its key, and 0.18 s held 35 cents off.
- A longer held pitch is a note.

**A real step stays two notes.** A pitch held 0.25 s, then a semitone up held 0.25 s, reads as two
notes. This clause comes from the held contour task.

**The readers:**
- The HMM already reads all eight approaches in table 1 this way. This design states what it does and
  witnesses it.
- The rules are the baseline, and they are unchanged: they read a held approach as a note. That is a
  stated limit, written on the `Segmenter`.

**Open: Blood Sport's opening on the HMM since #48.**
- There, the approach (F3, about 0.11 s) is as long as the arrival (F#3, about 0.1 s, held flat).
- The listening study measured the HMM before #48. Its later note does not cover the opening.
- This is a measurement asked of the owner's local session.
- **If the HMM reads F#3 there,** task B is witnesses only. **If it reads F3,** task B changes the
  decoder: the arrival is the pitch held last before the voice leaves, not the one held longest.

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

**Ruled.**
- **Moments.** `crates/duet-capture/corpus/wayfaring-stranger.moments` names the moments in both
  tables, with the take's licence.
  - Every listening tool can then cite the moments.
  - It can be committed: the take is CC BY 4.0, and the file states the attribution.
  - Blood Sport's moments stay in its untracked `fixtures/`.
- **`compare` plays a moment three ways:**
  - **melody:** the keys only;
  - **steps:** the keys, with each excursion played at its key, held for its frames;
  - **bends:** the keys plus the residual.
- **The owner rules each moment** through `duet-listen`. Does "steps" sound like what was sung? Is it
  an ornament, or a note? The rulings are recorded in Temper.
- **What the ear must still settle:** whether 9.55 s and 16.40 s are a new syllable (two notes, as
  both readers read them today) or an ornament inside one note.

## 5. The held contour task

**Ruled: cancelled, pointing at this spec.** That task is temper
`01a1dd79-a389-7162-a2dc-bbf89b3382cf`.

Its moments are covered elsewhere:

| its moment | where it went |
|---|---|
| Blood Sport 33.3 s, the turn | question 1 |
| the opening after the dip | question 2 |
| 6.4 s and 11.4 s, between keys | the tuning work and the transformations task (snap to key, per-note correction) |

Its one live clause, "real semitone steps, quick and then held, are still two notes", is task B's
witness.

## Build tasks

Each task is filed by the control plane. None is built before the one it needs.

**A. Excursions in the residual.**
- Witnesses:
  - A synthetic held G#3 with a 0.2 s turn up to A#3 and back reads as one note, with one excursion
    at +200 ± 10 cents, its frames within one of where it was synthesized.
  - A held approach of 0.1 s a semitone below reads as one excursion before `held_from`.
  - A vibrato of ±40 cents at 6 Hz has no excursion.
  - A wobble of 2 frames per pitch has no excursion.
  - A note with no ornament has none.
  - The loss is unchanged by the new part.
  - Every 1.4 example still reads.
- Once the owner has listened (D), the "Wayfaring Stranger" listened entry gains an excursion count,
  with a tolerance.

**B. The arrival rule, witnessed.**
- It needs measurement 1 (question 2).
- Witnesses:
  - A synthetic approach held 0.1 s, 35 cents flat a semitone below, then the arrival held 0.12 s,
    reads one note on the arrival, with `held_from` at the arrival.
  - A step held 0.25 s each side reads as two notes.
  - Both fail if the attack stops absorbing a held approach.
  - "Wayfaring Stranger" at 18.56 s reads F#3.
- If the measurement shows the HMM reading F3 at Blood Sport's opening, the decoder change and a
  synthetic witness of that shape join this task. That witness must fail on `main`.

**C. `compare` shows and plays excursions.**
- It needs A.
- It adds the stdout column, the SVG marks and the three renderings.
- Witness: a duet-e2e test on a synthetic turn checks two things. The stdout lists one excursion. In
  "steps", the WAV's right channel over the excursion's frames is within 1 cent of key plus offset.

**D. The "Wayfaring Stranger" moments, and the owner's listen.**
- It needs C.
- Witness: the owner's ruling on each moment, by ear.
- The rulings decide the listened entry's excursion count (A), and whether 9.55 s and 16.40 s are one
  note.

**Transformations task (temper `01a1df08-2b63-7658-8812-7823c97c6147`):** this design amends it in
two ways, and adds no task:
- `excursions` is a part a person keeps or drops per note;
- `correct k` keeps excursions.

## Formats

- **`capture-analysis` 1.5: additive (a minor).** `NoteResidual.excursions` is a new optional field,
  omitted when empty. Its absence means what 1.4 meant: no ornament stated. Task A adds it.
- **`song`, `performance`, the envelope:** no change in tasks A to D.
- **The notes and grid views:** no change in tasks A to D.
- **The later ornaments-in-the-score work would add:**
  - `song` 1.1, additive: `Note.ornament`, optional;
  - a new notes-view syntax version in its header, `notes v2`;
  - a wider `duet-abc` subset. That is not a duet kind, but it needs round-trip tests in
    `crates/duet-midi/tests/roundtrip.rs` and the ABC crate.

## Not decided here

- **Whether ornaments go into the score:** named above for later, and not filed until the owner asks.
- **Blood Sport's trill (9.4–10.9 s) and "-eak" wobble on the HMM since #48:** measurements 2 and 3
  are asked for with measurement 1. They test the excursion constants of task A on a real take. They
  are not witnesses in git.
