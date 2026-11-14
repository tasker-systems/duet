# `duet capture`: design

**Date:** 2026-11-14. **Goal:** "Now: a sung or tapped idea in an audio file becomes ABC or a drum grid we can discuss, and goes back to MIDI that opens in Logic" (temper `01a1da53-99fa-706b-bc13-a7c9ef1afaf4`). **Decisions it rests on:** capture starts from audio files (`01a1da53-5b19-74cf-8378-1f9fb01f9b60`); tempo and meter are optional inputs (`01a1da95-74a6-756a-807c-f0a88f06027e`).

## Intent

Capture was the reason duet exists. The owner records an idea on the Mac or a phone, whether sung, hummed, tapped, beatboxed or a single-note guitar line. Today it is trapped in an audio file that neither he nor Claude can read. `duet capture` turns it into the text we already share. A melody becomes ABC, and a rhythm becomes a `.drums` grid. Both already render to MIDI for Logic.

Everything downstream of a `Song` exists and is witnessed: quantize, the ABC writer and the drum-grid writer. Capture's job is to produce a faithful, **unquantized** `Song` from audio, and then hand it to that existing code.

**Capture is a stream from the first line of code.** v1 reads files, but the core never assumes it has the whole take. Two things come next: a live audio source (cpal), and an MCP server that streams what capture hears to a running LLM, so an idea can be worked on while it is played. Both are *additions*: a new source, and a new consumer. Neither reworks file processing. Large files are streamed for the same reason.

## Goals (v1)

- `duet capture melody <source>`: a monophonic take becomes a `.notes` performance, plus ABC.
- `duet capture rhythm <source>`: a tapped or beatboxed take becomes a `.notes` performance, plus a `.drums` grid.
  - **Beatboxed** hits are sorted into three lanes: kick, snare and hat (GM lanes `kick`, `snare`, `hh`).
  - **Tapped** hits go to one lane, named with `--lane`. Without `--lane`, a rhythm take is read as beatboxed. *(Builder's call: a default lane would make the two modes ambiguous.)*
  - Takes captured on different lanes are meant to compose into one groove. v1 doesn't merge them: a later `drums merge` does. v1 makes sure every take can be given `--tempo`, `--meter` and `--downbeat`, so takes line up when merged. *(Builder's call; the owner may revisit.)*
- WAV, MP3, Ogg Vorbis, AAC/M4A and FLAC are decoded through Symphonia, with its `mp3`, `aac` and `isomp4` features on.
- `--tempo` and `--meter` are optional.
  - **Given,** a value is used exactly.
  - **Left out,** it is estimated and applied, and the output states that it was estimated, with the value. For tempo it also gives the half-time and double-time alternatives.
- The analysis is a chain of **stages** behind one trait. A stage backed by an ONNX model can later replace or join a DSP stage without changing the chain.

## Non-goals (v1), each named for where it goes

| Not in v1 | Where it goes |
|---|---|
| Live audio input (cpal), OS audio sources | Next piece of work, as a new `Source` |
| The MCP streaming server (rmcp, tokio) | Next; it consumes the chain's events over an mpsc channel, as tasker-core bridges Python, Ruby and napi |
| Polyphony and chords (guitar chords, mixes) | The Later goal, as an ONNX stage such as basic-pitch |
| Stems, chord suggestion, bassline suggestion | Later; the last two work on a `Song`, not audio |
| Telling tap timbres apart (knuckle "tok", clap, stomp "thump", "tap") | After v1, as a better classifier stage |
| Tempo that changes within a take (rubato, accelerando) | After v1; v1 estimates one tempo per take |
| A take that mixes melody and rhythm | Stays unexamined in the goal's closure |
| Audio-graph crates (`dasp`, `fundsp`, `knyst`) | Not excluded for other uses; not the core of this one |

## Anti-goals

These make capture a failure even if it runs:

