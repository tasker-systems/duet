# Placing takes on a song's bars — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A song links the project's takes and places each on its bars (`click`, `estimated` or `set`), through `duet song link | place`, `duet record --song` and capture, which states where its reading disagrees with the song.

**Architecture:** The document shape and its checks go in `duet-schemas` (`song.rs`). The rules that need no device go in a new `duet-actions/src/placing.rs`: reading an estimate from an analysis, whether capture may replace a placement, and the disagreement statements. They are wired into `song.rs` (link, place, show, snapshot), `capture.rs` and `take.rs`. The click and the clock arithmetic are a pure submodule of `record`, `record/click.rs`, and `record.rs` opens the output stream beside the input.

**Tech Stack:** Rust, cpal 0.18.2 (already a dependency of `duet-actions`), serde/schemars, clap, the `duet-e2e` harness.

**Spec:** `docs/superpowers/specs/2026-11-21-placing-takes-design.md`, the build design. Read it first, with `docs/superpowers/specs/2026-11-17-songs-and-composition-design.md` §1 ("The song") and §4, and temper decision `01a1fe65-a864-72ad-9543-36d81a90cb7a` (ruling 17, `clock_ppm`).

**Tags.** Each task step that adds or changes behaviour is tagged CONFORM (honours something on disk, cited), EXTEND (goes beyond, authorised by a cited spec section) or AMEND (both).

## Global Constraints

- The format promises exact round trips (CLAUDE.md). `takes` and `placed` change `song` 1.0 in place (ruling 12). The schema is regenerated with `DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas` (`crates/duet-schemas/tests/schemas.rs:5`).
- Tests that run the binary go in `crates/duet-e2e` (CLAUDE.md). Each new e2e test is shown to fail against `main`'s binary: build `main` in a separate checkout with its own `CARGO_TARGET_DIR`, and run with `DUET_E2E_BIN`.
- **`placed.by`** is exactly one of `"click"`, `"estimated"` or `"set"` (ruling 8). **`bpm`** is present for `estimated` and `set` and absent for `click`. **`clock_ppm`** is allowed on `click` only (ruling 17).
- **The click is never written into the take.** Capture never writes the composition, and never changes its own estimate. A `click` or `set` placement is never replaced by an estimate.
- An estimated `bpm` is `60 × rate / timing.beat_period_frames`, never the rounded `timing.tempo_bpm`.
- `clock_ppm`: a moment `t` seconds after `bar` on the click's clock falls `at_secs + t × (1 + clock_ppm / 1_000_000)` seconds into the take. Positive means the take's clock ran fast.
- Tempos disagree when they are more than **0.5 bpm** apart. A tempo within 0.5 bpm of half or double the song's is said to be half or double.
- The count-in is **one bar** of the song's first meter. `record --song` places at `1.1.0`.
- CI gates: `cargo fmt --check`, clippy `-D warnings` (all features and no default features), rustdoc, and `cargo nextest run --profile ci`. Commits are conventional and end with the attribution line.

## Review Focus

