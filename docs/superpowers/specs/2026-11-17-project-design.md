# duet projects: `duet init`, a project document, and a layout sorted by what each file is

**Date:** 2026-11-17. **Task:** temper `01a1e007-8747-77ee-8333-dd462d6d01eb`.

**Status: Proposed. Nothing here is built, and nothing is built before the owner rules.** Each
point is marked **Ruled** (with where it was ruled) or **Proposed**. The questions for the owner
are numbered decisions at the end ([Decisions for the owner](#decisions-for-the-owner)).

**Builds on:**
- [duet documents](2026-11-15-duet-documents-design.md): the `.duet` envelope, kinds, the version
  policy, and the constraint that the project kind lives inside `.duet/` (its "Project, deferred").
- [Variants and comps](2026-11-17-variants-and-comps-design.md): the take digest over decoded
  samples, readings named as variants by annotation, comps that never name a path.
- The config decision (temper `01a1daa7-5cea-7301-80a6-7d3f1905f3a7`) and its build: `duet.toml`
  and `duet.local.toml` in the user's config directory and a project's `.duet/`, found by walking
  up from the working directory (`crates/duet-cli/src/config.rs`).
- The jobs-and-boundaries note (temper `01a1e57c-f9c8-72e5-8a9b-6a4e5874bfdc`): section 4, the
  owner's rulings on what duet is, and section 8, the layout by what each thing is.
- The composition task (temper `01a1dc62-9d9f-73ae-b4ec-03b07a08e407`): layered song, project, or
  both. This spec names the choice it forces and does not design composition.
- The workbench task (temper `01a1e563-22e6-70e6-a7ce-8b38e93b3091`), and the owner's ruling on it:
  a server loop is duet's event bus, built bus-and-CLI first, and the CLI always stands alone
  (temper `01a1e9ff-33f4-7123-85d9-7323abead248`, 2026-11-17). The bus is the workbench's to design;
  this spec states only what the layout must hold for it
  ([§10](#10-what-the-layout-holds-for-the-bus)).

**Grounding:** the code at `main` (`bb58d0f`, `capture-analysis` 1.12). The take digest of the
variants design is not built yet; this design depends on it (build task 1 there).

## The problem

Every take, reading, review sheet and verdict today sits in scratch space or in the duet checkout's
`fixtures/`, named on the spot. That fails three jobs from the jobs note:
- **J8:** coming back next week and finding a song's takes, readings, rulings and drafts.
- **C6:** the next session, local or cloud, finding what this one found.
- **The licence boundary:** the owner's takes stay out of git only because `fixtures/` is ignored
  (now in `.gitignore`, before that `.git/info/exclude`), which someone has to remember.

## What is already settled

- **There is no special project extension.** A project is a document of kind `project`, written
  as any `.duet` document is. **Ruled** (owner, 2026-11-15; the `duet init` task's "Settled").
- **`<project>/.duet/` is the project's config directory**, holding `duet.toml` and
  `duet.local.toml`, found by walking up from the working directory, nearest first. **Ruled** (the
  config decision) **and built** (`config.rs`, witnessed in `crates/duet-e2e/tests/cli.rs`).
- **A given flag always wins** over any file's default, as `--tempo` does today. **Ruled** (the
  config decision's precedence; the `duet init` task, question 3).
- **The project document cannot be a file named `.duet` at the root,** because that name is the
  directory. The documents design suggests `.duet/project.duet`. That is stated there as a
  constraint, not among its seven rulings.
- **The capture is never altered.** Every change is stated beside it. **Ruled** (owner,
  2026-11-16, the jobs note section 4).
- **A take is identified by a digest of its decoded samples,** with its rate, channels and frames.
  Readings are variants of a take by that digest, and a comp names variants by annotation, never by
  path. **Ruled** (owner, 2026-11-17, the variants design, rulings 1 and 4).
- **No Temper dependency.** A project is files in a directory. **Ruled** (the charter's
  `no-temper-dependency` constraint).
- **Model weights live in a per-user cache, never in a project.** **Ruled** (owner, 2026-11-16,
  "for now"; the jobs note section 9). Basic Pitch's model is embedded and needs no cache.

## Prior art, and what duet takes from it

Secondary sources, read 2026-11-17, listed at the end. What was not verified is said.

| Tool | What it does | Taken | Left |
|---|---|---|---|
| **Logic Pro** | A project is a package or a folder. "Copy audio files into project" decides whether imported audio is copied in or referenced where it is. Apple notes a folder lets several project versions share one set of audio. | The choice between copying a source in and naming it where it is, made per source and stated. A plain folder, never a package: a package hides files from the CLI and from Claude, and sync tools have broken packages into folders. | The package. |
| **Ardour** | A session is a folder with a fixed layout: `interchange/` for recorded and imported audio, `peaks/` and `analysis/` that are safe to delete and are rebuilt, `export/`, and the session file with its `.bak` and `.history`. Its manual says to move the whole folder, never the session file alone. | Fixed, named directories by what a file is. Rebuildable data marked as rebuildable. The record of actions (Ardour's `.history`) kept beside the state, not in place of it. | Undo history as a separate file per save: duet's record is one append-only stream. |
| **REAPER** | Media is referenced by path unless "copy all media into project directory" is ticked. A relative media path keeps a project portable; an absolute one breaks it. `.reapeaks` are rebuilt if missing. A shared media folder cleaned by one project can delete another's media. | Paths inside a project are always relative. A source outside the project is found by a machine-local map, never by a path written into a shared file. One project's cleanup never touches anything outside it. | |
| **DVC** | A small `.dvc` file, committed to git, names a large file by its hash, size and relative path; the content lives in a cache and is kept out of git. | **The core idea:** the shared half names a source by its content digest, and the content lives somewhere git does not see. duet's digest is already ruled. | A content-addressed cache directory of duet's own. A take is few and large, and its name is the owner's, not a hash. |
| **git-annex** | Locked files are symlinks to content named by a key (`SHA256E` by default: a SHA-256 of the content and the extension). Unlocked files are pointer files behind git's clean and smudge filters, because symlinks get in the way of tools that edit files. | Identity by digest, and the lesson of its unlocked mode: **no symlinks.** They fail on Windows, in sync tools and in cloud sessions, and an audio app that follows one may write through it. | Any git integration. duet never runs git hooks or filters. |
| **cargo** | `cargo new PATH` makes a directory; `cargo init` adopts the current one and refuses where `Cargo.toml` exists. It runs `git init` unless already inside a repository, and `--vcs none` turns that off. | **`init` works in an existing, non-empty directory and never overwrites.** One command for a new and an existing directory, as `uv init [PATH]` does. A refusal names what already exists. | Running git by default ([decision 7](#decisions-for-the-owner)). |
| **uv** | `uv init` writes `pyproject.toml`, a `.gitignore`, a `.python-version` pin and a README; `--bare` writes only the manifest. | A minimal default: the project document and the ignore rules, and nothing that has to be deleted. The tool version is pinned in the document, as `.python-version` pins Python. | A README and a source tree written by default. |
| **`temper init`** | A config wizard for the machine, and a separate project-setup flow in which an agent writes the project's prose in conversation. Temper ruled against scaffolding prose inside the binary, because an agent would still have to fill it. | **The split:** `duet init` writes structure and metadata only. Anything that needs words (the title is the one exception, and it defaults to the directory's name) is written by the owner or by Claude afterwards, through ordinary commands. | Creating a Temper context. That stays out of duet ([§9](#9-the-boundary-with-temper)). |

**Not verified first-hand:** Logic's internal package layout (Apple does not document it; only the
copy setting and the package-or-folder choice are from Apple), DVC's exact `.gitignore` handling
(its docs site was out of reach from this session), and git-annex's repository-version details.
None of the proposals rests on them.

## 1. The layout

**Proposed.** The jobs note's section 8 sorts a project's contents by what each thing is. This
keeps that sorting and changes three things in its illustrative tree:
1. **Sources and their derived files share one directory per take,** under `takes/`, so one
   ignore rule covers both. The jobs note's tree had `takes/` and `derived/` apart, which needs
   two rules and lets a clip end up in the wrong one. A clip of a take *is* the take.
2. **The project document sits in `.duet/`,** as the documents design proposed, so one marker
   (`.duet/`) finds a project and the root holds only the owner's work
   ([decision 2](#decisions-for-the-owner)).
3. **Comps get a directory of their own,** since they did not exist when the note was written.

```text
wayfaring/                          the project: any directory, outside the duet checkout
  .duet/
    project.duet                    the project document (kind: project)        authored
    duet.toml                       project config, as today                    authored
    duet.local.toml                 this machine's config, as today             ignored
    takes.local.toml                where named takes live on this machine      ignored
    run/                            the server's socket and lock, when running  ignored
    state/                          the state store, its kind still to be ruled ignored
  .gitignore                        written by init, its duet block kept by duet
  log.duet.jsonl                    the domain-event log                        the record
  takes/                            THE LICENCE BOUNDARY: ignored by one rule
    wayfaring/                      one take: its source and everything from it
      wayfaring.flac                the source, when copied in (absent when named)
      wayfaring.license             its licence notice, when it has one
      wayfaring.performance.duet.jsonl
      wayfaring.hmm.capture.duet    a reading (a variant)
      wayfaring.hmm.duet            its song
      wayfaring.hmm.abc             its score
      wayfaring.rules.capture.duet  another variant
      wayfaring.comped.capture.duet the comp applied, and its .duet and .abc
      compare/                      compare's SVGs and WAVs
      review/                       a review sheet and its clips
  review/
    wayfaring.moments               moments: times and labels, no audio         authored
  comps/
    wayfaring.comp.duet             picks and marks over the variants           authored
  parts/                            songs and grids the owner keeps             authored
  renders/                          MIDI and audio rendered from parts          ignored, rebuildable
```

### What each thing is

| What | Where | Kept how | Shared? |
|---|---|---|---|
| **Sources** | `takes/<name>/<file>`, or named where they are ([§4](#4-sources-named-without-copying)) | Never modified. Identified by digest | Restricted by default |
| **Derived from a source** | `takes/<name>/` | Rebuilt from the source and the tool's version. A reading names its reader and passes, so variants sit side by side | Inherits its take's restriction |
| **Authored** | `.duet/project.duet`, `.duet/duet.toml`, `review/`, `comps/`, `parts/` | Small and precious. Versioned like code | Shareable even when the audio is not |
| **The record** | `log.duet.jsonl` | Append-only | As authored |
| **Rebuildable, not restricted** | `renders/` | Rebuilt from parts | Ignored by default, as build output is |
| **Machine-local** | `.duet/*.local.toml`, `.duet/run/`, `.duet/state/` | This machine only | Never |
| **Outside the project** | the user's config directory, the weights cache, soundfonts | Per user | Not applicable |

### Rules the layout keeps

- **Every path a shared file writes is relative to the project root.** An absolute path appears
  only in `.duet/takes.local.toml`. (REAPER's lesson.)
- **A take's name is the owner's,** a short lowercase name with hyphens (`wayfaring`,
  `blood-sport-v2`), unique in the project, and the directory and file stems under `takes/` use
  it. Nothing reads a file's name to learn what it is: the envelope says that, as today.
- **A reading's stem is `<take>.<label>`,** where the label is its reader (`hmm`, `rules`,
  `basic-pitch`), or `comped`. This is the variants design's own example
  (`wayfaring.hmm.capture.duet`), made the project's default. A second reading with the same label
  is refused, as every capture output is today; `-o` names another.
- **The performance is written once per take,** at `takes/<name>/<name>.performance.duet.jsonl`, if,
  as appears from the code, it does not depend on the reader. A capture that finds one already
  there checks it is of the same take (`performance` 1.1's `take`, ruled in the variants design) and
  the same duet version, and reuses it; otherwise it refuses, naming both. *To verify when built.*
- **Directories are made when first written,** not by `init`. Git keeps no empty directory, and a
  project with no comps has no `comps/`. The names are fixed by this spec, not by what exists.
- **Projects live outside the duet checkout.** The duet repository is not a project, and its
  `fixtures/` stays what it is until the owner's work there is adopted
  ([§7](#7-adopting-scratch-work)).

### Where review sets, verdicts and comps go

The `duet init` task asks this for the workbench (its "Added 2026-11-16").
- **A moments file** is authored (times and labels, no audio): `review/<take>.moments`.
- **A review sheet** is HTML with clips cut from the take, so it is the take:
  `takes/<take>/review/`. Today `duet capture review` writes `STEM.review/`; in a project its
  default `--dir` becomes this.
- **A verdict that picks or marks** is a comp entry (variants design §8, ruled):
  `comps/<take>.comp.duet`.
- **A verdict that picks nothing** ("no audible difference") lives in the record, as the variants
  design says. Whether the review task also keeps answers beside the moments file
  (`review/<take>.answers`, say) is that task's call; `review/` is where it would go.

## 2. The project document: `project` 1.0

**Proposed. A new kind, `project` 1.0,** a `.duet` document at `.duet/project.duet`, with its schema
and example committed under `crates/duet-schemas/schemas/project/`.

```json
{
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/project/1.0.json",
  "duet": { "kind": "project", "version": "1.0", "tool": { "name": "duet", "version": "0.5.0" } },
  "body": {
    "title": "Wayfaring Stranger",
    "created": "2026-11-17",
    "duet": "0.5",
    "defaults": { "tempo_bpm": 92.0, "meter": "3/4", "key": "Ab", "tuning": "+0c", "grid": "1/16" },
    "takes": [
      {
        "name": "wayfaring",
        "take": { "digest": "sha256:9f2c…", "rate": 44100, "channels": 1, "frames": 976080 },
        "file": "wayfaring-stranger.flac",
        "kept": "copied",
        "restriction": "open",
        "licence": "CC-BY-4.0",
        "added": "2026-11-17"
      },
      {
        "name": "blood-sport",
        "take": { "digest": "sha256:41ab…", "rate": 48000, "channels": 1, "frames": 1203200 },
        "file": "Blood Sport 3.m4a",
        "kept": "named",
        "restriction": "restricted",
        "added": "2026-11-17"
      }
    ]
  }
}
```

### Fields

- **`title`**, required: the project's name for people. `init` sets it from the directory's name
  unless `--title` is given.
- **`created`**, required: the date `init` ran.
- **`duet`**, required: the `<major>.<minor>` of duet that made the project, as information. It is
  the tool version the task asks for. The envelope's `tool` says who last wrote the file; this says
  who made the project. Neither is ever checked.
- **`defaults`**, optional, every field optional: tempo, meter, key, tuning and grid, spelled as
  their flags spell them. What they apply to is [§5](#5-how-commands-find-and-use-a-project).
  - **The lane map is not here.** Lanes are config, in `.duet/duet.toml`, as ruled in the
    documents design ("Lane maps and config are not kinds"). One place for each thing.
- **`takes`**, required, possibly empty: the project's sources
  ([§4](#4-sources-named-without-copying)).
  - `name`: as in the layout's rules. Unique.
  - `take`: the variants design's `take` object, the same type. The digest is the identity.
  - `file`: the source's own file name when added, without its directory. Information, for people
    and for finding it again; never a path.
  - `kept`: `copied` (the source is at `takes/<name>/<file>`) or `named` (it is found through
    `.duet/takes.local.toml`).
  - `restriction`: `restricted` (the default) or `open` ([§6](#6-the-licence-boundary)).
  - `licence`, optional: an SPDX identifier when the take has a known licence, as the corpus
    manifest's `licence` does. The notice itself stays in `takes/<name>/<name>.license`, as the
    corpus and `duet capture review` already keep it.
  - `added`: the date it was added.
- **No `parts`, `pieces` or `tracks` in 1.0.** See [§8](#8-the-choice-composition-forces).

### The version

- **1.0, new.** The version policy is ruled; nothing about it changes here.
- **Expected minors:** a take's optional provenance (`from: "duet record" | "logic tap" | …`), an
  optional list of pieces once composition is designed
  ([§8](#8-the-choice-composition-forces)), and new `defaults` fields. Each is an optional field
  whose absence means what 1.0 meant.
- **Why `defaults` is a closed object** and not a free table: unknown fields are refused at every
  version (ruled), so a typo in a default is caught, not silently ignored.

### The Rust type

`Project` beside the other kinds' types, with `Kind` implemented in `duet-schemas`, as for every
kind (ruled in the documents design: no crate below `duet-schemas` reads or writes an enveloped
document). `Take` is the variants design's type, shared, not copied.

## 3. What `duet init` writes

**Proposed.**

```sh
duet init                      # the working directory becomes a project
duet init ~/Music/wayfaring    # makes the directory if it is missing
duet init --title "Wayfaring Stranger"
```

It writes, and prints each path it wrote:
1. **`.duet/project.duet`**: `project` 1.0 with `title`, `created`, `duet`, empty `takes`, and no
   `defaults`.
2. **`.duet/duet.toml`**, only if absent: every line a comment, naming what may be set here
   (`[song] end`, `[play] soundfont`, `[lanes.*]`), so it changes nothing until edited. An existing
   one is left alone and named.
3. **`.gitignore`**: the duet block ([§6](#6-the-licence-boundary)). Appended between marker lines
   to an existing file, never rewritten around the owner's own lines.

It does not:
- **overwrite.** A directory that already holds `.duet/project.duet` is refused, naming it. A
  `.duet/` with only config in it is adopted: it gains a project document and keeps its config.
- **move, copy or delete anything.** Adopting files is a separate, explicit step
  ([§7](#7-adopting-scratch-work)).
- **run git** ([decision 7](#decisions-for-the-owner)). It prints one line: `git init` to version
  the authored half.
- **make directories** beyond `.duet/`.
- **touch the network or Temper.**
- **write prose,** beyond a title it can derive. (`temper init`'s split.)

**Nested projects.** `init` inside an existing project is refused, naming the project above,
unless `--nested` is given. A nested project is legal (the nearest `.duet/` wins, as today) but is
nearly always a mistake.

**A `.duet` in a home directory.** Today a `~/.duet/` would make every directory below it a
project for config. `init` refuses to make one in the home directory itself.

## 4. Sources named without copying

**Proposed.** A take is added to the project explicitly, copied in or named where it is.

```sh
duet take add ~/Downloads/idea.m4a --name idea              # copied into takes/idea/
duet take add "/Volumes/Phone/Blood Sport 3.m4a" --named    # left where it is, named by digest
duet take locate blood-sport "/Volumes/Archive/Blood Sport 3.m4a"   # found again elsewhere
duet take list                                              # every take, where it is found, or not
```

- **Adding decodes the whole file once** and computes the take digest (the variants design's
  digester), its rate, channels and frames.
- **Copied:** the file is copied byte for byte to `takes/<name>/<file>`. Never transcoded: the
  capture is never altered (ruled), and a byte copy needs no argument about whether a re-encode is
  the same take.
- **Named:** nothing is copied. The absolute path goes into `.duet/takes.local.toml`, which is
  machine-local and ignored:

  ```toml
  [blood-sport]
  path = "/Volumes/Phone/Blood Sport 3.m4a"
  ```

  The shared document holds only the digest and the file's name. (DVC's pointer file, with
  REAPER's lesson that a path is the machine's, not the project's.)
- **Finding a take** for any command: `takes/<name>/<file>` first, then `takes.local.toml`. A file
  found is checked against the digest before use, once per command. A missing or changed file is
  refused, naming the take, its digest, where duet looked, and `duet take locate`.
- **No symlinks, ever** (git-annex's lesson). A take is a file in the project or a path in the
  local map.
- **`duet record` in a project** records straight into `takes/<name>/<name>.flac`, adds the take
  as `copied`, and logs it. It is the owner's own recording, so it is copied by definition.
- **A take is never removed by a command.** `duet take forget <name>` drops the entry and its
  local path, and leaves every file in `takes/<name>/` for the owner to delete.

Whether `add` copies by default is [decision 4](#decisions-for-the-owner).

## 5. How commands find and use a project

**Proposed, on top of the ruled walk-up.**

- **A project is the nearest directory, from the working directory up, whose `.duet/` holds
  `project.duet`.** A `.duet/` without one stays what it is today: a config directory, with no
  project behaviour. So nothing changes for anyone who has a `.duet/duet.toml` now.
- **`DUET_PROJECT=<dir>`** names the project outright, for scripts, tests and plugins, whose
  working directory means nothing. It mirrors `DUET_CONFIG_DIR`. duet-e2e clears it, as it already
  isolates config.
- **`duet project path`** prints the project found and how; **`duet project show`** prints the
  project document as text, every take with where it was found (or that it was not), and the
  ignore block's state. They mirror `duet config path | show`.

**What changes inside a project:**
- **A take can be given by name.** `duet capture melody wayfaring` reads the take named
  `wayfaring`. A path still works.
- **Default output locations.** Without `-o`, a capture of a project take writes to
  `takes/<name>/<name>.<reader>.*`. `compare`, `review` and `comp apply` default the same way, and
  `comp apply` looks in `takes/<name>/` for its variants when given no files (its `--in DIR`,
  ruled, defaulted).
- **A capture of a path that is not a project take** is refused inside a project, with the
  `duet take add` line to run, so that nothing derived lands outside the boundary by accident.
  `-o` still wins: given outside `takes/`, the outputs go there, and duet says the take is not in
  the project ([decision 5](#decisions-for-the-owner)).
- **Defaults.** Commands that make or render music without a recording use `defaults`, each
  stated: `duet play` and renders take `tuning`; a new song or grid takes tempo, meter, key and
  grid. Each says "tempo 92, from the project".
- **Capture does not take tempo, meter or key from the project.** Estimating them is what capture
  is for, and a hummed idea is rarely at the song's tempo. Capture states where its estimate
  disagrees with a project default, and `--tempo` gives one, as today. A later Logic tap stamps
  the host's tempo on the take, which is the case where a given tempo is real
  ([decision 6](#decisions-for-the-owner)).

**What never changes:** a given flag wins, every file still says what it is by its envelope, and
no command needs a project. Outside one, every command works exactly as today.

## 6. The licence boundary

**Proposed. The boundary is the `takes/` directory, and duet keeps the rule.**

`init` writes, and `duet take add` and `duet project show` keep, this block in `.gitignore`:

```gitignore
# duet: begin (kept by duet from .duet/project.duet; edit outside these lines)
/takes/*
!/takes/wayfaring/
/renders/
/.duet/*.local.toml
/.duet/run/
/.duet/state/
# duet: end
```

- **`/takes/*`, not `/takes/`,** so that an `open` take can be re-included by name: git cannot
  re-include a path below a directory it has excluded, but can below an excluded directory's
  *contents*.
- **A take is `restricted` by default.** `duet take add --open --licence CC-BY-4.0` marks it open,
  and the block gains its `!` line. Restriction is the take's, so everything derived from it in
  its directory follows it.
- **The block is derived from the project document.** duet rewrites only what is between its
  markers, and only when the document's takes change. The owner's own lines are never touched.
- **`duet project check`** reports, and exits non-zero, when the block does not match the document,
  or when a restricted take's directory is not ignored by it. It reads the files only: it does not
  run git, so it cannot see a file already committed. That limit is stated in its output.
- **A part made from a restricted take.** A song captured from the owner's take is in
  `takes/<name>/`, restricted. Copying it into `parts/` (for example
  `duet apply wayfaring.hmm.notes -o parts/melody.duet`) is the owner's act of keeping it as
  authored, and the boundary does not follow it there. Whether a reading's *notes* should stay
  restricted with their take is [decision 3](#decisions-for-the-owner).
- **The authored half can be a repository of its own.** With the block in place, `git init` in the
  project commits only what is shareable. That is how a cloud session could ever see a project: it
  sees the moments, comps, parts and record, and none of the audio, exactly as today's cloud
  sessions work from what the owner chose to share.
- **The record names takes by name and digest, never by path,** so a shared record leaks no
  machine path.

## 7. Adopting scratch work

**Proposed. Adoption is a plan, then an explicit apply. It copies; it never moves.**

```sh
cd ~/Music/wayfaring && duet init
duet project adopt ~/duet/fixtures/wayfaring          # prints a plan, writes nothing
duet project adopt ~/duet/fixtures/wayfaring --apply  # copies as planned
```

The plan reads every file in the directory given and sorts it:
- **Audio** becomes a take, added as `copied` (or `--named`), its name from its stem.
- **A `.duet` or `.duet.jsonl`** is sorted by its envelope's kind:
  - `capture-analysis`, `performance`, and a song or grid written beside a capture with the same
    stem: derived. It goes to the take whose digest it states. A document from before the take
    digest (`capture-analysis` below the variants minor, `performance` 1.0) states none, so it is
    matched by rate and length, as `compare` matches today, and the plan says so. Such a reading is
    not a variant (ruled, the variants design): the plan lists it, and suggests capturing again
    when a comp is wanted.
  - `comp`: authored, to `comps/`.
  - any other song or grid: authored, to `parts/`.
- **A `.moments` file** goes to `review/`.
- **A `STEM.review/` directory** goes to `takes/<name>/review/`.
- **`.notes`, `.drums` and `.abc` text** goes to `parts/` unless it sits beside a capture of the
  same stem, which makes it derived.
- **Anything else** is listed as left behind, never guessed at.

Then:
- **`--apply` copies** what the plan said, refusing any file that exists at the destination, and
  logs one act per take. The scratch directory is untouched: deleting it is the owner's.
- **Nothing is rewritten.** An adopted document's bytes are the same; its new path is the only
  change. That keeps every digest a comp or view header holds.
- **The duet checkout's `fixtures/`** is the first thing to adopt. Once its work is in projects,
  the `/fixtures/` ignore line can stay as a guard for test output, or go.

## 8. The choice composition forces

The composition task is open: parts layered in one song document, a project-kind document that
orchestrates them, or both. The owner has broader compositions and a richer song layer in mind.
This design must leave room for either, so it names the one choice that cannot be deferred:

> **Is a project one piece, or a workspace that may hold several?**

- **If a project is one piece,** the project document is the natural place for the composition: its
  parts, their order, tracks, alignment and sections. Then `project` 1.0's shape is a guess at the
  composition design, and getting it wrong costs a major.
- **If a project is a workspace,** composition is its own kind (an `arrangement`, say, or a richer
  `song`), and a project may hold several. The project document stays a registry: identity,
  defaults, takes. It gains an optional `pieces` list as a minor once that kind exists.

**Recommendation: a workspace, and composition as its own kind.**
- A song's life already spans several takes and several attempts at one piece. A remix (the Later
  goal) holds stems of one track and several mixes of them.
- It keeps `project` 1.0 small and stable now, which the owner wants soon, and lets the composition
  design take whatever shape the richer song layer needs, without a project major.
- **Room left either way:** if the owner rules "one piece", the composition design adds its fields
  to `project` as a minor (optional, absent meaning "no composition yet"). Nothing in 1.0 is in the
  way. What 1.0 must not do is put parts in it now.

This decides nothing about layered against orchestrated, or how takes with different tempos and
downbeats align. Those stay the composition task's. It is [decision 1](#decisions-for-the-owner).

## 9. The boundary with Temper

**Ruled:** no Temper dependency. **Proposed:** `duet init` never offers to create a Temper context.
The control-plane habit (cloud sessions design, the local session listens, Temper carries results)
needs nothing from duet's files, and an offer inside `init` is a dependency in all but name. A
project's title and digests are what a Temper note would cite, and they are plain text.

## 10. What the layout holds for the bus

**Ruled** (owner, 2026-11-17, temper `01a1e9ff-33f4-7123-85d9-7323abead248`): a server loop is
duet's event bus, built bus-and-CLI first. The CLI always stands alone. When a server is running for
the project, the CLI posts its events to it and can subscribe. The desktop face, later, is Tauri.
**Still open there:** where the line falls between a streaming domain-event log and UI, session and
project state kept in something more like SQLite.

The bus, its events and the CLI's hand-off are the workbench design's. This spec states what the
layout must hold, **Proposed**, and leaves the rest open:

1. **The domain-event log is `log.duet.jsonl` at the project root,** one stream per project. It is
   the durable record: a restarted server replays it, and a CLI with no server appends to it. Its
   kind and its events are the workbench's. It is authored-tier, so it is committed with the
   authored half and names takes by name and digest, never by path ([§6](#6-the-licence-boundary)).
2. **The server is discoverable at a fixed path under `.duet/run/`,** git-ignored and machine-local:
   its socket, and a lock that says a server owns the project, holding its process id. A CLI looks
   there and nowhere else, so the quick check the ruling asks for is one stat and one connect. A
   stale lock (its process gone) is the CLI's to clear. The file names, and whether the transport is
   a Unix socket or a localhost port written into the lock, are the workbench's.
3. **A state store has room at `.duet/state/`,** git-ignored and machine-local, whatever its kind
   turns out to be (SQLite, a key-value file, or nothing). Proposed constraints, for the owner to
   rule with the durable/ephemeral line:
   - **it is never the only copy of authored work.** The project document, the comps, the moments
     and the log stay files, as files first requires; a cloud session or a fresh clone sees no
     state store;
   - **deleting it loses only view and session state** (what was open, selected, playing), and
     anything else in it can be rebuilt from the documents and the log;
   - **it is per machine,** never synced or committed, like `takes.local.toml`.

   If the owner rules that some project-level state lives only in the store, that state is not
   shareable and a cloud session cannot see it. That is the trade
   [decision 11](#decisions-for-the-owner) puts.
4. **A plugin** (the Later goal's tap and play-in) finds its project by `DUET_PROJECT` or a path set
   in its own UI, since a DAW's working directory means nothing, then finds the server as the CLI
   does. Whether a sandboxed AUv3 can reach a project directory at all is the plugin design's check.

**Open, for the workbench design:** the log's kind and events; what the server holds in memory; the
transport; whether the log is ever split or rotated; and whether the state store exists at all.

## Witnesses

For the build tasks below, each in the crate CLAUDE.md names:
- **`crates/duet-schemas/tests/`:** a `project` document round-trips exactly through its reader,
  and validates against `project/1.0.json`; its committed example reads; an unknown field, a
  duplicate take name and an unknown `kept` or `restriction` are each refused by name.
- **`crates/duet-e2e`:**
  - `init` in an empty directory writes exactly the three files, and `init` again is refused
    naming `.duet/project.duet`.
  - `init` in a directory with `.duet/duet.toml` keeps that file byte for byte.
  - `init` appends its block to an existing `.gitignore` and leaves every other line as it was.
  - `take add` a copied and a named take: the copy is byte-identical, the named one is found by the
    local map, and a changed file at that path is refused naming the digest.
  - In a project, `capture melody <name>` writes under `takes/<name>/`, and a capture of a path not
    in the project is refused with the `take add` line.
  - A `.duet/` with no project document behaves exactly as today (the config precedence witness
    still passes unchanged).
  - `project check` fails when a restricted take's directory is re-included by hand.
  - `project adopt` without `--apply` writes nothing; with it, every adopted document's bytes are
    unchanged and the scratch directory is untouched.

## Build tasks this design names

Each names its witness. None is built before the owner rules.

1. **`project` 1.0** in duet-schemas: the type, `Kind`, schema and example. **Needs:** the variants
   design's task 1 (the take digest and `Take` type). **Witnesses:** the duet-schemas ones above.
2. **`duet init` and project discovery:** `init`, the `project.duet` marker, `DUET_PROJECT`,
   `duet project path | show`, the `.gitignore` block. **Needs:** 1. **Witnesses:** the `init`,
   discovery and unchanged-config e2e tests.
3. **Takes in a project:** `duet take add | locate | list | forget`, `takes.local.toml`,
   `duet record` into a project, `duet project check`. **Needs:** 2. **Witnesses:** the `take add`
   and `project check` tests.
4. **Commands that default into a project:** capture, compare, review, `comp apply` and renders take
   a take name and write under `takes/<name>/`; defaults applied and stated. **Needs:** 3, and for
   `comp apply` the variants design's task 2. **Witnesses:** the capture-in-a-project tests.
5. **Adoption:** `duet project adopt` and `--apply`. **Needs:** 3. **Witness:** the adopt test, and
   the owner's `fixtures/` adopted into a project on the owner's machine.

The log's kind, and the server's socket, lock and state store under `.duet/`, are the workbench
design's to build. This design reserves their paths and ignores them.

## Decisions for the owner

1. **A project is a workspace, not one piece** ([§8](#8-the-choice-composition-forces)), so
   composition becomes its own kind later and `project` 1.0 holds no parts. *Recommended: yes.*
2. **The project document lives at `.duet/project.duet`** ([§1](#1-the-layout)), as the documents
   design proposed, rather than `project.duet` at the root as the jobs note drew it. One marker,
   and a clean root, against a document hidden in Finder. *Recommended: `.duet/project.duet`.*
3. **A reading's notes stay restricted with their take** ([§6](#6-the-licence-boundary)) until the
   owner copies them into `parts/`. The alternative is to treat notes as shareable and only audio
   (the take, the performance's pitch, clips, renders of the take) as restricted, which would let a
   cloud session see readings of restricted takes. *Recommended: restricted with the take; copying
   to `parts/` is the explicit release.*
4. **`duet take add` copies by default** ([§4](#4-sources-named-without-copying)), with `--named`
   to leave a file where it is. Copying makes a project whole and survives a phone being unplugged;
   naming saves space and leaves a file that may not be the owner's to copy where it is.
   *Recommended: copy by default.*
5. **Inside a project, capturing a file that is not a take is refused** with the `take add` line,
   rather than adding it silently ([§5](#5-how-commands-find-and-use-a-project)). *Recommended:
   refuse.*
6. **Capture never takes tempo, meter or key from the project's defaults**; it estimates, states,
   and says where it disagrees ([§5](#5-how-commands-find-and-use-a-project)). Commands that make
   or render music without a recording do use them. *Recommended: yes.*
7. **`duet init` does not run `git init`** ([§3](#3-what-duet-init-writes)); it writes the
   `.gitignore` block and says to. cargo and uv run git by default; duet would then depend on a git
   binary, and the owner may keep the authored half in a repository elsewhere.
   *Recommended: no git, a printed line instead.*
8. **The boundary directory is named `takes/`** ([§1](#1-the-layout)), for what it holds, rather
   than for the rule it carries (`private/`, `restricted/`). *Recommended: `takes/`.*
9. **`renders/` is ignored by default** ([§6](#6-the-licence-boundary)), as build output, though a
   render of an authored part is shareable. *Recommended: ignored; commit a render by hand when one
   matters.*
10. **The composition task does not fold into this one.** It stays its own task, constrained by
    decision 1. *Recommended: keep it separate.*
11. **The state store is never the only copy of authored work**
    ([§10](#10-what-the-layout-holds-for-the-bus)). It holds view and session state and what can be
    rebuilt; the project document, comps, moments and log stay files. This is to rule together with
    the bus decision's open durable/ephemeral line. *Recommended: yes.*

## Not decided here

- **The log's kind and events, the bus, and the state store's kind:** the workbench design's.
- **Composition:** layered or orchestrated, and alignment across takes: the composition task's.
- **A take kind of its own**, listing a take's variants (the variants design left it to "the project
  and session record"): the project document's `takes` entry carries the take's identity, and the
  readings in `takes/<name>/` are its variants. A separate kind is not needed until a take carries
  more than that.
- **Converting a take** (a phone's M4A to FLAC) on adding: not proposed, since a byte copy is never
  in question.
- **Moving or renaming a take,** and a project-wide cleanup of stale derived files: later, once
  there are projects to clean.

## Sources

Outside facts, read 2026-11-17, secondary unless stated:
- Logic Pro, package or folder, and copying audio: https://support.apple.com/en-in/101354 and
  https://support.apple.com/en-mz/guide/logicpro/lgcp58b54e81/mac; packages broken by sync:
  https://www.dropboxforum.com/t5/Files-folders/Logic-Pro-X-Files-being-converted-to-folders-can-t-open/m-p/364580
- Ardour's session folder: https://manual.ardour.org/working-with-sessions/whats-in-a-session
- REAPER's media and peaks: https://reaper.blog/2013/07/reaper-101-file-management/ ,
  https://reaper.blog/2012/01/reaper-101-peaks-folder and
  https://www.soundonsound.com/techniques/reaper-keeper
- DVC's `.dvc` files: https://dvc.org/doc/user-guide/dvc-files/.dvc (not reached from this session;
  summarised from search results)
- git-annex's backends and unlocked files: https://man.archlinux.org/man/git-annex-unlock.1.en
- uv's `init`: https://docs.astral.sh/uv/concepts/projects/init/
- cargo's `new` and `init`: from the cargo book, as known; not re-read this session.
- `temper init`: the Temper decision "/temper init becomes a routed skill command"
  (temper `01a08da2-115b-7592-ac7f-c7a41acc024a`).