- **A silently wrong tempo or meter.** Estimates are always stated, never just applied.
- **Losing the performance.** The unquantized `.notes` file is always written, and quantizing never replaces it (`no-quantizing-happens-unasked`).
- **Overriding a given tempo or meter** (`no-provided-tempo-or-meter-is-overridden`).
- **A core that only works on a whole file.** A stage that buffers the entire take to do its work, other than the final assembly into a `Song`, is a design failure. Live input would then be a rewrite.
- **A model runtime, a model download, or Python** in v1.
- **Dropping an undecodable file quietly.** HE-AAC, which Symphonia cannot decode, and any unknown codec are refused by name (the goal's refusal face).
  - **Declared HE-AAC** (object type 5 or 29 in the header) is refused by name.
  - **HE-AAC signalled only inside the audio data** decodes quietly as AAC-LC, at half bandwidth. It is detected by comparing the declared and decoded sample rates, if that proves cheap. Otherwise it is recorded as a known boundary in the crate docs and the goal's evidence. *(Builder's call.)*
- **Losing notes before the downbeat.** No note is ever dropped from a score (owner, 2026-11-14). See *Tempo, meter and downbeat*.

## Constraints

- **Licences:** every new dependency passes `deny.toml`'s allowlist (`no-dependency-outside-the-licence-allowlist`).
  - Symphonia is MPL-2.0, which is allowed.
  - `rustfft` and `rubato` are MIT or Apache-2.0.
  - aubio, aubio-rs and bliss-audio are GPL and excluded.
- **No Temper dependency** (`no-temper-dependency`).
- **`no-capture-requires-live-input`:** every path works from a file alone.
- **The format promises exact round-trips** (CLAUDE.md). Capture *writes* `.notes`, `.abc` and `.drums` through the existing writers, and adds no writer of its own.
- **The domain split:** a new crate, `duet-capture`. The CLI gains a `capture` command group, following the decision that commands are domain groups plus a universal `convert`.

## Approaches considered

**A. Our own small synchronous stage pipeline (chosen).**
- **How it works:** a stage is a plain trait. It is pushed a chunk of input, appends whatever output it has *settled*, and is told when the stream ends so it can flush. The owner of the loop drives the chain: the CLI's file reader in v1, a thread fed by cpal's callback later, an MCP task reading a channel later still. Async lives only at those edges.
- **Why:** determinism. The same file gives the same events every run, so a reference take is a real test. A model stage is just one more implementation.

**B. Async streams throughout** (tokio, `futures::Stream`).
- **Rejected:** async would run through every stage and test, and timing would depend on the scheduler. Its one gain, an MCP fit, is reached just as well by A plus a channel.

**C. An existing audio-graph crate.**
- **Rejected for this work:** those crates are built for synthesis, where audio goes out, not analysis, where events come in. Event outputs, delayed settling and a model stage would all fight the framework. They stay available for other uses.

## The design, as decided

### Shape

```
Source ──chunks──▶ Prepare ──mono chunks──▶ [analysis stages] ──events──▶ Assemble ──▶ Song
(Symphonia)       (downmix,                  melody: pitch + onsets → notes       (needs tempo;     ├─▶ .notes (exact)
                   resample)                 rhythm: onsets → hits → lanes         buffers events)  └─▶ quantize → .abc / .drums
```

- **`Source`** yields chunks of interleaved samples with their rate and channel count, and reports end of stream. v1 has `FileSource` over Symphonia, pulling packet by packet so it never holds the file in memory.
  - Its error names the codec it cannot decode, and the file.
  - A live source is the same trait later.
- **`Stage`**: `push(&mut self, input, out: &mut Vec<Output>)` and `finish(&mut self, out)`.
  - A stage may hold input back until it is settled: pYIN-style smoothing settles a note a fraction of a second late, and onset peak-picking needs a few frames of lookahead.
  - A stage reports its worst-case settling latency, so a live consumer knows how far behind real time the events run.
  - Times inside the chain are **sample frames at the analysis rate**, not musical positions. Musical time doesn't exist until a tempo does.
- **`Prepare`** downmixes to mono and resamples to one analysis rate, chosen by the builder (22 050 or 44 100 Hz), with `rubato`. Every stage after it can assume one rate.
- **Events.** The chain's vocabulary is a `#[non_exhaustive]` enum, so model stages and the MCP consumer can add to it:
  - `Onset { frame, strength }`
  - `Note { start, end, midi, confidence, velocity }`
  - `Hit { frame, lane, strength }`
  - `TempoEstimate { bpm, alternatives, confidence }`
  - `MeterEstimate { … }`
  - `DownbeatEstimate { frame }`
