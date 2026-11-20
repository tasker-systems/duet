# The capture reference corpus

Short recordings (FLAC or Ogg, a few seconds), each with a hand-checked transcription.
`tests/corpus.rs` runs every take in `manifest.toml` through the library pipeline and compares
the result with the transcription; read the manifest's header comment for the rules. The
takes witness the three capture goal clauses, and only takes recorded by a person do:
`synthetic` takes (made by `scripts/capture-fixtures.sh`) prove the harness bites and count
for nothing else.

## Adding a take

1. Put the recording beside `manifest.toml`.
2. Draft the transcription with `duet capture`, into a scratch directory so nothing lands in
   the corpus, and with the same `tempo`, `meter`, `downbeat` and `key` you will put in the
   manifest (they change where notes land, so a draft made without them differs):

   ```sh
   duet capture melody take.flac --tempo 96 --meter 4/4 --downbeat 0.4 --key G -o /tmp/draft/take
   duet capture rhythm take.flac --lane hh --tempo 96 -o /tmp/draft/take   # tapped
   duet capture rhythm take.flac --tempo 96 -o /tmp/draft/take             # beatboxed
   ```

   `-o` is a stem: it writes `take.notes`, the score and `take.capture.json` there, and never
   overwrites. The `.notes` is the exact performance; it is the file to start from.
3. **The draft is not the answer.** It is what the pipeline produced, so a test that compares
   the pipeline with it can never fail. Listen, and check every note against the audio: its
   pitch, the step it starts on, and for a drum take the drum of every hit. Correct the file,
   delete what the take does not have, and add what it has. Only then copy it beside the
   recording as `<name>.notes`.
4. Add a `[[take]]` to `manifest.toml`: `file`, `kind` (`melody`, `tapped` or `beatboxed`),
   `provenance` (who recorded it and how), `licence`, `expected`, and the optional `tempo`,
   `meter`, `downbeat` (seconds into the file), `key` and `grid`. A tapped take needs `lane`;
   no other kind may have one.

## What the comparison reads

- **Melody:** notes of at least an eighth (0.5 quarter notes) must each have a found note on
  the same step with the same pitch, and a found note of at least an eighth where the
  transcription has none fails. Shorter notes are reported only, so a stray grace note
  costs nothing.
- **Drums:** in a drum `.notes`, each hit is a note on the drum track (channel 10) whose pitch
  is the lane's MIDI key in the `gm` lane map: `C2` (36) kick, `D2` (38) snare, `F#2` (42)
  closed hi-hat (`hh`). A beatboxed take is marked lane by lane, so a hit in the wrong lane
  fails. A tapped take's hits are all the asked lane's key. The comparison is the set of
  (step, key): a missing hit and an extra hit both fail. Lengths and velocities are not
  compared.
- **The grid:** both the capture and the transcription are quantized to the take's `grid`
  (default `1/16`) before comparing, and a note's step is where it lands. Write the
  transcription on the grid you will compare at (positions are `bar.beat.tick`, 480 ticks to
  the quarter, so a sixteenth is 120 ticks), and use `grid = "1/8t"` for a triplet feel.

## Listened takes

A `[[listened]]` entry in the manifest is a recording the owner has judged by ear but not
transcribed note by note: `wayfaring-stranger.flac`, the owner singing the traditional song,
is the first. Its test holds only what the owner agreed (the key, the tuning, and the number
of notes, each within a tolerance), so it catches a reader change that moves the take a long
way, and says nothing about any one note. Its `digest` pins the file as the take: the SHA-256 of
its decoded samples, as `duet capture` states it in the analysis's `take`, so a file changed
in any sample fails by name, and a re-encode that keeps every sample does not. It is licensed CC BY 4.0, not MIT; the notice is
`wayfaring-stranger.flac.license`, beside it. `wayfaring-stranger.moments`, also beside it,
names the moments the owner rules on by ear, for `duet capture review --moments`.

## Outside transcriptions

`wayfaring-stranger.logic-flex-pitch.mid` is the same performance transcribed by another tool:
Logic Pro's Flex Pitch, from the 48 kHz original, exported unedited. It is a reference to set
duet's readings and key estimate beside, not an answer: Flex Pitch makes its own choices, and
splits the take into more and shorter notes (52) than any of duet's readers. It is CC BY 4.0,
as the recording is; the notice is `wayfaring-stranger.logic-flex-pitch.mid.license`.
`tests/flex_pitch.rs` reads it through duet-midi and holds it to the take: its note count, and
where its first note starts and its last note ends against the HMM's reading of
`wayfaring-stranger.flac`, so a different export, or one shifted in time, fails.

## Takes not yet read by a test

`simple-beat.flac` is the owner beatboxing a simple beat, recorded 2026-11-17 with `duet record`
(48 kHz 24-bit mono, 11.2 s). It is here so a test can be written against it; none reads it yet,
since a beatboxed `[[take]]` needs a hand-checked `.notes` and has none. Its digest, as
`duet capture` states it, is
`sha256:0daefbc35a7366706f79be1c23d1f12774166fc81fbb15d6dd427f2fc7aff74b`, and the same digest
computed apart from duet over libFLAC's decode agrees. The committed file is the reference
`flac -8` encoding of the recording, with every sample kept: `duet record` wrote one block,
4 096 frames from frame 430 080 that clip at full scale, as a 100 MB frame, and duet reads that
frame back as silence, so the original's digest is
`sha256:1efda451c159fb3954ddb07e1508e13baadd549edb1df119aaf810baa595bf06`, not the take's. It is
licensed CC BY 4.0, not MIT; the notice is `simple-beat.flac.license`, beside it.
