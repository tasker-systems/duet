# `duet capture`: a gesture read where it arrives, an ornament shorter than any note, and a note rejoined across its dip

**Date:** 2026-11-16. **Status:** **Ruled (owner, 2026-11-16).** Questions 1 to 5 are ruled, as
recorded in the owner's Claude Doc "duet: ornament and tuning rulings".

The owner has answered the four checks by ear (temper task
`01a1e4a5-ce43-7159-86d5-9ea18dc5731f`). Answer 1 removed the arrival merge pass that the earlier
draft proposed; answer 3 adds a same-key rejoin (question 2).

Two questions are still open, each marked **Open** in question 2:
- how deep a rejoined dip may be;
- how a reader tells a breathy ornament from a breath.

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

- **18.56 s looks like Blood Sport's opening:** a flat F3 held about as long as the F#3 it arrives
  at. The owner hears the opening's F3 as a sung note of its own (question 2). Nobody has listened to
  18.56 s yet (task D).
- **The HMM names the arrival every time.** It states the approach only as a long scoop, or as drift
  and rest. That is also how it states Blood Sport's turn at 33.3 s.

**2. Ornaments inside a held note.** Most of them move less than a semitone.

| time (s) | what the voice does | rules | HMM |
|---|---|---|---|
| 1.42–1.61 | turns on D#3: down to D3 +30, up to E3 −30, back | adds an E3 (between keys) | one D#3, rest 41c |
| 9.55–9.79 | dips from D#3 to C#3 +40 and back | two D#3 | two D#3 (the owner hears one, question 2) |
| 16.40–16.55 | a new word on F#3, then rises to G3 −25 and back | two F#3 | two F#3 (right, question 2) |
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
The owner heard it as an ornament (check 2), so the floor stands.

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

## 2. Which key a gesture takes, and when one note's split is rejoined

**Ruled: the arrival's key.** #28 extends to this:

> **An approach belongs to the note it arrives at, held or not.**

- The held approach is kept as a grace excursion (question 1), so nothing sung is lost.
- **The note HMM already does this, and nothing new is built for it.** Its attack takes in all eight
  approaches in table 1, so this design states that behaviour and witnesses it there.
- **No merge pass, and no bound to set.** The owner's checks by ear (temper task
  `01a1e4a5-ce43-7159-86d5-9ea18dc5731f`) showed that Blood Sport's opening is three sung notes, F3,
  F#3 and G#3, as the HMM reads them.
  - A pass joining a held approach to its arrival would have fused that F3 into the F#3: it is short,
    moves toward the F#3, and stays strongly voiced across the join.
  - So where the HMM's own prices make a held pitch a note, it stays a note. The opening is a "stays
    separate" witness.
- **The rules are unchanged.** They are the baseline and read a held approach as a note. That is a
  stated limit, written on the `Segmenter`.

**Ruled by the owner's ear: a same-key split inside a phrase is one note.** The owner heard two
"Wayfaring Stranger" splits where both readers read two notes on one key:

| time (s) | what is sung | both readers | the owner hears |
|---|---|---|---|
| 9.54 | a dip to about D3 and back, inside one phrase | D#3, D#3 | **one D#3**, the dip an excursion |
| 16.39 | a consonant starting a new word on the same F#3 | F#3, F#3 | **two F#3**, as read |

Blood Sport's opening F3 is the same case as 9.54 s: the HMM splits it around an intentional drop
that the owner hears inside one F3. It is a witness for local listening only.

So the rule the design needs is a **same-key rejoin**:
- Two adjacent notes on one key, split across a dip within a phrase, are rejoined into one note.
- The dip's frames become an excursion of that note.
- A split at a consonant or an onset stays two notes.

**This narrows #28's "a dip is two notes"** to a dip that a consonant or an onset marks.

**What separates the two moments in "Wayfaring Stranger"**, measured on `d0fd967`:

| evidence | 9.54 s (one note) | 16.39 s (two notes) |
|---|---|---|
| an onset at the split | none | one, at 16.393 s (weak, like every onset on this take) |
| voicing | stays at 0.83 or above | stays at 0.83 or above |
| level | falls by about half, smoothly, with the pitch, and comes back | flat across the split |
| pitch | a smooth V, 170 cents down and back over 0.1 s | continuous; a few cents' step, then a rise toward G3 |

- **On this take the onset is the separator.**
- Voicing does not separate them: each dips to 0.83 for a frame or two.
- Level separates them only in the wrong sense for a consonant: the dip loses level, and the
  consonant does not.

