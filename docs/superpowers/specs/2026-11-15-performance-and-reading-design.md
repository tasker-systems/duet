# `duet capture`: the performance and its reading

**Date:** 2026-11-15. **Status:** four rulings made by the owner on 2026-11-15, each marked **Ruled** where it falls:
- layer 1 is its own file;
- one tuning reference per take;
- the design chooses the note HMM, built after a baseline;
- the crate split goes first.

Every other answer is marked **Proposed** and stands until the owner rules otherwise.

**Goal:** "Now: a sung or tapped idea in an audio file becomes ABC or a drum grid we can discuss, and goes back to MIDI that opens in Logic" (temper `01a1da53-99fa-706b-bc13-a7c9ef1afaf4`).

**Decision it builds:** capture keeps the performance (the pitch time series) apart from its reading (tuning, key, notes). The reading is judged by its residual against the performance, and transformations work on the reading plus the residual. The owner made it on 2026-11-15 (temper `01a1defe-d22e-70f4-9e4b-ef56dd0b1e2f`). This design settles how that is built. It doesn't reopen the decision.

**Amends:** [`2026-11-14-capture-design.md`](2026-11-14-capture-design.md), where a rule it states changes. See *Amendments* at the end.

**Grounding:**
- The code at `3969832`.
- Two sources read first-hand on 2026-11-15:
  - The pYIN Vamp plugin's note HMM (`c4dm/pyin`: `MonoNoteHMM.cpp`, `MonoNoteParameters.cpp` and `PYinVamp.cpp`). It is **GPL-2.0-or-later**, so it can be an oracle and a description of the method, never code we copy or link.
  - librosa's `estimate_tuning` and `pitch_tuning` (ISC).
- The papers themselves (Mauch & Dixon 2014; Mauch et al. 2015; Molina et al. 2014) were still **not read**. This design builds on the code above, not on the papers.

## What exists today, and what is wrong with it

- `PitchTrack` is a `Stage<Mono, PitchFrame>`. Each frame has a smoothed fundamental (`hz`, with parabolic refinement), a voicing probability and a level, on a 256-frame hop at 22 050 Hz: 11.6 ms, or 86 frames a second.
- `Melody` runs `PitchTrack` and `Onsets`, then segments the frames into `Event::Note` inside the same stage. The frames are dropped once they are segmented.
- A note's `midi` is the median of its held frames, rounded to the nearest key **on A440**. That one step fuses three things:
  - what was sung (the performance);
  - where the singer's own in-tune lies (the tuning);
  - which key the note is (the reading).
- So the Blood Sport take, sung a steady 44 cents sharp, lands about halfway between keys on 40 of its 87 notes. The take itself isn't off. Only the A440 grid is.
- `.capture.json` (the `Analysis`) holds the events, the timing, the key, the beat series and the quantize offsets. It has no pitch curve, so nothing written can show where a note departs from the voice.

## The shape

```
FileSource ─▶ Prepare ─Mono─▶ Performance ──frames + onsets──▶ <stem>.performance.jsonl   (layer 1, streamed)
                               (PitchTrack + Onsets)    │
                                                        ▼
                                       read(performance, given) ─▶ Reading                   (layer 2)
                                       tuning ─▶ notes ─▶ key        │  + residual, derived
                                                                     ▼
                                        transform(reading, residual) ─▶ Reading′             (layer 3)
                                                                     ▼
                                        assemble(reading′, timing) ─▶ Song ─▶ .notes / .abc / MIDI
```

`Melody` splits in two:
- **`Performance`** is the stage: pitch tracking and onsets, unchanged, with every frame emitted rather than consumed.
- **The reading** is a function over the settled performance. It is a stage only where a live reading needs one (see question 2).

`Hits` is untouched in this design (question 8).

## 1. Layer 1: what it holds, and where it is written

> **File name (2026-11-15, [duet documents](2026-11-15-duet-documents-design.md)):** `<stem>.performance.jsonl` becomes `<stem>.performance.duet.jsonl`, a `performance` stream: its header is the envelope's `head`, and each later line is a record.

**Ruled (owner, 2026-11-15): its own file, `<stem>.performance.jsonl`.** The rest of this answer is proposed.
- **Contents, per frame:**
  - `pitch`: the smoothed fundamental, or `null` when the track says unvoiced.
  - `voicing`: the frame's own voicing probability.
  - `level`: RMS.