- **`Assemble`** is the one stage that keeps the whole take, and only as events, never samples. It needs the tempo and the downbeat to turn frames into bar.beat.tick:
  - It builds a `Song` at the default 480 ticks per quarter, with one track: `melody`, or the drum track on channel 10.
  - For files it converts at `finish`. For live input later, it converts provisionally under the tempo so far, which is what the MCP stream will send.
- **Output.** Capture writes `<stem>.notes` (the exact, unquantized performance) and either `<stem>.abc` or `<stem>.drums`. The score file is the `Song` quantized with `Song::quantized(grid)`.
  - `--grid` sets the grid; the default is a sixteenth.
  - `-o` sets the output stem; the default is the input's stem beside it.
  - An output file that already exists is refused, never overwritten, because the acts never change their inputs and nothing is overwritten.
- **Velocity** comes from onset strength, mapped to 1–127. The `.notes` file keeps those raw values.
- **Velocity in the scores is snapped, and the snap is stated** (owner, 2026-11-14).
  - Both score writers refuse off-level velocities by default ("nothing is snapped unasked"). Asking capture for a score counts as asking for the snap.
  - ABC is written with `Dynamics::Nearest`, which already states the snap in a comment.
  - The grid is written from `DrumGrid::snapped_to_levels`, and capture adds a `#` comment saying it was snapped. The `.drums` writer has no such comment today.
  - The raw dynamics are never lost, because `.notes` holds them.

### Melody

- **Pitch:** a streaming YIN/pYIN-style tracker of our own on `rustfft`.
  - Each frame gets a fundamental-frequency estimate and a voicing probability.
  - It is smoothed with fixed-lag decoding (Viterbi over a window that settles and slides), not decoding over the whole take.
  - The `pyin` crate (MIT) is **batch only**: `PYINExecutor::pyin` takes the whole signal. It cannot be a stage. It may be a **dev-dependency** used to check our tracker on files, since its output is the reference pYIN's.
- **Notes:** voiced runs are split at onsets and at pitch changes held long enough to count. Each note's pitch is the **median** of its voiced frames, rounded to the nearest semitone.
  - The median is the declared reading for vibrato and slides, which the goal lists as unexamined.
  - The builder sets the thresholds: the shortest note, and how long a pitch must hold to start a new note.
- **Key:** v1 names no key, and the ABC writer's default applies. Estimating a key is a candidate for later, stated if it is ever applied.

### Rhythm

- **Onsets:** spectral flux with adaptive-threshold peak-picking.
- **Lanes:** with `--lane <name>`, every hit goes to that lane (tapping). Without it, a classifier stage sorts hits into `kick`, `snare` and `hh`:
  - v1 uses band energies and spectral centroid in a short window after each onset;
  - the thresholds are tuned on the owner's reference takes.
  - Lane names resolve through the configured `LaneMap`, so the user's or project's aliases apply.

### Tempo, meter and downbeat

- **The unit is quarter notes per minute, always.** That's how `Song` and ABC's `Q:1/4=` already count it, and it applies to `--tempo` and to the stated estimate alike. In a compound meter the statement also gives the felt beat, so a 1.5× mix-up can be seen: `tempo: estimated 135 bpm (dotted quarter = 90)`. *(Builder's call.)*
- **Tempo, given:** used exactly. A melody take's `--tempo` must be a whole bpm, because the ABC writer writes `Q:1/4=<n>` and refuses anything else. A fractional one is refused before analysis starts, naming that reason. Rhythm takes accept any tempo. *(Builder's call.)*
- **Tempo, left out:** estimated from the onset-strength signal by autocorrelation, or an inter-onset-interval histogram (the builder's choice). The estimate is **rounded to a whole bpm** before it is applied and stated, and is given with the half and double tempos as alternatives. *(Builder's call.)*
- **Meter, given:** used exactly. **Left out:** estimated from accent periodicity, choosing between 4/4, 3/4 and 6/8. When the evidence is weak it applies 4/4 and states it as a low-confidence estimate. Weak estimates are left open by the tempo decision, and this is the v1 answer.
- **Downbeat (where bar 1 starts)** is inferred too, because a take usually starts before or after its first bar line.
  - v1 places the first strong onset on beat 1, or on the beat the meter estimate favours.
  - It states the offset applied, the same way as tempo. A `--downbeat <seconds>` option overrides it.