**The rule rejoins two adjacent notes on one key** when:
- no onset falls between the first note's last held frame and the second note's `held_from`;
- every frame between them is voiced: no silent frame, no frame the pitch track decoded unvoiced;
- the dip stays within a neighbour's depth (the open parameter below).

**Open, for the owner: how deep may a rejoined dip be?** Two witnesses in `notes.rs` from the #28
ruling pin a dip with no onset and no silence as two notes:
- `a_dip_between_two_syllables_is_two_notes_and_the_dip_none`: 8 semitones down and back, "a voiced
  consonant between two syllables on one pitch";
- `a_slower_dip_is_still_no_note_at_its_bottom`: 3 semitones down and back.

Neither has an onset. So "no onset" alone would rejoin both and overturn that ruling's witnesses.
- **Options:**
  - **(a)** rejoin only a dip within a whole tone (200 cents) of the note. Both existing witnesses
    stay two notes;
  - **(b)** rejoin any dip with no onset. Both existing witnesses are rewritten as one note with an
    excursion, and #28's dip case then needs an onset to be two notes;
  - **(c)** a depth set between the two, after the owner listens to a synthetic 3-semitone dip.
- **Recommended: (a).** A neighbour ornament moves a step. 9.54 s moves 170 cents, and Blood Sport's
  turn moves a whole tone. A voiced consonant's dip, as the #28 witness models it, falls much
  further. (a) leaves the existing ruling's witnesses as they are.

**Built as a pass over the reading, after decoding,** so it can be witnessed on its own against
either reader's reading:
- The pass states each rejoin.
- The rejoined note's dip frames count as held frames in the loss, priced by their pitch as any held
  frame is. A rejoin that raises the loss is stated, not hidden.

**A real step stays two notes.** This clause comes from the held contour task. Its witness is set
where a pass could most easily go wrong: two sixteenth notes at 108 bpm, a semitone apart, each on
its key. The rejoin never joins notes on two keys.

**Open, for the owner: how does a reader tell a breathy ornament from a breath?**
- The owner hears Blood Sport's "-eak" (34.66–34.93 s) as one F#3 with ornaments. That includes the
  fall after it, where voicing drops to about 0.01–0.11. The take's capture (a laptop mic, a voice
  memo) and breathiness explain the low voicing.
- **This is in tension with #48.** #48 prices a frame left unread by its voicing, so a breath
  (voicing about 0.02) is free to leave out, and the HMM ends a note there.
- **The evidence available beyond voicing:**
  - **level:** a breath's level falls toward the noise floor; a breathy sung frame keeps some of the
    note's;
  - **pitch continuity:** a sung frame's pitch, where the tracker gives one, continues the note's
    line; a breath has none, or jumps;
  - **position:** low voicing between two notes on one key, or at the end of a note, against low
    voicing at the start of a phrase or between phrases;
  - **onsets:** a breath before a new phrase usually precedes an onset; an ornament inside a note
    does not.
- Ruling how these combine is the owner's call, not this design's. No task is filed until it is
  ruled (task F).
- **Ruled (owner, 2026-11-16):** a weakly voiced frame belongs to the note it sits in when its
  pitch continues the note's line, its level stays within a stated range of the note's held
  level, and no onset follows before the next note; otherwise it is a breath, left unread as
  #48 prices it. Pitch continuity weighs above level. Built as a pass over the reading, before
  the rejoin (`crates/duet-dsp/src/breathy.rs`, which states the values and why).

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

**Ruled, with two additions:** the Blood Sport moments, and the sixteenth-note step as task B's
witness.

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

The owner has answered all four checks (temper task `01a1e4a5-ce43-7159-86d5-9ea18dc5731f`,
"## Answers").

| check | the answer | what it decides |
|---|---|---|
| 1. Blood Sport 0.56–0.82 s | three sung notes, F3, F#3 and G#3; the F3 is one note with its drop inside it | no merge pass and no bound (question 2); the opening is a "stays separate" witness, and its F3 a rejoin witness, both for local listening |
| 2. Blood Sport "-eak", 34.66–34.93 s | an ornament, including the low-voicing fall after it | the 3-frame floor stands (task A); the breath question opens (task F) |
| 3. "Wayfaring Stranger" 9.55 s and 16.40 s | 9.54 s is one D#3 with a dip; 16.39 s is two F#3, at a new word | the same-key rejoin (task B) |
| 4. Blood Sport's tuning | goes to the tuning task | nothing here |

**Still to rule:**
- the depth of a rejoined dip (question 2), which gates task B;
- a breathy ornament against a breath (question 2), which gates task F.

## Build tasks