- **Header:** the rate, the hop, the first frame, and the pitch unit.
- **Onsets:** carried as they are today, as `Event::Onset` in `.capture.json`. They are already settled events. The performance file doesn't repeat them.
- **The unit is "absolute cents": `1200·log2(hz/440) + 6900`.**
  - This is MIDI's numbering at 100 per key, so 6900.0 is 440 Hz. It is a unit, not a reading: it maps one-to-one onto Hz and is never rounded.
  - At 0.1 cent of resolution it is lossless at any precision the tracker has. The tracker's HMM bins are 10 cents, refined parabolically.
  - "In cents above A440's grid" is easier to reason about by eye than Hz, and the residual is a plain subtraction in it.
- **It is a file of its own, `<stem>.performance.jsonl`, not a section of `.capture.json`.**
  - **The line format:** the first line is a header. Every line after it is a block of frames, as columns: `{"start":…,"pitch":[…],"voicing":[…],"level":[…]}`. One block is one settled push of the stage, or a second's worth.
  - **Why its own file:**
    - **It streams.** A live source or the MCP server appends lines as frames settle. `.capture.json` is one object, written once at the end, so it can't be appended to.
    - **It is the performance; `.capture.json` is the analysis.** The reading can be re-run over the performance file without decoding the audio again. That is what the comparison tool and every transformation do.
    - **Size.** A 3-minute take is about 15 500 frames. In columns at 0.1 cent and three decimals it is about 21 bytes a frame, roughly **330 KB**. One JSON object per frame would be about 1.4 MB.
- **It is not an `Event`.**
  - `Event` is the sparse vocabulary the MCP stream sends: an onset, a note, a hit.
  - Frames are dense, and their consumers are different: the reading, the plot, the residual.
  - `Performance` is a `Stage<Mono, Layer1>`, where `Layer1` is `Frames { start, frames }` or `Onset`. The MCP server, later, sends frames on a channel of their own.