- **Notes before the downbeat** (owner, 2026-11-14). A `Song` can't hold a position before `1.1.0`, so:
  - **If they fit a pickup,** they become one: a short first bar whose length the ABC writer accepts as a pickup meter (`pickup_meter`), followed by a time change at bar 2.
  - **Otherwise,** bar 1 becomes a full bar of rests that holds them, and the downbeat bar becomes bar 2.
  - Drum grids number bars from 1 with no gaps, so a rhythm take always gets the full lead-in bar.
  - **Nothing is ever dropped.** The output states which was done.
- **How estimates are stated:**
  - on stderr, one line each: `tempo: estimated 92 bpm (half 46, double 184)`;
  - and as comment lines in the written `.notes` and score file, so the files themselves say so (`an-inferred-tempo-or-meter-is-stated`).
  - `.notes` and `.drums` comments start with `#`; ABC comments start with `%`.
  - A given value is not reported as an estimate.

### Where a model fits later

basic-pitch (Apache-2.0, ships an ONNX export) would be a stage: mono chunks in, `Note` events out, run through `ort`. It replaces the pitch and note stages for polyphonic takes, and `Assemble` doesn't change.

v1 does not build this. It only keeps the trait able to hold it: a stage may own its own buffering and report its latency.

ONNX-only is the starting point. If a model we want ships only PyTorch weights, burn and candle are the next places to look, before a Python sidecar.

## Witnesses

| Clause | How v1 witnesses it |
|---|---|
| `a-sung-melody-file-reaches-abc` | A committed reference corpus of the owner's takes, each with a hand-checked expected `.notes` or `.abc`. The test checks pitch to the semitone and onset to the nearest grid step, for every note of at least an eighth. |
| `a-tapped-rhythm-file-reaches-a-drum-grid` | The same, for rhythm takes. **Amended** (see below) to include the lane for beatboxed takes. |
| `an-inferred-tempo-or-meter-is-stated` | Capture with each of tempo and meter left out, in turn, then both: the output files and stderr carry the stated estimate. With them given, no estimate is stated. |
| `no-provided-tempo-or-meter-is-overridden` | A given tempo and meter appear exactly in the written `Song`, including on takes where the estimate would differ. |
| `no-capture-requires-live-input` | Every test runs from files. |
| Refusal face (undecodable audio) | An HE-AAC file and a non-audio file are refused, naming the file and the codec. |
| `no-quantizing-happens-unasked` (extended) | The `.notes` written by capture is unquantized: its onsets are the detected ones, not grid steps. |

**Synthetic witnesses come first.** Before the owner's takes exist, each stage is tested on signals generated in the test:
- sine and harmonic tones at known pitches and onsets;
- click tracks at known tempos and meters;
- band-limited noise bursts standing in for kick, snare and hat.

These pin down behaviour deterministically, and they fail against an unbuilt stage. The owner's takes then witness the clauses.

**The reference takes are committed** (owner, 2026-11-14), so CI runs the clause witnesses.
- They are short (a few seconds) and small (FLAC or Ogg), under `crates/duet-capture/corpus/`.
- A manifest gives each take's provenance and licence, and its hand-checked expected transcription.
- The owner chooses which takes go public. The witnesses run on those.

**Determinism:** a run is deterministic on one machine. Floating-point differences across platforms can move a value by a hair. So tests compare at the clause's own tolerance (a semitone, a grid step), never exact samples.

## Goal amendments this design needs

The owner approves each one before it is made, through the goal's amendment discipline:

1. **`a-tapped-rhythm-file-reaches-a-drum-grid`** gains its lanes:
   - a beatboxed take's hits land in the right one of kick, snare and hat;
   - a tapped take's hits land in the lane asked for.
2. **The goal's "unexamined" list:** vibrato and portamento move to "examined and specified", read as the median pitch of each note.
3. **The goal's stated silence** gains the downbeat: how it is inferred is left open, but it is stated like tempo and meter.

## Left to the builder

- The analysis rate; frame and hop sizes; the onset and voicing thresholds; the shortest note.
- Autocorrelation or IOI histogram for tempo, and the meter heuristic's details.
- The classifier's features and thresholds, tuned on the reference takes.
- The exact trait signatures and event fields, within the shape above.
- How the reference corpus is laid out, following `crates/duet-abc/corpus/manifest.toml`'s pattern: a manifest with provenance per take.