1. **A song written before this task, with no `takes` key, still reads,** and is written back with `takes: []`. Owner: Task 1.
2. **A take linked to two songs:** a capture places and states for each song, and a refusal in one song leaves the other written. Owner: Task 4.
3. **`place` on an unplaced link with only some flags given** is refused, naming each flag still needed, and leaves the song's bytes unchanged. Owner: Task 3.
4. **A capture whose tempo was given** (`--tempo`, or the take's remembered `given`) is not stated as "estimated". The disagreement says "given", and an unplaced link is still placed from it. Owner: Task 4.
5. **`record --song` stopped by Ctrl-C during the count-in, before bar 1 sounds:** the take is kept as today, and the link is written unplaced, with a line saying bar 1 never played. Owner: Task 7.

---

### Task 1: `song` 1.0 gains `takes` and `placed`, with the composition's lookups

**Files:**
- Modify: `crates/duet-schemas/src/song.rs` (`Song` at `:40`, `Song::new` `:56`, `Song::check` `:72`, module docs `:14-15`)
- Modify: `crates/duet-schemas/schemas/song/1.0.json` (regenerated), `crates/duet-schemas/schemas/song/examples/1.0.duet`
- Test: `crates/duet-schemas/tests/song.rs`

**Interfaces:**
- Produces (in `duet_schemas::song`, re-exported as the existing items are):
  - `pub struct TakeLink { pub take: Name, pub digest: TakeDigest, pub r#for: Vec<Name> /* serde "for" */, pub placed: Option<Placed> }` with `TakeLink::new(take: Name, digest: TakeDigest) -> Self`.
  - `pub struct Placed { pub by: By, pub bar: Position, pub at_secs: f64, pub bpm: Option<f64>, pub clock_ppm: Option<f64> }` with constructors `Placed::click(bar, at_secs, clock_ppm: Option<f64>)`, `Placed::estimated(bar, at_secs, bpm)` and `Placed::set(bar, at_secs, bpm)`.
  - `pub enum By { Click, Estimated, Set }`, serialized lowercase, with `as_str()` and `Display`, like `Status` at `:146`.
  - `Song.takes: Vec<TakeLink>` (`#[serde(default)]`, always written).
  - `Song::link(&self, take: &Name) -> Option<&TakeLink>`, `Song::link_mut`, and `Song::linking(&self, digest: &TakeDigest) -> Option<&TakeLink>`.
  - `Song::check_for<'a>(&self, parts: impl IntoIterator<Item = &'a Name>) -> Vec<String>`: one line for each `for` that names no part, for `song show` to state. It is never an error.
  - `Composition::tempo_at(&self, at: Position) -> Result<f64, String>`, `Composition::meter_at(&self, at: Position) -> Result<TimeSig, String>`, and `Composition::key_at(&self, at: Position) -> Option<Key>`: the change in force at `at`.
  - `Composition::secs_between(&self, from: Position, to: Position) -> Result<f64, String>`: the seconds from `from` to `to` through the tempo map, tempo held between changes. Negative when `to < from`.
  - `TakeDigest` is `duet_dsp`'s (`crates/duet-dsp/src/take.rs:57`), the type `journal::TakeRef.digest` already uses (`crates/duet-schemas/src/journal.rs:1480`).

- [ ] **Step 1: Write the failing tests** in `crates/duet-schemas/tests/song.rs`, extending its fixture `song()` (`:53`) with two links: one `estimated` (bpm 88.6, bar `1.1.0`, at_secs 1.057, `for: ["melody"]`) and one `click` (bar `9.1.0`, at_secs 31.4, clock_ppm -41.0).
  - `a_song_round_trips_exactly_and_validates_against_its_schema` (existing, `:83`) now covers `takes`. CONFORM: it is the round-trip witness CLAUDE.md asks for.
  - `a_song_with_no_takes_key_reads_as_linking_none_and_writes_it_empty`: delete `body.takes` from the JSON; it reads with `takes.is_empty()`, and `write()` contains `"takes": []`.
  - `a_placement_that_breaks_its_kind_is_refused_by_name`, using `read_broken` (`:76`), with each message containing the field path:
    - `bpm` on `click` → `takes[1].placed.bpm`;
    - no `bpm` on `estimated` or `set`;
    - `clock_ppm` on `estimated` → `takes[0].placed.clock_ppm`;
    - `at_secs` -1 or NaN;
    - `bpm` 0;
    - `bar` `3.4.0` in 3/4 → "not how the song's bars write that place" (reuse the `real` check at `:327`).
  - `two_links_to_one_take_by_name_or_digest_are_refused_by_name`.
  - `a_for_that_names_a_part_twice_is_refused_and_one_naming_no_part_is_only_stated`: two "melody" entries are a read error; `check_for(&[bass])` returns one line naming `melody`.
  - `the_composition_says_what_holds_at_a_place`: tempo `[1.1.0 → 92, 9.1.0 → 120]` in 4/4.
    - `tempo_at(5.1.0) == 92`, `tempo_at(9.1.0) == 120`;
    - `secs_between(1.1.0, 9.1.0)` is 8 bars × 4 beats × 60/92 within 1e-9;
    - `secs_between(1.1.0, 10.1.0)` adds 4 × 0.5;
    - `secs_between(9.1.0, 1.1.0)` is its negative.

- [ ] **Step 2: Run the tests to see them fail.** Run `cargo nextest run -p duet-schemas --test song`. Expected: compile errors naming `TakeLink`, `Placed` and `tempo_at`.

- [ ] **Step 3: Implement** the Interfaces types and methods in `song.rs`. EXTEND (songs spec §1 "The song", §4; ruling 17).
  - `TakeLink` and `Placed` derive as `Placement` does (`:520-524`: `deny_unknown_fields`, `non_exhaustive`). `Option`s use `skip_serializing_if`.
  - The kind checks go in a new `Song::check_takes(&self, map: &TimeMap) -> Result<(), String>`, called from `Song::check`, using the same `real` position check `Composition::check` builds. Factor `real` out into a private fn taking `&TimeMap`, without changing its message.
  - Update the module docs at `:14-15` and the type docs, which say takes come with task 3.

- [ ] **Step 4: Regenerate the schema** with `DUET_BLESS_SCHEMAS=1 cargo test -p duet-schemas`, then hand-edit `schemas/song/examples/1.0.duet` to add the §1 example's `takes` entry. Run `cargo nextest run -p duet-schemas`. Expected: all pass, `every_schema_is_committed_and_current` included.

- [ ] **Step 5: Commit** with `feat(schemas): song 1.0 links takes and places them on its bars (click, estimated, set; clock_ppm)`.

---

### Task 2: `placing`: an estimate from an analysis, what capture may replace, and the disagreements

**Files:**
- Create: `crates/duet-actions/src/placing.rs` (unit tests inline, as `take.rs` keeps them)
- Modify: `crates/duet-actions/src/lib.rs` (add `pub mod placing;`)

**Interfaces:**
- Consumes: Task 1's `Placed`, `By`, `TakeLink`, `Song`, `Composition::tempo_at | meter_at | key_at`; `duet_capture::Analysis` (`crates/duet-capture/src/assemble.rs:90`) and its `timing: Timing` (`crates/duet-dsp/src/timing.rs:202`: `tempo_bpm: Origin<f64>`, `beat_period_frames`, `meter: Origin<TimeSig>`, `downbeat_frame: Origin<u64>`), and `key: Option<KeyEstimate>`.
- Produces:
  - `pub fn measured_bpm(timing: &Timing, rate: u32) -> f64` returns `60 × rate / beat_period_frames`.
  - `pub fn estimate(analysis: &Analysis) -> Placed` returns `Placed::estimated(Position::bar(1), downbeat_frame / rate, measured_bpm)`.
  - `pub enum Replace { Place(Placed), Keep(By) }` and `pub fn by_capture(current: Option<&Placed>, estimate: Placed) -> Replace`.
    - Unplaced or `estimated` gives `Place`.
    - `click` or `set` gives `Keep`.
  - `pub fn placed_said(song: &Name, take: &Name, before: Option<&Placed>, after: &Placed) -> String` states the new placement and, when there was one, what moved. For example: `placed the take wayfaring on wayfaring-stranger's bars, estimated: bar 1.1.0 at 1.057 s, 88.6 bpm (was 88.2 bpm, bar 1.1.0 at 1.061 s)`.
  - `pub fn disagreements(song: &Name, composition: &Composition, placed: &Placed, analysis: &Analysis) -> Vec<String>`, in §4's words. Each line names the origin (`estimated` or `given`) from the value's `Origin` (`crates/duet-dsp/src/timing.rs:158`: `Given(T)` or `Estimated { value, confidence }`). The lines:
    - **tempo:** `tempo estimated 88.6 bpm; wayfaring-stranger is 92 at bar 1`;
    - **half or double:** `… 46 bpm, half wayfaring-stranger's 92 at bar 1`;
    - **meter:** `meter estimated 4/4; wayfaring-stranger is 3/4 at bar 1`;
    - **key:** `key estimated A minor; the song is G minor at bar 1`;
    - **a `set` placement:** also `tempo estimated 88.6 bpm; the placement is set at 90 bpm`.
    - Nothing within the 0.5 bpm tolerance, and nothing for a value the analysis lacks (a rhythm has no key).
  - `pub fn latest_analysis(take_dir: &Path, digest: &TakeDigest) -> Result<Option<(PathBuf, Analysis)>>`: of the `*.capture.duet` files in `take_dir` whose `analysis.take.digest == digest`, the one modified most recently. An unreadable file is skipped, not an error.
  - `pub fn songs_linking(root: &Path, digest: &TakeDigest) -> Result<Vec<(Name, PathBuf)>>`: each `songs/<song>/song.duet` whose `Song::linking(digest)` is some, sorted by name. A song that doesn't read is an error naming it.

- [ ] **Step 1: Write failing unit tests in `placing.rs`.** Build an `Analysis` by hand. Check the "capture-analysis" fixtures under `crates/duet-capture` before inventing one, and reuse one if there is one.
  - `the_estimate_is_the_measured_tempo_not_the_rounded_one`: `beat_period_frames` for 92.4 at rate 11025, `tempo_bpm` 92 → `bpm` 92.4 within 1e-9, and `at_secs == downbeat_frame / rate`.
  - `capture_replaces_an_unplaced_or_estimated_placement_and_keeps_click_and_set`.
  - `a_tempo_more_than_half_a_bpm_from_the_songs_is_stated_and_one_within_it_is_not`: 92.6 vs 92 is stated; 92.4 vs 92 is not.
  - `a_tempo_near_half_or_double_is_said_to_be`.
  - `a_meter_and_a_key_that_differ_are_stated_and_a_rhythm_states_no_key`.
  - `a_given_tempo_is_stated_as_given`.
  - `a_set_placement_also_states_the_set_tempo_it_disagrees_with`.
  - `the_latest_analysis_of_the_take_is_read_and_another_takes_is_not`: a tempdir with two analyses of the digest and one of another.

- [ ] **Step 2: Run** `cargo nextest run -p duet-actions placing` to see them fail. Expected: unresolved module.

- [ ] **Step 3: Implement** the Interfaces in `placing.rs`. EXTEND (design "Capture of a linked take", "duet song link"; Global Constraints for the values). Format numbers with `capture::statement::number` (`crates/duet-actions/src/capture/statement.rs:286`, `pub(crate)`). CONFORM: capture's statements already format tempos this way.

- [ ] **Step 4: Run** `cargo nextest run -p duet-actions placing`. Expected: PASS.

- [ ] **Step 5: Commit** with `feat(actions): placing — an estimate from capture's analysis, what capture may replace, and its disagreements with a song`.

---

### Task 3: `duet song link` and `duet song place`; `song show` lists the takes

**Files:**
- Modify: `crates/duet-actions/src/song.rs`:
  - `SongCommand` at `:46`, with new `LinkInput` and `PlaceInput` beside `StatusInput` at `:115`;
  - `run` at `:158`;
  - `shown` at `:941`.
- Modify: `crates/duet-actions/src/registry.rs` (after `song.status`, `:192-198`)
- Modify: `README.md` (the songs commands section)
- Test: `crates/duet-e2e/tests/placing.rs` (new; copy the `project`, `clicks` and `wav` helpers' shape from `crates/duet-e2e/tests/takes.rs:28-53`, and `mod audio;` as `takes.rs` does)

**Interfaces:**
- Consumes: Task 1 (`TakeLink`, `Placed`, `Song::link_mut`) and Task 2 (`estimate`, `latest_analysis`, `placed_said`); `take::read_project` (`take.rs:710`); `read_song` (`song.rs:731`) and `write_song` (`song.rs:789`), which already journals `doc.written` through `ctx.wrote`.
- Produces:
  - `pub struct LinkInput { song: Name, take: Name, #[arg(long = "for")] r#for: Vec<Name>, #[arg(long, default_value = ".")] project: PathBuf }`.
  - `pub struct PlaceInput { song, take, #[arg(long)] click: bool, #[arg(long)] bar: Option<Position>, #[arg(long)] at: Option<f64>, #[arg(long)] bpm: Option<f64>, project }`. `--bpm` conflicts with `--click` (clap `conflicts_with`).
  - `pub(crate) fn link_take(ctx: &Context, song_dir: &Path, take: &ProjectTake, placed: Option<Placed>) -> Result<()>`, which Task 7 reuses: it links (or finds the link) and writes the placement, then `song.duet`.

- [ ] **Step 1: Write failing e2e tests** in `placing.rs`. Each new song uses `duet song new <song> --tempo 92 --meter 4/4`. Each take is `wav(&dir, "beat.wav", &clicks(92.0))` added with `duet take add`.
  - `link_before_any_capture_links_unplaced_and_says_the_next_capture_places_it`: `takes[0]` is `{take: beat, for: []}` with `placed` absent, and stderr names the next capture.
  - `link_after_a_capture_places_it_estimated_from_that_capture`. Run `duet capture rhythm beat` (default stem, into `takes/beat/`), then `link`.
    - `by: estimated`, and `bpm` equals `60 × rate / timing.beat_period_frames` read back from `takes/beat/beat.rhythm.capture.duet`.
    - stderr names that analysis file.
  - `link_again_adds_its_for_parts_and_without_for_changes_nothing`: the bytes are unchanged on the second plain `link`.
  - `place_with_bpm_sets_it_keeping_every_value_not_given` (half of witness 2). Link after a capture, then `place --bpm 90`.
    - `by: set`, `bpm: 90`;
    - `bar` and `at_secs` are the estimate's.
  - `place_click_with_bar_and_at_places_it_by_click_and_refuses_bpm_with_click`.
  - `place_on_an_unplaced_link_without_every_value_is_refused_naming_each_and_leaves_the_song` (Review Focus 3): `place --bpm 90` alone names `--bar` and `--at`, and the bytes are unchanged.
  - `place_on_a_take_not_linked_links_it_then_places_it`.
  - `link_refuses_a_take_not_in_the_project_naming_take_add`.
  - `song_show_lists_each_take_with_its_placement_and_states_a_for_naming_no_part`.

- [ ] **Step 2: Run** `cargo nextest run -p duet-e2e --test placing`, and against `main`'s binary with `DUET_E2E_BIN`. Expected: all fail ("unrecognized subcommand 'link'").

- [ ] **Step 3: Implement `link`, `place`, `link_take` and the `shown` additions.** EXTEND (design "duet song link", "duet song place"; songs spec §4 "Takes without a click").
  - **Registry:** both entries are `writing(…, DOC_WRITTEN, writes("the song's song.duet, over itself", true), schema::<…>)`. CONFORM `registry.rs:192-198`.
  - **The take is found** in the project document by name. A name that isn't there is refused with `take::add_line`'s command (`take.rs:510`). CONFORM: capture's refusal at `capture.rs:444-448`.
  - **Every refusal** happens before `write_song`, so a refused command leaves the bytes as they were.

- [ ] **Step 4: Run** `cargo nextest run -p duet-e2e --test placing` and `cargo nextest run -p duet-actions`. Expected: PASS. The registry's own tests (`crates/duet-e2e/tests/actions.rs`) also pass.

- [ ] **Step 5: Commit** with `feat: duet song link and duet song place, and song show lists a song's takes`.

---

### Task 4: Capture of a linked take places it and states its disagreements

**Files:**
- Modify: `crates/duet-actions/src/capture.rs` (`run`, after `remembered.remember(…)` at `:366`, before scoring; the module docs)
- Modify: `crates/duet-actions/src/registry.rs` (the capture entries' `writes`: they now also write a linking song's `song.duet`)
- Test: `crates/duet-e2e/tests/placing.rs`

**Interfaces:**
- Consumes: Task 2 (`songs_linking`, `by_capture`, `disagreements`, `placed_said`, `estimate`) and Task 3 (`link_take`, or the song read and write it wraps).
- Produces: `fn place_in_songs(ctx: &Context, analysis: &Analysis) -> Result<Vec<String>>`, private to `capture.rs`. It returns the lines to state. It runs only when `ctx.project()` is some and `analysis.take` is some.

- [ ] **Step 1: Write failing e2e tests** in `placing.rs`:
  - `a_capture_places_an_unplaced_link_estimated_and_says_so`.
  - `a_capture_of_a_placed_take_states_a_tempo_that_disagrees_and_changes_neither` (witness 3). Use song tempo 100 and take clicks at 92.
    - The song's composition bytes are unchanged; compare `body.composition` as a value before and after.
    - stderr contains `tempo estimated 92` and `is 100 at bar 1`.
    - The analysis's `timing.tempo_bpm` is the estimate (92), not 100.
  - `a_later_capture_leaves_a_set_placement_and_states_what_disagrees_with_it` (the rest of witness 2). After `place --bpm 90`, capture again: `placed` is byte-equal to before, and stderr states the set tempo.
  - `a_take_linked_to_two_songs_is_placed_and_stated_for_each` (Review Focus 2).
  - `a_given_tempo_places_the_link_and_is_stated_as_given` (Review Focus 4): `--tempo 92` on a song at 100.

- [ ] **Step 2: Run** them, and against `main`'s binary. Expected: FAIL (no link, or `takes` unknown on `main`).

- [ ] **Step 3: Implement `place_in_songs`** and call it after the analysis is written. EXTEND (songs spec §4 "Estimate, then compare"; design "Capture of a linked take").
  - **Write and state:** each song whose placement `by_capture` replaces is written through the same atomic `write_song` path, and the `placed_said` line is stated. Every song gets its `disagreements` lines.
  - **Where the lines go:** to stderr, after the timing lines, and into the part's `statements`. That means collecting them before `part_said` is built at `capture.rs:326`. Place them there, ahead of the part write, so the part states them. CONFORM: capture's other statements reach the part the same way (`capture.rs:324-327`).
  - **A failure in one song** is reported naming it, and the others are still placed.

- [ ] **Step 4: Run** `cargo nextest run -p duet-e2e --test placing --test takes --test capture --test actions`. Expected: PASS. The existing capture goldens do not change, because no golden project links a take. If one changes, stop and report it; do not bless.

- [ ] **Step 5: Commit** with `feat(capture): a capture of a linked take places it on each song and states where it disagrees`.

---

### Task 5: `take forget` and `song snapshot` read the song's links

**Files:**
- Modify: `crates/duet-actions/src/take.rs` (`linked_by` at `:809`, and `forget` at `:210`, which reports it)
- Modify: `crates/duet-actions/src/song.rs` (`take_snapshot`, where `Snapshot::new` is built at `:314`)
- Test: `crates/duet-e2e/tests/takes.rs`, `crates/duet-e2e/tests/snapshots.rs`

**Interfaces:**
- Consumes: Task 2 `songs_linking`; `snapshot.takes: Vec<TakeRef>` (`crates/duet-schemas/src/snapshot.rs:51`) and `TakeRef::new` (`journal.rs:1485`).

- [ ] **Step 1: Write failing tests.**
  - In `takes.rs`, extend `take_forget_leaves_the_files_and_says_which_song_still_links_the_take` (`:311`) with a song that links the take through `takes` but has no comped part. Stderr names `songs/<song>/song.duet`.
  - In `snapshots.rs`, add `a_snapshot_names_the_takes_the_song_links_by_name_and_digest`: `snapshot.duet`'s `takes` equals the song's links as `(take, digest)`, sorted.

- [ ] **Step 2: Run** both tests. Expected: FAIL.

- [ ] **Step 3: Implement.** AMEND #94's `linked_by`: keep the comped parts, and add the songs found by `songs_linking`, one line each, sorted. Fill `body.takes` before `body.check()`. CONFORM: snapshot spec §3 ("the takes the song linked with their digests"). Update the comment at `take.rs:809` and #94's note that "the song document doesn't name its takes yet".

- [ ] **Step 4: Run** `cargo nextest run -p duet-e2e --test takes --test snapshots`. Expected: PASS.

- [ ] **Step 5: Commit** with `feat: take forget names the songs that link a take, and a snapshot names the song's takes`.

---

### Task 6: The click and the clock arithmetic, with no device

**Files:**
- Create: `crates/duet-actions/src/record/click.rs` (unit tests inline). `record.rs` stays where it is and declares `mod click;`; Rust resolves it to `record/click.rs`.

**Interfaces:**
- Consumes: Task 1 `Composition::secs_between` and `meter_at`.
- Produces (all `pub(crate)`):
  - `struct Ticks { frames: Vec<(u64, bool)> /* output frame, is a downbeat */, bar_one: u64 }` and `fn Ticks::of(composition: &Composition, rate: u32, lead_frames: u64, until_secs: f64) -> Result<Ticks>`.
    - Beat by beat: one bar of count-in in the first meter, at the first tempo, then from `1.1.0` through the composition's maps until `until_secs` after bar 1.
    - `lead_frames` of silence come before the count-in.
    - A beat is the meter's denominator: a quarter in 4/4, an eighth in 6/8.
    - `bar_one` is the frame of bar 1's tick.
  - `struct ClickVoice` with `fn ClickVoice::new(ticks: Ticks, rate: u32, channels: u16) -> Self` and `fn fill(&mut self, out: &mut [f32])`, which writes interleaved samples and advances its own frame counter.
    - A tick is a 20 ms decaying sine: 1 760 Hz on a downbeat and 880 Hz otherwise, peak 0.5.
    - Everything else is silence.
  - `struct Stamp { frame: u64, nanos: u128 }`: a stream's frame index and the host instant the system stamped on it.
  - `struct Span { first: Stamp, last: Stamp }` and `fn Span::rate(&self) -> Option<f64>`: frames per host second, or `None` when the span is under 1 s.
  - `fn offset_frames(input: &Span, output: &Span, bar_one: u64, nominal_in: u32, nominal_out: u32) -> u64` and `fn clock_ppm(input: &Span, output: &Span, nominal_in: u32, nominal_out: u32) -> Option<f64>`. The algorithm the tests don't fix, written out:

    ```text
    r_in  = input.rate()  or nominal_in        r_out = output.rate() or nominal_out
    bar_one_nanos = output.first.nanos + (bar_one − output.first.frame) × 1e9 / r_out
    offset_frames = round((bar_one_nanos − input.first.nanos) × r_in / 1e9) − input.first.frame
    clock_ppm     = ((r_in / nominal_in) / (r_out / nominal_out) − 1) × 1e6   (None if either rate is None)
    ```

- [ ] **Step 1: Write failing unit tests:**
  - `the_count_in_is_one_bar_of_the_first_meter_and_bar_one_follows_it`: 92 bpm in 3/4 at 48 kHz with `lead_frames` 4800.
    - Ticks at 4800 + k × 60/92 × 48000, rounded, for k = 0..3.
    - `bar_one` is the fourth.
    - Downbeats are k = 0 and k = 3.
  - `ticks_follow_a_tempo_change_on_the_composition`.
  - `the_voice_writes_a_tick_only_at_its_frames_and_silence_between`.
  - `the_offset_is_bar_ones_playback_less_the_takes_first_capture_in_whole_frames`: with fake stamps, input at 0 ns at frame 0 and output starting 5 ms later, `bar_one` = 4800 frames at 48 kHz gives 0.105 s × 48000 = 5040.
  - `matched_clocks_drift_zero_ppm_and_a_fast_input_drifts_positive`: input 48 002.4 frames per host second against nominal 48 000 gives +50 ppm within 0.01.
  - `a_span_under_a_second_measures_no_drift`.

- [ ] **Step 2: Run** `cargo nextest run -p duet-actions click`. Expected: FAIL.

- [ ] **Step 3: Implement** the Interfaces in `record/click.rs`. EXTEND (design "duet record --song"; ruling 17).

- [ ] **Step 4: Run** `cargo nextest run -p duet-actions click`. Expected: PASS.

- [ ] **Step 5: Commit** with `feat(record): the click's ticks and voice, and the offset and drift from the streams' stamps`.

---

### Task 7: `duet record <take> --song <song>`

**Files:**
- Modify: `crates/duet-actions/src/record.rs`:
  - `RecordArgs` at `:34`: add `#[arg(long)] song: Option<Name>` and `#[arg(long, requires = "song")] no_reconcile: bool`;
  - `run` at `:81`;
  - `Shared` at `:69`: add the input span;
  - `input` at `:437`: record the first and last capture `Stamp`;
  - the module docs at `:1-11`.
- Modify: `crates/duet-actions/src/registry.rs` (record's `writes` gains the song's `song.duet`)
- Modify: `crates/duet-cli/src/main.rs:98-100` (record's help line), `README.md`
- Test: `crates/duet-e2e/tests/record.rs` (existing) and `crates/duet-e2e/tests/placing.rs`

**Interfaces:**
- Consumes: Task 6 (`Ticks`, `ClickVoice`, `Span`, `Stamp`, `offset_frames`, `clock_ppm`), Task 3 `link_take`, and `take::add_recorded` (`take.rs:853`).

- [ ] **Step 1: Write failing tests:**
  - In `placing.rs`, `record_song_is_refused_before_any_device_outside_a_project_for_an_unknown_song_and_for_a_file_output`:
    - outside a project;
    - `--song nope`, which names the project's songs;
    - `duet record x.flac --song s`, which says `--song` takes a take's name.

    Each refusal's message is checked. None opens a device: they pass on CI.
  - In `placing.rs`, `record_song_places_the_take_by_click_and_its_ticks_land_where_it_says` (witness 1, the speakers-on check, owner 2026-11-21).
    - **Opt-in:** it returns early with `eprintln!("skipped: set DUET_E2E_SPEAKERS=1 …")` unless `DUET_E2E_SPEAKERS` is set. Set, it fails loudly if no device opens.
    - **The recording:** a song at 92 in 4/4, then `duet record t --song s --seconds 8` with the click played aloud.
    - **The placement:** it asserts `by: click` and `bar: 1.1.0`, that `at_secs × rate` is a whole number, and that `clock_ppm` is present.
    - **The ticks in the take:** it decodes the FLAC with `duet_capture`'s file source and finds the first sample after `at_secs − 50 ms` whose magnitude exceeds 8× the median absolute level of the first 100 ms. That onset is within `TOLERANCE_MS = 2.0` of `at_secs`.
    - **A later bar:** the same holds for bar 3's tick, at `at_secs + secs_between(1.1.0, 3.1.0) × (1 + clock_ppm/1e6)`.
    - **The report:** it prints both errors in ms, so the PR can state them.
  - In `placing.rs`, `record_song_with_no_reconcile_writes_no_clock_ppm`, gated the same way.

- [ ] **Step 2: Run** them, and against `main`'s binary. Expected: the refusal test fails ("unexpected argument '--song'"). Run the gated tests once with `DUET_E2E_SPEAKERS=1` on the Mac mini; they fail the same way.

- [ ] **Step 3: Implement in `run`.** EXTEND (design "duet record --song"; ruling 17).
  - **Refusals:** each one comes before `cpal::default_host()`. `--song` requires `into_project` to be some.
  - **The output device:**
    - Open the host's default output and build an f32 output stream filled by `ClickVoice`, with `lead_frames` = 0.2 s.
    - Record each buffer's playback `Stamp` (`OutputCallbackInfo::timestamp().playback`) into an output `Span`, as `play.rs:388-420` builds its stream (CONFORM).
    - Start the input stream first, then the output.
  - **What it says at the start:**
    - the headphones line;
    - when `name_of(input) != name_of(output)`: `the input (…) and output (…) are different devices: their clocks may drift, and duet measures it`;
    - on a host other than macOS (`cfg!(not(target_os = "macos"))`): the offset is as the system reports it, unchecked there.
  - **The placement, after `add_recorded`:**
    - Compute `offset_frames` and `clock_ppm`, and state the drift.
    - Write `Placed::click(Position::bar(1), offset / rate, if no_reconcile { None } else { ppm })` through `link_take`.
  - **Stopped before bar 1** (Review Focus 5): when the take ended before `bar_one` played, link it unplaced and say bar 1 never played.

- [ ] **Step 4: Run** `cargo nextest run -p duet-e2e --test placing --test record` and `cargo nextest run -p duet-actions`. Then, on the Mac mini, run `DUET_E2E_SPEAKERS=1 cargo nextest run -p duet-e2e --test placing record_song --no-capture` in a quiet room with the speakers on. Expected: PASS, with both tick errors printed under 2 ms. Record those figures for the PR.

- [ ] **Step 5: Commit** with `feat(record): duet record --song plays the song's click and places the take by it, to the frame, with the clocks' drift`.

---

### Task 8: The specs, README and help agree with what was built

**Files:**
- Modify:
  - `docs/superpowers/specs/2026-11-17-songs-and-composition-design.md`: build task 3 at `:500-503`, marked **Built** with its temper ref as tasks 4 and 5 are; task 4's "stays empty until task 3" at `:511`.
  - `docs/superpowers/specs/2026-11-21-placing-takes-design.md`: any call the build changed.
  - `README.md` (the songs, record and capture sections).
  - The `SongCommand` and `RecordArgs` doc comments, which are the CLI's help.

- [ ] **Step 1:** Update each. CONFORM: how build tasks 4 and 5 were marked built (`:504-521`).
- [ ] **Step 2:** Run CI's steps locally:
  - `cargo fmt --check`;
  - `cargo clippy --locked --all-targets --all-features -- -D warnings` (`ci.yml:146`), and the same with `--no-default-features`;
  - `cargo doc --locked --no-deps` (`ci.yml:148`);
  - `cargo nextest run --profile ci`;
  - `python3 -I .conventions/baseline/tools/conventions.py check` and `… witness` (`.github/workflows/ci.yml:73-75`).

  Expected: all pass.
- [ ] **Step 3: Commit** with `docs: songs task 3 built — placing takes, in the songs design, README and help`.