Each task is filed by the control plane. None is built before the one it needs.

**A. Excursions in the residual.**
- **Needs:** nothing. **Gate:** none. Check 2 confirmed the 3-frame floor.
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
  - Every example of the minor before still reads.
- **Then:** once the owner has listened (task D), the "Wayfaring Stranger" listened entry gains an
  excursion count, with a tolerance.

**B. The same-key rejoin.**
- **Needs:** A, since the dip becomes an excursion. **Gate:** the ruling on a rejoined dip's depth
  (question 2).
- **Witnesses that fail on `main`:**
  - "Wayfaring Stranger" at 9.54 s reads one D#3, with one excursion for the dip. On `main` both
    readers read D#3, D#3.
  - A synthetic held D#3 that dips 170 cents and back over 0.1 s, with no onset, reads one note. On
    `main` the HMM reads two notes. The rules read one, since the dip is too short for them to leave
    the note.
  - A synthetic held G#3 that turns up to A#3 for 0.14 s in its middle, with no onset, reads one note
    with one excursion under the HMM. On `main` the HMM reads two G#3. The rules read G#3, A#3, G#3,
    which is no same-key pair, so that reading stays as the rules' stated limit.
- **Witnesses that pass on `main` and must keep passing:**
  - "Wayfaring Stranger" at 16.39 s reads two F#3: an onset falls at the split.
  - The synthetic dip above, with an onset at its bottom, reads two notes.
  - The sixteenth-note step: two sixteenths at 108 bpm, G3 then G#3, each 0.139 s on its key, then
    a held A#3, read as three notes under both readers.
  - A synthetic approach held 0.1 s, 35 cents flat a semitone below, then its arrival, reads one
    note on the arrival under the HMM. This is the arrival rule, witnessed.
  - "Wayfaring Stranger" at 18.56 s reads F#3 under the HMM.
  - Under depth option (a): both #28 dip witnesses in `notes.rs`, run through the pass, still read
    two notes.
- **For local listening only, never committed:** Blood Sport's opening reads F3, F#3, G#3, with the
  F3 rejoined across its drop. On `main` the HMM splits the F3.

**C. `compare` shows and plays excursions.**
- **Needs:** A. **Gate:** none.
- It adds the stdout column, the SVG marks and the three renderings.
- **Witness:** a duet-e2e test on a synthetic turn checks two things. The stdout lists one excursion.
  In "steps", the WAV's right channel over the excursion's frames is within 1 cent of key plus
  offset.

**D. The moments, and the owner's listen.**
- **Needs:** C. **Gate:** none.
- It commits the "Wayfaring Stranger" moments file. A draft is kept in the build notes.
- **Witness:** the owner's ruling on each moment, by ear, through the three renderings.

**F. A breathy ornament is read as part of its note.**
- **Filed** once the breath question in question 2 was ruled: temper
  `01a1e523-ebde-773f-92b7-f810e8c112b5`. `capture-analysis` 1.9 (or the next free minor),
  additive: the reading's optional `breathy`.
- **Witness, once ruled:** Blood Sport's "-eak", for local listening only, reads one F#3 with its
  excursions. A synthetic breath between two phrases is still left unread.

(Task E, from the earlier draft, is now part of B: a turn in the middle of a note is a same-key
split.)

**Transformations task (temper `01a1df08-2b63-7658-8812-7823c97c6147`):** this design amends it in
two ways, and adds no task:
- `excursions` is a part a person keeps or drops per note;
- `correct k` keeps excursions.

## Formats

- **`capture-analysis`, additive (a minor): the minor after the tuning change's.** The tuning task
  takes the next minor first.
  - `NoteResidual.excursions` is a new optional field, omitted when empty. Its absence means what the
    minor before meant: no ornament stated. Task A adds it.
  - The rejoin's statements are a new optional field of the reading in the same minor, omitted when
    there is none. Task B adds it.
- **`song`, `performance`, the envelope:** no change in any task here.
- **The notes and grid views:** no change in any task here.
- **The later ornaments-in-the-score work would add:**
  - `song` 1.1, additive: `Note.ornament`, optional;
  - a new notes-view syntax version in its header, `notes v2`;
  - a wider `duet-abc` subset. That is not a duet kind, but it needs round-trip tests in
    `crates/duet-midi/tests/roundtrip.rs` and the ABC crate.

## Not decided here

- **Whether ornaments go into the score:** named above for later, and not filed until the owner asks.
- **How deep a rejoined dip may be:** open (question 2). It gates task B.
- **A breathy ornament against a breath:** open (question 2). It gates task F.
