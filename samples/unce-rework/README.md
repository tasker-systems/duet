# Unce rework: a weak take, and the beat it became

This is a duet project. It holds a 15-second freestyle beatbox take that is, in the owner's
words, "not my best work". It also holds the song that take was reworked into, step by step.
Each step is a snapshot. A weak take is the material duet is for: it is captured, read and
reworked until it is something worth keeping. This sample shows that.

The take is `takes/unce/unce.flac`, recorded by the owner on 2026-11-20 at 24 kHz, mono, and
licensed CC BY 4.0 (`unce.flac.license`). The rest of the project is under the repository's MIT
licence.

## The steps

Run each from this folder. `duet play` sounds it through the built-in synth.

| Step | Snapshot | What changed | Heard |
|---|---|---|---|
| Take | `takes/unce/unce.flac` | Freestyle, with no set tempo or pattern | The owner |
| Reading | `takes/unce/unce.rhythm.duet` | `duet capture rhythm unce`: 125 bpm (confidence 0.45, weak), the downbeat at 6.17 s (weak), and mostly snares, with 5 kicks and 4 hats | The owner: "a pretty accurate capture". The kicks were right. The "ce"s came out as hats and snares, which the owner put down to their own enunciation |
| v0 | `v0-quantized` | The reading's grid, quantized to 1/16 (`duet drums render`), at 125 bpm | Played |
| v1 | `v1-loop` | Bars 4 and 5, the strongest stretch, looped four times. A steady 8th-note hat runs under them, louder on each beat, in place of the unclear "ce"s | Played |
| v2 | `v2-groove` | 98 bpm. Kicks are added on 1 and 3 of bar 1, and on 1 and the "and" of 3 in bar 2, beside the take's own two. The snares are thinned to a backbeat on 2 and 4, with ghost notes that move each time through. A snare fill ends it | The owner: "I'd listen to that beat" |

```sh
duet play takes/unce/unce.rhythm.duet                                   # the reading, as performed
duet play songs/unce-rework/snapshots/2026-11-21-v0-quantized/song.duet
duet play songs/unce-rework/snapshots/2026-11-21-v1-loop/song.duet
duet play unce-rework                                                   # the working song: v2
duet view songs/unce-rework/parts/v2-groove.duet --as notes             # what any step holds, as text
```

The song's composition carries each step's tempo, so each snapshot plays at the tempo it was
heard at. `crates/duet-e2e/tests/sample_unce_rework.rs` checks three things: that the take
matches its digest, that each step reads, plays with no disagreement and restores. Nothing
checks how a step sounds. That was the owner's judgement, by ear.

## What this does not show

- **This is not a capture corpus take.** No hand-checked transcription of the freestyle exists,
  so it witnesses nothing about the capture pipeline's accuracy.
- **v1 and v2 were written as notes text by Claude, from the owner's directions.** They are
  not the output of a duet command. The directions were: loop bars 4–5 with a hat; then slower,
  thinner and more varied snares, and more kick.