- **Written always,** like `.notes` and `.capture.json`: once analysis succeeds, the performance is written even if no reading can be made (the capture design's "the performance is always written").
- **A rhythm take** writes no performance file in this design (question 8).

**Witnesses:**
- A synthetic tone at 442 Hz writes frames whose pitch is 6907.9 ± 1.
- The file round-trips exactly through its reader.
- A take pushed in chunks of 1, 512 and 4096 samples writes the same frames.
- A tone 13 cents off A440's grid is written 13 cents off, never rounded to a key.

## 2. What a reading holds

**Proposed.**

```text
Reading {
  tuning:  Origin<Cents>          // the reference: where key 69 (A4) sits, in absolute cents (6900 = A440)
  key:     KeyEstimate            // as today, but over the notes read on `tuning`
  notes:   Vec<ReadNote>
  loss:    Loss                   // question 3
}
ReadNote {
  start, end: u64                 // frames, as Event::Note; `start` includes the slide in, by the #28 ruling
  held_from: u64                  // where the slide ends and held pitch begins
  key: u8                         // the MIDI key number on `tuning`
  centre: f32                     // the held pitch, in cents from `key` on `tuning` (−50..+50)
  confidence, strength: f32
  residual: NoteResidual          // question 3
}
```

- **The tuning is given or estimated, as tempo is.**
  - `--tuning 440`, `--tuning 442` (Hz of A4) or `--tuning +44c` (cents from A440). Given, it is used exactly.
  - Left out, it is estimated and stated on stderr and in the written files: `tuning: estimated A4 = 451.3 Hz (+44 cents), spread 26 cents`.
- **The estimate follows librosa's `pitch_tuning`.** Its method was read in the source:
  - Take each voiced frame's distance from the nearest key, folded into −50..+50 cents.
  - Histogram those distances at 1-cent bins and take the peak.
  - **Weighting:** voiced frames weighted by voicing probability, and only frames whose pitch is steady over 50 ms. Slides and scoops would otherwise smear the peak. librosa thresholds on magnitude instead, which a voice doesn't need.
  - **Confidence:** the share of weight within ±15 cents of the peak.
  - **Why a histogram peak rather than the circular mean the held tuning task planned:** on a take sung wide, the mean is pulled by the tails, while the peak finds where most of the singing sits. This is a choice to witness, not a given. The witness below pits them against each other.
- **The order is tuning, then notes, then key.**
  - The tuning needs only the frames, so it comes first.
  - Notes are read against it.
  - The key is estimated from those notes, by the Krumhansl–Schmuckler method `key.rs` already has.
  - The key never changes a note's `key` number inside the reading (question 5 and *Amendments*).
- **Ruled (owner, 2026-11-15): one reference per take, with drift reported.** A reference that drifts within a take is deferred, the way tempo maps were.
  - Within-take drift still shows: it is each note's `centre` over time, and the per-take summary gives its slope (question 3). So a drifting take is reported truthfully before it is corrected.
  - A drifting reference is a later field, `tuning_map`, built against that report once a real take needs it.
- **A take sung wide of any one reference, as Damocles is (centre −3 cents, spread about 26):**
  - It reads on its best reference.
  - The tuning states its spread and a lower confidence.
  - Every note whose `centre` is more than 35 cents from its key is flagged **between keys** in `.capture.json` and counted on stderr: `notes: 87 read, 9 between keys`.
  - Nothing is silently placed, and those notes are exactly where a key-aware transformation (question 5) has something to decide.
- **Streaming.** The reading is a function over a settled performance, like `estimate` and `assemble` over settled events.
  - For files, it runs once the performance is complete. The whole take is kept as frames (about 86 a second), never as samples, which the stream rule already allows for events.
  - For live input later, the tuning firms up as frames arrive. Notes are decoded by fixed-lag Viterbi (question 4) under the tuning so far, and read again if the tuning moves by more than a few cents. That is the same "provisional under the tempo so far" that `assemble` already promises live input.
  - *Amendment to the stream rule:* the reading joins `estimate` and `assemble` as a function over settled output.

> **Built (2026-11-15, build task 3):** `duet_dsp::read` and `estimate_tuning`, with `--tuning`; the reading is `capture-analysis` 1.1's optional `reading` (additive). Where it departs from the above:
> - **The peak is a 30-cent window's, not a 1-cent bin's.** The bin whose ±15-cent window holds the most weight is the peak, refined by that window's weighted mean. A take sung 40 cents flat whose flattest notes sit 52 flat has its heaviest single bin at +48, a key away; its heaviest window is where it is sung. Confidence is still the share within ±15 cents.
>
> **Amended (capture-analysis 1.5):** confidence is that share measured above a flat spread's (0.31), `(share − 0.31) / 0.69`, at least 0, so a take whose held pitch wanders across the hundred cents is stated weak, and capture then suggests `--tuning`. The estimate is unchanged.
> - **A frame's pitch is smoothed over 0.2 s before it is binned,** and is steady when that smoothed pitch moves no more than 10 cents over 50 ms. Unsmoothed, a vibrato's steady frames are its extremes, which make two peaks either side of where it is sung.
> - **The notes are segmented from the performance's cents** (`fine_hz`, to 0.1 cent), by `Melody`'s rules, so the reading is a function of the kept performance and its onsets alone. `Melody` itself is unchanged and still segments the 10-cent track; the two agree within a frame on synthetic takes, and the corpus passes on the reading.
> - **The key is not in `Reading`:** it stays the analysis's `key`, estimated by assembly from the read notes.
> - **No ndarray yet:** the reading's maths is one 1-D histogram. Unvoiced frames load as `NaN` in one `f32` column (`PerformanceFrames`), and every use tests for it first.
> - **A given tuning is at most 100 cents from 440 Hz:** a single reference cannot tell a take sung 60 cents sharp from one sung 40 flat of the key above.

**Witnesses:**
- A synthetic melody sung 44 cents sharp, then 40 cents flat, reads the keys it reads in tune. This fails on `main` today.
- A take on A440 reads as it does today.
- A given tuning is used exactly, even where the estimate differs.
- The estimate is stated on stderr and in `.capture.json` and `.notes`, and isn't stated when the tuning is given.
- **Peak against mean:** a synthetic take with 70 % of its frames at +20 cents and 30 % at −30 estimates within 3 cents of +20.

## 3. The residual, and how it splits

**Proposed.**
- **Per frame:** the residual is `performance − (tuning + 100·key)`, on every voiced frame inside a note. It is undefined on unvoiced frames and on frames outside every note.
  - Slide frames are measured against the note they arrive at, by the #28 ruling.
  - Gaps are not residual. Voiced frames that no note covers are counted separately as **unread**, so a reading can't lower its loss by leaving singing out.
- **Per note, as Melodyne splits a note.** Melodyne is a concept here, not code.

  | Part | What it is | How it is measured |
  |---|---|---|
  | **centre** | where the note is held, against its key | mean residual over held frames, vibrato cancelled by a whole number of cycles where one is found, else the plain mean (`HELD_SECS`' reasoning in `notes.rs`) |
  | **drift** | a slow move across the held span | a least-squares line through the held residual: its slope (cents/s) and range |
  | **modulation** | vibrato | the residual less the drift line, as rate (Hz) and depth (cents, half peak-to-peak) where its autocorrelation peaks between 3 and 9 Hz; otherwise none |
  | **transition** | the slide in (scoop), and any fall at the end | the residual at `start` (cents) and the slide's length (ms); the same for a fall-off |
  | **rest** | what is left | the RMS of held residual less centre, drift and modulation |

- **Per take:**
  - the tuning and its spread;
  - the histogram of `centre` over notes;
  - the slope of `centre` over the take (drift of the reference);
  - the count of between-keys notes;
  - the **loss** (below).
- **What is written:**
  - The per-note parts and the per-take summary go in `.capture.json`, in a `reading` section.
  - The per-frame residual isn't written: it is `performance − reading`, and either file can rebuild it.
- **The loss:** `L = Σ_held r² / σ² + Σ_unread w + Σ_notes c`.
  - `σ` is the held-pitch tolerance.
  - `w` is the cost of a voiced frame left unread.
  - `c` is the **cost per note**: question 4 sets it and states it.
  - The loss is reported as its three terms, never as one number alone, so a reading that wins by adding notes shows it.

> **Built (2026-11-15, build task 4):** `duet_dsp::residual`, filled by `read`: each `ReadNote`'s optional `residual` and the `Reading`'s, `capture-analysis` 1.2 (additive). Where it departs from the above:
> - **The held span is found again from the residual,** not taken from segmentation: the slide in is the frames until the pitch first reaches the note's median, when the first frame is further from it than the vibrato's depth and 30 cents; a fall off is the same from the end. Segmentation's `held_from` is where it arrived *and held for 50 ms*, which a vibrato pushes late by up to a quarter of its cycle.
> - **The drift line and the vibrato are fitted together** (least squares on 1, t, sin, cos at the rate the autocorrelation finds): a line alone is pulled by a vibrato over a span of no whole number of its cycles, by about 12A/(T²ω): 5.7 cents a second for the witness's.
> - **The rate is the shortest lag that peaks above 0.5,** not the highest peak: a vibrato correlates as well at two periods as at one. A fitted depth under 5 cents is no vibrato.
> - **The loss's first term covers every voiced frame a note covers,** its slide and fall included, as `pitch`: a reading cannot hide frames from it by calling them slide. `σ` is 50 cents (half a key, so a frame that far costs one unread frame), `w` is 1, and `c` is the shortest note's frames (0.05 s, 4 frames) times `w`, so a note must account for at least that much singing that would otherwise be unread. Each is a named constant in `residual.rs`; the note HMM sets them on real takes.
> - **The tuning's spread is the reading's `tuning.spread_cents`,** not repeated in the take's residual.
> - **No ndarray yet:** one 4×4 solve and an autocorrelation over a few dozen lags per note.

**Witnesses:**
- A held note with a 6 Hz vibrato of ±40 cents, a 10 cents/s drift and a 300 ms scoop from 300 cents below is measured within 10 % on each part.
- A note with no vibrato reports none.

## 4. How notes are chosen over the loss

**What the pYIN Vamp plugin does (read in its source, 2026-11-15).**
- Its note model is a second HMM over the smoothed pitch track.
  - **States:** three per pitch (**attack**, **stable** and **silent**), on a grid of a third of a semitone from MIDI 35, 69 semitones wide.
  - **Observations:** attack frames are Normal around the state's pitch with σ = 5 semitones, so pitch barely matters there. Stable frames have σ = 0.8. Silent frames score the frame's unvoiced probability.
  - **Transitions:**
    - attack stays 0.9 and goes to stable 0.1;
    - stable stays 0.99 and goes to silent 0.01;
    - silent stays 0.9999;
    - from silent, a new attack is weighted by the interval, Normal(0, 0.7 semitones), allowing the same pitch or a jump between 0.5 and 13 semitones.
  - **Decoding:** fixed-lag Viterbi, as our pitch track already decodes.
  - **Notes:** runs of attack and stable frames. An amplitude-ratio test splits a note at a re-attack. A note's pitch is the median of all its frames.
- **The cost per note is explicit in it.** Leaving a stable state costs `−ln 0.01`, and a held, wobbly note stays one note because σ = 0.8 absorbs vibrato. That is exactly the trade-off the decision names.
- **The attack state is our slide.** It is pitch not yet committed, and it belongs to the note that follows it. That is the #28 ruling.
- **Its one mismatch with our rulings is legato.** Stable can only leave through silent, so a step between two notes with no breath has to be decoded through a silent state the voicing argues against. Tony adds hand-editing for this. We need a direct transition, stable(i) → attack(j), for j ≠ i.
- **Its grid is fixed on A440,** like ours today.

**Ruled (owner, 2026-11-15): the design chooses this structure, adapted. It is built as its own task, after a baseline.**
- States per key on the **take's tuning**, not A440, at one state per semitone.
  - The third-of-a-semitone grid exists to follow an unknown tuning, which question 2 has already estimated.
  - Each note's `centre` carries the fine pitch, so the decoder doesn't have to.
- **Attack** (σ wide) → **stable** (σ ≈ the vibrato tolerance) → either:
  - **silent**, for a breath or a dip: silent → attack on the same key is allowed, so a dip is two notes, by the #28 ruling;
  - or **attack of another key directly**, for legato.
- **The cost per note `c` is the log probability of leaving stable,** stated in the code as one named constant and in `.capture.json` as a number.
  - Raising it merges; lowering it splits.
  - Its value is set on the corpus and the owner's takes, never by the residual alone.
- **The owner's three rulings are witnesses of the model, not afterthoughts:**
  - **a note is held pitch:** stable is the only state that gives a `key`;
  - **a slide belongs to the note it arrives at:** attack precedes stable on the same key;
  - **a dip is two notes:** stable → silent or attack → stable on the same key, which costs `c`, paid because the voicing drop or the onset pays for it.
- **Onsets enter as evidence:** an onset raises the probability of leaving stable at that frame, as the amplitude-ratio test does in pYIN.

**The order (ruled with the method).**
1. **Restate first.** The current hand rules move behind the new layers, reading on the take's tuning. They are the baseline, with no other change, and the residual and loss are reported for them.
2. **Then build the HMM** as its own task. It is judged against the baseline on:
   - the loss's three terms;
   - the owner's ear, through the comparison tool;
   - a sung-note measure (COnPOff, as in Molina et al. and mir_eval) wherever an annotated corpus with a licence we can use exists.
   - Optionally, pYIN Vamp's own notes serve as an oracle, run outside the build like abc2midi, never linked.

**The trap, and its witness.**
- Judged by the residual alone, one note per frame is zero loss.
- **Witness:** a synthetic 2-second note with ±50 cents of vibrato at 5.5 Hz and 15 cents/s of drift reads as **one** note. It also reads as one note with `c` halved. The same signal with its pitch stepped a semitone halfway, held 0.5 s each side, reads as two.
- **A second witness:** for any reading the decoder returns, splitting one of its held notes in two raises `L`. So `c` is never zero in effect.

> **Built (2026-11-15, build task 6):** `duet_dsp::NoteModel`, chosen by `duet capture melody --reader hmm`; the rules stay the default. The reading records its reader and cost per note as `capture-analysis` 1.3's optional `reading.reader` (additive). Mauch et al. 2015 could not be fetched from this build's network, so the model is built from pYIN's source as described above. Where it departs from the above:
> - **The attack is pitch-agnostic, at a flat 0.5 nats a frame beyond a frame held on its key (`ATTACK_COST`), and its transitions are free.** pYIN's Normal(σ = 5 semitones) attack, which stays with probability 0.9, costs about 2.3 nats a frame more than holding. That made a 20-semitones-a-second scoop or glide read as a short note on each key it passed.
> - **Stable is a chain of 5 frames (0.05 s, the shortest note) before it may be left,** so no note is shorter than the rules allow.
> - **Leaving stable costs `c` whatever it leads to:** silence, or the attack or the stable state of any key, the same key included. pYIN weights the next note by its interval. So `c` is exactly what a note costs, and the model is a cost model whose rows need not sum to one. A note that starts on its pitch, after silence or legato, may enter stable directly with no attack.
> - **One silent state,** not one per pitch, since nothing here weights the interval.
> - **Onsets:** at the frame an onset falls in, leaving stable has probability 0.9 (`ONSET_LEAVE`) instead of `e^−c`. So an onset splits a held pitch as the rules do, whatever `c` is.
> - **Observations:** a voiced frame scores `ln v` plus its pitch's Normal density (σ = 0.5 semitones, `STABLE_SIGMA_SEMITONES`) in stable, and `ln(1 − v)` in silence, with `v` floored at 0.01 either way. A frame the pitch track decoded unvoiced scores `ln 0.05` (`DROPOUT`) in a note, so one lost frame stays inside its note and two end it.
> - **States cover all 128 keys,** so nothing about the take is needed in advance; the lag is 1 s (`LAG_SECS`), and on the witnesses the lagged decode equals the whole take's.
> - **`c` = 4 nats (`NOTE_COST`), shared with the loss's notes term.** It is the loss's own derivation (the shortest note's 4 frames times `w`) and close to pYIN's 4.6. It was set by sweeping `c` from 1 to 12 over the witnesses and the synthetic takes, never by the residual. Every take reads its expected number of notes for `c` from 2 to 8. Below 2, staying in a note costs `ln(1 − e^−c)` a frame (0.46 nats at `c` = 1), and every take shatters. At 12, the semitone step and a legato tune merge. **The owner's takes were not available to this build:** `c` has yet to be checked on them.
> - **Where it is worse than the rules:** it leaves the last, weakly voiced frame of a decaying note unread (one unread frame a note, or two between attacked notes). A glide slower than about 8 semitones a second (15 at `c/2`) reads as a note on every key it passes, where the rules read a few fewer.

> **Amended (2026-11-15, the listening findings' first task): sung frames are never cheaper to leave unread than to read, and the decoder and the loss price the same events the same way.** The owner's takes showed the HMM leaving 45–55 % of the voiced frames unread, as attack runs ending in silence, and the loss scoring that better, because `w` = 1 sat below a read frame's typical `r²/σ²`. What changes:
> - **Attack → silent is forbidden, and the take never ends in an attack** (the task's option b, over a: pricing an attack run that never holds "as unread" needs the decoder to know, at the run's first frame, whether it will hold, which is no Markov state). With the transition gone, the only way to leave voiced frames out is the silent state, and silent already prices each frame by its voicing, `−ln(1 − v)`. So an attack that never holds is priced as unread singing, frame by frame, which is what option a asks; b gets it with one transition fewer and no new state.
> - **One price per frame, defined once** (`residual.rs`: `sounding_cost`, `silent_cost`, `held_cost`, `unread_cost`), and the decoder scores frames by it. The loss takes each frame's price relative to the cheapest the frame could be, so every term stays at least zero, and becomes four terms, in nats:
>   - **pitch:** `d²/2σ²` a held frame (σ = 50 cents, the decoder's `STABLE_SIGMA_SEMITONES`), and a flat `ATTACK_COST` a slide frame. It was `r²/σ²`, twice the decoder's.
>   - **voicing (new):** for each frame a note covers, what sounding it costs beyond leaving it out: `max(0, ln((1 − v)/v) + ln(σ√2π))`, so a breath read as a note costs what its voicing says it is not; and `−ln DROPOUT` for a frame the pitch track decoded unvoiced inside a note, as the decoder charges.
>   - **unread:** `max(0, ln(v/(1 − v)) − ln(σ√2π))` for each voiced frame no note covers, not 1. That is about 2 nats at v = 0.9 and 4.4 at the floor's 0.99, and nothing below v ≈ 0.56, so breath and consonants stay free to leave out.
>   - **notes:** `c` a note, unchanged.
> - **What "never cheaper" means exactly:** a frame voiced above about 0.67 costs more unread than read anywhere within half a key, held or as slide. A frame voiced below about 0.56 costs more read than unread. Between the two, the voicing itself is in doubt, and the decoder and the loss weigh pitch against it in the same way.
> - **Slide frames are measured as slide.** The frames a note covers before its `held_from` cost the flat `ATTACK_COST`, as the decoder's attack does, and not `d²/2σ²` against the key they arrive at. They stay the note's, by the #28 ruling. The reading's own `held_from` says which frames are slide, not the residual's measured scoop: for the HMM, that is exactly its attack, so the decoder and the loss agree frame by frame. The residual's re-found held span would call a dip's first frames held, since they start on the pitch. The guard against hiding pitch as slide is that a slide frame still costs `ATTACK_COST` and gives no key. Neither reader moves `held_from` to lower the loss: the HMM's is its first stable frame, and the rules' is where the pitch arrives and holds for 50 ms.
> - **Where the decoder and the loss still differ** (named on `Loss`):
>   - onsets: the decoder leaves a note at an onset for `−ln 0.9` rather than `c`. The loss has no onsets, so any reading of a performance can be scored.
>   - staying held: the decoder charges `−ln(1 − e^−c)`, 0.018 a frame at `c` = 4. The loss leaves it out.
>   - dropped notes: a decoded note shorter than the shortest note, or less voiced than the rules allow, is dropped from the reading as the rules drop it, and its frames are unread in the loss.
>   - the rules' `held_from` is theirs.
> - **`c` stays 4,** though its derivation (4 frames × `w` = 1) no longer holds: an unread frame at v = 0.9 now costs about 2, so 4 is two such frames' worth. It is re-judged by ear through `duet-listen`, after this change, not by note counts. On the synthetic takes, every take reads its expected notes for `c` from 2 to 8 (and to 12, but for the legato tune), as before. The HMM still leaves the last, weakly voiced frames of a decaying note unread (one to seven a take), and their unread cost is nothing.
> - **Witnesses:**
>   - a note held 45 cents off its key, bounded by weakly voiced consonants, is read;
>   - a short note between two breaths is read: 0.08 s on its key, and 0.15 s held 33 cents flat. 0.15 s on its key was already read before this change, since on its key nine frames pay a note's cost;
>   - a breath is left unread;
>   - dropping a held note raises the loss, on its key, 45 cents off, or scooped;
>   - a breath is cheaper left out than read;
>   - dropping any note the HMM reads, on every synthetic take, raises the loss.
> - **Format:** `capture-analysis` 1.4, additive in shape: `loss.voicing`, taken as 0 when absent. `pitch` and `unread` are priced anew, so a 1.3 loss and a 1.4 loss are not compared term by term.

## 5. Transformations

**Proposed.** A transformation maps a `Reading` (with its residual) to a `Reading`. It never touches the performance file.

| Name | What it changes | Asked by |
|---|---|---|
| **transpose** `n` | every `key` by `n`; residual unchanged | `--transpose n` |
| **retune** | the tuning to a target (default A440); keys and residual unchanged, so a performance render sounds at the target | `--retune 440` |
| **correct** `k` | scales each note's `centre`, `drift` and `rest` by `k` (0 = flat on the key, as hard auto-tune; 1 = as sung); vibrato and slides kept | `--correct 0.5` |
| **snap to key** | moves *between keys* notes, and only them, to the neighbour in the key's scale, and states each move; a note held within 35 cents of an out-of-key semitone stays chromatic | `--snap-to-key` |
| **reshape** | scales modulation depth, or slide length | later; named, not built |

- **The rule stays:** nothing is snapped unasked, and every transformation applied is stated in the output files and on stderr.
- **A score renders the reading alone.** Its keys go through the existing `.abc` writer. The tuning is stated as a comment, since ABC has no field for it.
- **A performance renders the reading plus residual,** through `Control::Bend` on the melody track. The `.notes` and MIDI writers already carry it, and `duet play` already sends bends to a port.
- **The bend range is the limit.**
  - A receiver's default is ±2 semitones, which a scoop from three semitones below exceeds.
  - The render sets ±12 with RPN 0 at the track's start: CC 101 = 0, CC 100 = 0, CC 6 = 12, CC 38 = 0. These are ordinary `Control::Cc` lines, so no writer changes.
  - At ±12 a bend unit is 0.15 cents.
  - Bends are emitted where the residual moves more than 2 cents, at most one per frame (86 a second).
  - Bend is channel-wide, which is right for a monophonic melody.
  - **Not known yet:** whether Logic's instruments and `duet play`'s built-in synth honour RPN 0. That is a build-time check by ear. If one doesn't, it is a stated limit, not a silent wrong pitch.
> **File names (2026-11-15, [duet documents](2026-11-15-duet-documents-design.md)):** capture's `.notes` output becomes a song document, `<stem>.duet`, and its text is the `notes` view; `.capture.json` becomes `<stem>.capture.duet`.

- **The default `.notes`** stays the reading's keys, unquantized, as today, with the tuning as a comment. The performance render is asked for (`--render performance`).
  - The anti-goal "losing the performance" is met by `.performance.jsonl` itself, not by bends in `.notes`.
  - **Proposed; not yet ruled.**

**Witnesses:**
- transpose then transpose back is the identity;
- retune to the given tuning is the identity;
- correct 1 is the identity, and correct 0 leaves every held frame's residual equal to its modulation alone;
- snap to key moves no note within 35 cents of its key;
- a performance render, read back from `.notes` and its bends converted back to cents, gives each held frame's pitch within 1 cent of the performance.

## 6. The comparison tool

**Proposed:** `duet capture compare <stem>`. It reads `<stem>.performance.jsonl` and `<stem>.capture.json`, or a transformed reading, and writes:
- **`<stem>.compare.svg`:** the performance as a line in cents over time, the reading's notes as bars at `tuning + 100·key`, the residual shaded between them, between-keys notes marked, and onsets as ticks. It is hand-written SVG, with no dependency.
- **`<stem>.compare.wav`:** the take in the left channel, and the reading rendered legato in the right as a sine (one continuous tone, gliding only through slides) at the take's own tuning. This is the 2026-11-15 listening method, made a command. It needs no soundfont.
- **On stdout:** one line per note (time, key, centre, drift, vibrato, scoop and rest), then the loss's three terms.
- **`--from` and `--to`** select a window, so a named moment, such as Blood Sport at 33 s, can be looked at alone.
- **`--reading other.capture.json`** compares two readings of one performance. This is how the baseline and the HMM are judged against each other in question 4.

**Witness:** the e2e test runs `compare` on a synthetic take and checks that the SVG has one bar per note and that the WAV's right channel is within 1 cent of each held note.

## 7. The crate boundary

**Proposed:**
- The layer 1 types (`Layer1`, frames, the performance file's reader and writer), the reading (`Reading`, `ReadNote`, tuning, the note decoder) and the residual go in **`duet-dsp`**: they are analysis.
- Transformations go in `duet-dsp` too, since they work on a reading, not on a `Song`.
- `duet-capture` keeps sources, `Prepare`, assembly to a `Song`, and the files.
- **Ruled (owner, 2026-11-15): the split goes first,** as its own small task with no behaviour change. Building the new types into `duet-capture` and moving them a week later would mean two moves.

## 8. Rhythm takes

**Named, and out of scope for this design's build.** A rhythm take has the same two layers:
- **Layer 1:** per-band onset envelopes, the spectral flux per frame in low, mid and high bands. They stream, and they are what the "unce" task needs.
- **Reading:** hits with lanes.
- **Residual:** envelope energy no hit explains (a hat inside a kick's "unce"), and each hit's offset from the grid.

The melody is where the owner has listened and ruled. The rhythm reading can wait until the "unce" task (temper `01a1dcff-5cde-7512-a5c9-b5f945c32bed`) is taken up, and should be designed with it.

## Build tasks this design names

Each task is filed once the owner has ruled. Each names its witness, and none is built before its predecessor.

1. **Split duet-capture into duet-dsp and duet-capture** (the existing task). No behaviour change.
2. **Layer 1 written:** `Performance` emits frames, and `<stem>.performance.jsonl` is written and read back. Witnesses: question 1.
3. **The reading on the take's tuning, with the rules restated:** tuning given or estimated; notes by the current rules, read on it; key after. Witnesses: question 2.
   - This **replaces** the held task *a sung note's key is read against the singer's own tuning, not A440*. Its acceptance criteria carry over unchanged.
4. **The residual and the loss:** the per-note split and per-take summary, written in `.capture.json`. Witnesses: question 3.
5. **The comparison tool:** `duet capture compare`. Witnesses: question 6. The owner listens to the restated baseline here first.
6. **The note HMM:** the decoder of question 4, judged against the baseline. Witnesses: the trap and the three rulings.
   - This **replaces** the held task *an expressive rise and fall inside a sustained sung note reads as the owner hears it*. Its named moments become this task's listening set.
7. **Transformations:** transpose, retune, correct, snap to key, and the performance render with its bend range. Witnesses: question 5.
   - This **replaces** the held task *a sung melody's notes are read in the key it is sung in*. Snap to key is its answer.

Tasks 4 and 5 can be built together. Task 7 needs only 3 and 4, not 6.

## Amendments to the capture design

- **Melody, notes:** "Each note's pitch is the median of its voiced frames, rounded to the nearest semitone" becomes: "a note's key is the nearest key, **on the take's tuning**, to its held pitch, and its held pitch is kept as `centre` in cents".
- **Key:** "The key changes spelling and signature only, never a MIDI note" **stays, for the reading.** Snap to key is a transformation, asked and stated, so the rule "nothing is snapped unasked" holds.
- **The stream rule:** the reading joins `estimate` and `assemble` as a function over settled output. It keeps frames, never samples.
- **Output:** capture also writes `<stem>.performance.jsonl`, under the same "the performance is always written" and "never overwritten" rules.
- **Tempo, meter and downbeat** gain a sibling: the tuning is given or estimated and stated, under the same rules (`an-inferred-tempo-or-meter-is-stated` extends to it). This needs a goal amendment, approved by the owner.

## Not decided here, and why

- **A drifting tuning reference:** deferred until a real take needs it (question 2).
- **The note HMM's constants:** set in its build, on the corpus and the owner's takes, never by the residual alone.
- **A sung-note corpus with annotations:** its licences are unchecked. This belongs with the reference-corpus task, widened from beats to notes.
- **Polyphony:** the decision says a model stage fills layer 1 too. basic-pitch's salience curve isn't a single pitch per frame, so layer 1 then needs more than one pitch per frame. That waits for the Later goal, and nothing here closes it off: the header names the format.
