# Modes: a given mode first, and an evidenced suggestion later

**Date:** 2026-11-20. **Status: Proposed.** Nothing here is ruled or built. Each of the task's open
questions is answered below with a proposal and its reasons. What needs the owner's ruling is listed
apart, in [For the owner](#for-the-owner).

**Task:** temper `01a1e97a-a969-777c-b6d6-fc4430458d63`. Its first real need is the per-note
snap-to-key suggestion of the transformations task (temper `01a1df08-2b63-7658-8812-7823c97c6147`,
its "Ruled" section). That task's transformation set and its 35-cent threshold are proposed, not
ruled, and nothing here rules them.

**Builds on:**

- The key evidence for "Wayfaring Stranger" (research `01a1e926-5755-7477-a36c-86dfb4763cce`):
  E♭ minor, Dorian in colour, with Dorian against Aeolian resting on two or three C notes.
- The profile task (`01a1e95b-3e7a-7284-a8c1-58862685f522`, done): Aarden–Essen is the profile, the
  estimate states it (`capture-analysis` 1.12), and the corpus pins E♭ minor.
- The six-accidental spelling task (`01a1e97a-cb37-70f3-9681-88f4cdc2e505`, done): the key spells a
  note of its scale as the scale spells it, and another white key as its natural letter, so the C of
  E♭ Dorian is C.
- The intent decision (`01a1e524-486a-77dd-ac84-de2e665729dc`): a suggestion is stated beside what
  was read, never over it.
- The documents design's rule for classifying a change
  ([§3](2026-11-15-duet-documents-design.md)).

**Grounding:**

- The code at `main` `ce02481` (`capture-analysis` 1.13), release build. Re-measured at `main`
  `6037aee` (`capture-analysis` 1.14, the FLAC reader): every number below is the same.
- `crates/duet-capture/corpus/wayfaring-stranger.flac` (CC BY 4.0), captured with `--key "Eb
  minor"`, with `--key "Eb dorian"`, with nothing given, and with `--reader rules`.
- The ABC corpus tunes `drunken-sailor` (D Dorian), `god-rest-ye` (E minor), `amazing-grace`,
  `washerwoman`, `twinkle` and `frere-jacques`, converted to parts and read through `duet view`.
- The measuring scripts were throwaway Python over those files. They re-implement `key.rs`'s
  Aarden–Essen estimate, weighting each note by its length, and agree with duet's own estimate on
  "Wayfaring Stranger" (E♭ minor, 0.773). They are not committed. Their rules are the ones proposed
  below.

## The premise, corrected: a given mode is already built

**The task body says the `Key`'s mode "gains variants". It already has them.** Commit `bb4f907`
(2026-11-14, "a Key has a mode") gave `duet_core::Mode` all seven: major, minor, Dorian, Phrygian,
Lydian, Mixolydian and Locrian. Ionian and Aeolian read as major and minor.

**Everything step 1 names, but the snap scale, works on `main` today:**

| | on `main` | where it is witnessed |
|---|---|---|
| `--key "Eb dorian"` on `duet capture melody` | accepted, used exactly, written as given | tried here |
| `--key "D dorian"` on `duet song new` | accepted | `crates/duet-e2e/tests/songs.rs`, `project_defaults.rs` |
| `capture-analysis` and `song` | `Key` is a string with a pattern that has taken every mode since 1.0 | `crates/duet-schemas/tests/schemas.rs` |
| ABC | reads `K:Ebdor`, `K:Eb dorian`, `K:F#Mix`; writes `K:Eb dorian` | the `key-modes` and `drunken-sailor` corpus tunes, checked against abc2midi |
| the notes view | `key Eb dorian` | `crates/duet-midi/tests/roundtrip.rs` |
| MIDI | the signature, plus a `duet:key Eb dorian` text event that duet reads back | `crates/duet-midi/tests/roundtrip.rs` |
| spelling | the mode's natural scale; another white key is its natural letter | `crates/duet-e2e/tests/spelling.rs` |

**So no document changes version for a given mode.** The documents design's rule names "a new
variant of a tagged enum" as a minor. `Key` is not a tagged enum in any document: it is one string,
`"Eb dorian"`, whose pattern already admits every mode. A given mode is already valid
`capture-analysis` 1.0 and `song` 1.0.

**What a given E♭ Dorian changes on "Wayfaring Stranger", against a given E♭ minor:**

- **The ABC:** `K:Eb dorian` (five flats, D♭ major's signature) instead of `K:Ebm` (six). The three
  C naturals lose their `=`. Nothing else in the file changes.
- **The notes view:** only the `key` line. Every note is spelled the same under both: the C is C
  natural under either key, and nothing in the take is a C♭.
- **No note's MIDI number changes,** as the capture design requires.

**What `key.rs` says is now only half true.** Its module comment says "v1 covers major and minor
only; the modes come later". The estimate covers major and minor only. A given key has covered the
modes since before the estimate existed. The corpus manifest's comment on "Wayfaring Stranger" ("v1
has major and minor only, so its Dorian C is an accidental") is true of the estimate only. Both are
corrected in build task M1.

## 1. The scale a snap suggestion uses

**This is step 1's one missing piece, and the first real need.** The transformations task's
suggestion says "between keys: nearest in key is X". It needs a set of pitch classes that are in
key. duet has none in public: `Mode::spelling_steps` is private to `duet-core` and exists for
spelling.

**Proposed: `Key::in_key(pitch_class) -> bool` in `duet-core`, public, with these scales:**

| mode | in key | why |
|---|---|---|
| major, and Dorian, Phrygian, Lydian, Mixolydian, Locrian | the mode's seven natural degrees | a mode is its natural scale |
| minor | the natural minor's seven degrees, **and** the raised seventh: eight pitch classes | a sung minor tune uses both sevenths |

**Why minor is not the spelling scale.** duet spells minor as harmonic minor, so its leading tone is
raised and its subtonic is not in the scale. On "Wayfaring Stranger", that would be wrong twice:

- **The cadences use the subtonic.** Phrases 1 and 4 close D♭ → E♭ (the research, §2). Under
  harmonic minor, D♭ is out of key. The HMM reads D♭3 at 5.18 s (+38 cents) and D♭4 at 8.43 s
  (+36 cents) between keys. Each would be suggested up to D, the leading tone, which the cadence
  does not sing.
- **Harmonic minor has a gap of three semitones** (C♭ to D). The C at 8.99 s (+39 cents) then has
  neither neighbour in key: C and D♭ are both out. "Nearest in key" would have to reach past a
  neighbour, which the suggestion's wording does not allow.

**Why not natural minor alone.** The take sings D natural too: a D3 of 0.24 s at 6.62 s, and the
HMM's fall to D3 at 20.18 s. The leading tone is a note of the minor key in sung practice. Leaving
it out would mark it as an accidental to be moved.

**Why not melodic minor's raised sixth as well.** With it, minor would hold every note of Dorian,
and a given mode would change no suggestion. The raised sixth is the one degree that tells Dorian
from minor (§3).

**A property the build can rely on.** In every proposed scale, no two adjacent keys are both out of
key. A diatonic scale has no gap wider than a whole tone, and minor's eight notes have none either.
So a note between keys has its own key or the neighbour it leans toward in key, and "nearest in key"
is always one of the two keys either side of what was sung. Harmonic minor breaks this, which is a
second reason not to use it.

**The suggestions on "Wayfaring Stranger", by scale.** The HMM reads 6 of its 38 notes between keys,
on its estimated tuning (−7 cents). A note whose own key is in key keeps it. A note whose own key is
out is suggested toward the neighbour it leans to.

| note | sung | E♭ minor, harmonic (today's spelling) | E♭ minor, proposed | E♭ Dorian |
|---|---|---|---|---|
| D♭3 at 5.18 s | +38 c | **to D3** | keeps D♭3 | keeps D♭3 |
| E♭3 at 5.36 s | +36 c | keeps | keeps | keeps |
| D♭4 at 8.43 s | +36 c | **to D4** | keeps D♭4 | keeps D♭4 |
| C3 at 8.99 s | +39 c | neither neighbour in key | **to D♭3** | keeps C3 |
| A3 at 12.00 s | +41 c | to B♭3 | to B♭3 | to B♭3 |
| G♭3 at 14.84 s | +36 c | keeps | keeps | keeps |

The rules reader reads 9 between keys. Under the proposed E♭ minor, its two C3s (8.91 s, +43 cents;
10.04 s, +44 cents) are suggested to D♭3, and under E♭ Dorian they keep C3. Its other seven are the
same under both scales.

**The task body's example is corrected.** It says a sung C natural "could be suggested toward C♭"
under E♭ minor. It would not: the Cs are sung sharp, 39 to 44 cents above C, so the neighbour they
lean to is D♭. A C would only be suggested to C♭ if it were sung flat.

**So a mode changes exactly what it should.** Given E♭ Dorian, the Cs are in key and no suggestion
moves them. Given E♭ minor, they are suggested up to D♭. Nothing is moved unasked either way: the
suggestion is accepted note by note (the transformations task's ruling).

## 2. Which modes, and how they are written

**Proposed: every mode stays accepted as a given key, as it is on `main`. Only the suggestion of §3
is scoped, to Dorian and Mixolydian first.**

- **Narrowing the given modes would be a major version** of `capture-analysis` and `song`: "a range
  or pattern narrowed" (the documents design, §3). Nothing gains from it, and every mode is already
  witnessed through ABC, MIDI and the views.
- **The suggestion is where scope costs something.** Each mode suggested needs its own evidence
  rule, witnessed on a tune in that mode. The owner's folk and gospel material leans on Dorian and
  Mixolydian. The corpus has a Dorian tune (`drunken-sailor`) and no Mixolydian one.

**Proposed: modes are written as they are today.**

- **ABC:** `K:Eb dorian`, the mode's full name after a space. ABC 2.1 matches a mode on its first
  three letters, ignoring case (§3.1.14). So `K:Ebdor`, `K:EbDor` and `K:Eb dorian` are one key, and
  duet reads all three. duet writes the full name, which abc2midi reads: the `key-modes` corpus
  tune, as duet writes it, is checked against abc2midi's output.
- **The notes view header and notefmt:** `key Eb dorian`, as today.
- **Standard error and the analysis:** `Eb dorian`, the same string.

## 3. An evidenced suggestion, if wanted

**Proposed: after the major or minor estimate, look at the one degree that tells each mode from its
parallel major or minor. Suggest the mode only when that degree is sung on its key and its
counterpart is not sung at all.** The suggestion is stated beside the estimate, never as it.

### Why not modal profiles

- **No folk-fitted modal profiles exist to use.** Aarden–Essen and Krumhansl–Kessler are major and
  minor only.
- **A profile made by turning the major one cannot find a modal tonic.** The Aarden–Essen major
  profile turned to start on its second degree, and set on E♭, is exactly D♭ major's profile. Its
  correlation with any histogram is D♭ major's score. It can only say what the signature is, which
  the 24 scores already say, and never that the tonic is E♭.
- **So the tonic comes from the estimate, and the mode from one degree.** The estimate's tonic is
  firm on "Wayfaring Stranger" under every reader (the research, §1 and §4). Its colour is not, and
  a colour is what one degree decides.

### The rule

| over an estimate of | suggest | when this degree is sung | and this one is not |
|---|---|---|---|
| minor | **Dorian** | the raised sixth (C over E♭) | the lowered sixth (C♭) |
| major | **Mixolydian** | the lowered seventh (D♭ over E♭) | the raised seventh (D) |
| minor | Phrygian, later | the lowered second | the natural second |
| major | Lydian, later | the raised fourth | the natural fourth |

- **"Sung" means on at least two notes that are on their key,** not between keys. A note between
  keys is the reading's own statement that its key is unsure (35 cents, owner, 2026-11-15). Evidence
  for a mode should not rest on it.
- **"Not sung" means on no note at all,** between keys or not.
- **Locrian is never suggested.** It is rare, and absent from the owner's material and the corpus,
  so no tune could witness its rule.
- **A tune with neither degree gives no suggestion,** as a pentatonic tune does. Nothing says Dorian
  or minor there.

### What it finds

| tune | estimate | the degree | its counterpart | suggests |
|---|---|---|---|---|
| `drunken-sailor` (notated D Dorian) | D minor, 0.753 | B: 3 notes, 3.1 % of the length | B♭: none | **Dorian** |
| `god-rest-ye` (E minor) | E minor, 0.948 | C♯: none | C: 3 notes | nothing |
| `washerwoman` | G major, 0.846 | F: none | F♯: 8 notes | nothing |
| `amazing-grace` | G major, 0.842 | F: none | F♯: none | nothing (pentatonic) |
| `twinkle`, `frere-jacques` | G major, F major | none | none | nothing |
| "Wayfaring Stranger", HMM | E♭ minor, 0.773 | C: 3 notes, 0.55 s, 3.9 %; **2 on their key** | C♭: none | **Dorian** |
| "Wayfaring Stranger", rules | E♭ minor, 0.739 | C: 2 notes, 0.51 s, 3.3 %; **none on its key** | C♭: none | nothing |

- **Dorian evidence is thin even in a notated Dorian tune.** Drunken Sailor's raised sixth is three
  quarter notes in 24 bars as played, with the repeat: 3.1 % of its length. "Wayfaring Stranger"'s
  C is no thinner by share (3.3–3.9 %). What makes it thin is how it is sung.
- **The Cs are sung sharp.** Under the HMM they sit at +25, +39 and +7 cents (the last is the C4 at
  13.22 s, which the rules read as no note). Under the rules they sit at +43 and +44 cents, both
  between keys. Whether that is a Dorian C sung high or a note on its way to D♭ is a question for
  the ear, not for the numbers.
- **So the readers disagree on the suggestion,** and the rule says so plainly rather than hiding it.

### Where it is written

- **In the analysis:** a new optional field on `KeyEstimate`, `suggested`, holding the suggested key
  (`"Eb dorian"`) and its evidence: the degree's pitch class, its notes on their key and their total
  length, and the counterpart's note count. It is absent when nothing is suggested, and absent means
  what an older document meant. That is a minor version of `capture-analysis`, whichever is next
  when it merges.
- **On standard error,** a line after the key line:

  ```text
  key: estimated Eb minor (relative Gb major), confidence 0.77
  mode: Dorian suggested, C on 2 notes on their key (0.38 s), C-flat on none; --key "Eb dorian" uses it
  ```
- **Never applied.** The part's key, the spelling, the signature and every snap suggestion stay the
  estimate's. A person uses the mode by giving it.
- **Not made for a given key.** A given key is used exactly, and the suggestion would second-guess
  it.

## Witnesses the build authors

**All are declared holes.** None exists yet. Each fails on `main` because the function, the
suggestion or the field does not exist, except where stated.

**For M1, the scale:**

- **W1. Each mode's scale** (`crates/duet-core/src/time.rs` tests): `in_key` holds exactly the
  degrees in [§1](#1-the-scale-a-snap-suggestion-uses)'s table, for every tonic, with minor's eight.
  It fails on `main`, where `in_key` does not exist.
- **W2. No two adjacent keys are both out,** for every key from seven flats to seven sharps in every
  mode. Harmonic minor would fail it; a test comment says so.
- **W3. The snap suggestion follows the mode** (in the transformations task's build, on
  "Wayfaring Stranger" read by the HMM): under a given E♭ minor, the C3 at 8.99 s is suggested to
  D♭3 and the D♭ notes keep their keys; under a given E♭ Dorian, the C3 keeps C3; the A3 at 12.00 s
  is suggested to B♭3 under both. It is pinned as it reads when the build lands. A reader change
  that moves a note across the 35-cent line fails by name.

**For M2, the suggestion, if wanted:**

- **W4. A Dorian tune is suggested Dorian:** a synthetic take of `drunken-sailor`'s melody, made by
  `scripts/capture-fixtures.sh` like the corpus's synthetic takes and read with `--tuning 440` and a
  given tempo. The estimate is D minor, and the suggestion is D Dorian with the B's evidence.
- **W5. An Aeolian tune and a pentatonic one are suggested nothing,** on the same kind of take
  (`god-rest-ye`, `amazing-grace`).
- **W6. The rule's edges, on note lists built by hand** (`crates/duet-dsp/src/key.rs` tests): one
  note on its key is not enough and two are; a note between keys does not count toward the degree;
  one counterpart note, even between keys, blocks the suggestion; a given key gets none; Mixolydian
  over a major estimate.
- **W7. The field round-trips,** and a document from before the field reads as suggesting nothing
  (`crates/duet-schemas` tests and the committed schema).
- **W8. "Wayfaring Stranger" is pinned as it reads under each reader** (HMM: Dorian; rules:
  nothing), like the manifest's listened counts. The fragility is stated on the test: one C crossing
  the 35-cent line flips either reader. The owner's ear, not the count, rules a change.

**A hole no build closes:** the Mixolydian rule is measured on no tune. The corpus has none. W6
covers it on a hand-built list only, until a Mixolydian take is added.

## For the owner

Each question changes what gets built. A to C are the task's own; D to F come from what was
measured.

**A. Which modes are in scope.**

- **(a)** all seven given, as on `main`, and the suggestion for Dorian and Mixolydian first;
- **(b)** all seven given, and the suggestion for every mode but Locrian;
- **(c)** narrow the given modes to Dorian and Mixolydian: a major version of `capture-analysis` and
  `song`.
- **Recommended: (a).** Given modes cost nothing more, and each suggested mode needs a tune to
  witness it, which the corpus has only for Dorian.

**B. How a mode is written.**

- **(a)** as today: `K:Eb dorian` in ABC, `key Eb dorian` in the notes view and notefmt;
- **(b)** the three-letter form, `K:Ebdor`, in ABC.
- **Recommended: (a).** Both are standard ABC and duet reads both. The full name is what every other
  view writes, and abc2midi already reads it in the corpus.

**C. Is step 2 wanted at all?**

- **(a)** not now: a given mode is enough. M2 is filed held, with this design as its spec;
- **(b)** yes, build M2 after M1.
- **Recommended: (a).** The owner knows the mode of their own material and can give it. The one sung
  witness splits the two readers, and the evidence is the kind a listener should rule on. M2 is
  worth building when a take turns up whose mode the owner did not know.

**D. The minor scale for the snap.**

- **(a)** natural minor and the raised seventh, eight pitch classes;
- **(b)** natural minor only;
- **(c)** harmonic minor, the spelling scale.
- **Recommended: (a).** On "Wayfaring Stranger", (c) suggests the cadences' D♭s up to D and leaves
  the C at 8.99 s with no neighbour in key; (b) marks the sung D naturals as accidentals.

**E. By ear: should "Wayfaring Stranger" suggest Dorian?** The task asks whether 0.2–0.5 s of C over
two notes should. The measure is 0.51–0.55 s over two or three notes, sung 25 to 44 cents sharp. The
proposed rule suggests Dorian under the HMM and nothing under the rules. Whether the C is a Dorian
sixth or a note sung on its way to D♭ is the owner's to hear. Nothing here confirms it. If the owner
hears Dorian, the given key `Eb dorian` is the way to say so, under either answer to C.

**F. The alternative of a given mode.** For a given `Eb dorian`, the analysis states the alternative
`Bb minor`. That is what the field says it is: "the relative major or minor of `key`, which shares
its signature". It reads oddly, since E♭ minor is the likelier other reading.

- **(a)** leave it, and correct nothing;
- **(b)** make a modal key's alternative its parallel major or minor: a change of an existing
  value's meaning, so a major version.
- **Recommended: (a).** It is stated nowhere but the analysis, and a given key's alternative is
  rarely read. A suggestion under M2 carries the parallel reading where it matters.

## Not measured

- **The owner's ear** on the Cs of "Wayfaring Stranger" (question E).
- **Any Mixolydian tune,** sung or written.
- **The owner's own takes** in Dorian or Mixolydian. None is in the worktree.
- **A sung Dorian take other than "Wayfaring Stranger".** The Drunken Sailor numbers are from its
  notated part, not a capture of it.

## Build tasks

**M1. Small: the key's scale for the snap suggestion.**

- **Needs:** nothing. **Gate:** questions A and D ruled. **Sequenced with:** the transformations
  task's snap suggestion, which reads it. M1 can land first, or as that build's first commit.
- **What:**
  - `Key::in_key` in `crates/duet-core/src/time.rs`, with the scales of §1;
  - `key.rs`'s module comment and the corpus manifest's comment on "Wayfaring Stranger" corrected:
    the estimate is major and minor, a given key may be any mode.
- **Witnesses:** W1 and W2 here; W3 in the transformations build.
- **No format changes.**

**M2. Small to medium, held under C (a): the mode suggestion.**

- **Needs:** M1. **Gate:** questions A, C and E ruled.
- **What:** the rule of §3 in `crates/duet-dsp/src/key.rs`; the `suggested` field, a minor of
  `capture-analysis`; its line on standard error; the synthetic Dorian, Aeolian and pentatonic takes
  in `scripts/capture-fixtures.sh`.
- **Witnesses:** W4 to W8.

## Not decided here

- **The snap suggestion's threshold, wording and marks.** They are the transformations task's, and
  still proposed there.
- **A mode change inside a part** (`key E phrygian at 3.1.0`). notefmt and MIDI carry it already.
  The estimate is one key per take, and so is the suggestion.
- **Harmony proposals and new patterns** in a mode. They read the scale M1 provides, but their
  design is their own.

