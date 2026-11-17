# duet projects: `duet init`, a project document, and a layout of takes and songs

**Date:** 2026-11-17. **Task:** temper `01a1e007-8747-77ee-8333-dd462d6d01eb`.

**Status: Ruled (owner, 2026-11-17).** The owner ruled the design's eleven decisions ("yes, those
stand"), with two dropped and two changed, and added one: temper
`01a1eb60-07f8-77a6-8a0d-fb3947f0ba0d`, read together with the domain model, temper
`01a1eb48-6b7a-7797-bb2b-68f335de87e9`. The rulings are listed in
[Decisions (ruled 2026-11-17)](#decisions-ruled-2026-11-17). Decisions 12 and 13, taken in from the
bus design after the eleven were set out, are still open, and one line `init` writes is in tension
with a ruling ([§3](#3-what-duet-init-writes)). What is still marked **Proposed** is the design those
rulings rest on: the owner ruled its decisions, not each line. Nothing here is built.

**Builds on:**
- [duet documents](2026-11-15-duet-documents-design.md): the `.duet` envelope, kinds, the version
  policy, and the constraint that the project kind lives inside `.duet/` (its "Project, deferred").
- [Variants and comps](2026-11-17-variants-and-comps-design.md): the take digest over decoded
  samples, readings named as variants by annotation, comps that never name a path.
- [Songs and composition](2026-11-17-songs-and-composition-design.md) (**Proposed**): what a song
  folder holds, the composition, and how a take is placed on a song's bars. It takes over the
  composition task (temper `01a1dc62-9d9f-73ae-b4ec-03b07a08e407`), folded in by decision 10.
- The config decision (temper `01a1daa7-5cea-7301-80a6-7d3f1905f3a7`) and its build: `duet.toml`
  and `duet.local.toml` in the user's config directory and a project's `.duet/`, found by walking
  up from the working directory (`crates/duet-cli/src/config.rs`).
- The jobs-and-boundaries note (temper `01a1e57c-f9c8-72e5-8a9b-6a4e5874bfdc`): section 4, the
  owner's rulings on what duet is, and section 8, the layout by what each thing is.
- The workbench task (temper `01a1e563-22e6-70e6-a7ce-8b38e93b3091`), and the owner's ruling on it:
  a server loop is duet's event bus, built bus-and-CLI first, and the CLI always stands alone
  (temper `01a1e9ff-33f4-7123-85d9-7323abead248`, 2026-11-17). The bus is designed, and its fifteen
  decisions ruled, in [the bus design](2026-11-17-bus-design.md) (#75). This spec states only what
  the layout holds for it ([§10](#10-what-the-layout-holds-for-the-bus)).

**Grounding:** the code at `main` (`bb58d0f`, `capture-analysis` 1.12) when written. The take digest
of the variants design (its build task 1) has since merged (#73): `capture-analysis` 1.13 and
`performance` 1.1 state the take, and `Take` lives in `crates/duet-dsp/src/take.rs`.

## The problem

Every take, reading, review sheet and verdict today sits in scratch space or in the duet checkout's
`fixtures/`, named on the spot. That fails two jobs from the jobs note:
- **J8:** coming back next week and finding a song's takes, readings, rulings and drafts.
- **C6:** the next session, local or cloud, finding what this one found.

And the owner works on several songs at once, each with takes that may serve more than one of them.
A project is where all of that lives on the owner's machine.

## What is already settled

- **There is no special project extension.** A project is a document of kind `project`, written
  as any `.duet` document is. **Ruled** (owner, 2026-11-15; the `duet init` task's "Settled").
- **`<project>/.duet/` is the project's config directory**, holding `duet.toml` and
  `duet.local.toml`, found by walking up from the working directory, nearest first. **Ruled** (the
  config decision) **and built** (`config.rs`, witnessed in `crates/duet-e2e/tests/cli.rs`).
- **A given flag always wins** over any file's default, as `--tempo` does today. **Ruled** (the
  config decision's precedence; the `duet init` task, question 3).
- **The project document is `.duet/project.duet`.** **Ruled** (decision 2).
- **The capture is never altered.** Every change is stated beside it. **Ruled** (owner,
  2026-11-16, the jobs note section 4).
- **A take is identified by a digest of its decoded samples,** with its rate, channels and frames.
  Readings are variants of a take by that digest, and a comp names variants by annotation, never by
  path. **Ruled** (owner, 2026-11-17, the variants design, rulings 1 and 4).
- **A project is a workspace that holds songs,** one folder each, and **a take belongs to the
  project,** linked to a song and a part, able to serve several. **Ruled** (decision 1, the new
  decision, and the domain model's rulings 3 and 6).
- **A session is not a duet entity.** Continuity is the journal plus the project's files. **Ruled**
  (the domain model, ruling 1).
- **There is no sharing status in a project,** and **git is not duet's concern.** **Ruled**
  (decisions 3 and 7; [§6](#6-no-sharing-status-and-no-git)).
- **No Temper dependency.** A project is files in a directory. **Ruled** (the charter's
  `no-temper-dependency` constraint).
- **Model weights live in a per-user cache, never in a project.** **Ruled** (owner, 2026-11-16,
  "for now"; the jobs note section 9). Basic Pitch's model is embedded and needs no cache.

## Prior art, and what duet takes from it

Secondary sources, read 2026-11-17, listed at the end. What was not verified is said.

| Tool | What it does | Taken | Left |
|---|---|---|---|
| **Logic Pro** | A project is a package or a folder. "Copy audio files into project" decides whether imported audio is copied in or referenced where it is. Apple notes a folder lets several project versions share one set of audio. | The choice between copying a source in and naming it where it is, made per source and stated. A plain folder, never a package: a package hides files from the CLI and from Claude, and sync tools have broken packages into folders. One set of takes shared by several songs. | The package. |
| **Ardour** | A session is a folder with a fixed layout: `interchange/` for recorded and imported audio, `peaks/` and `analysis/` that are safe to delete and are rebuilt, `export/`, and the session file with its `.bak` and `.history`. Its manual says to move the whole folder, never the session file alone. | Fixed, named directories by what a file is. Rebuildable data in a place of its own. The record of actions (Ardour's `.history`) kept beside the state, not in place of it. | Undo history as a separate file per save: duet's journal is one append-only stream. |
| **REAPER** | Media is referenced by path unless "copy all media into project directory" is ticked. A relative media path keeps a project portable; an absolute one breaks it. `.reapeaks` are rebuilt if missing. A shared media folder cleaned by one project can delete another's media. | Paths inside a project are always relative. A source outside the project is found by a machine-local map, never by a path written into the project document. One project's cleanup never touches anything outside it. | |
| **DVC** | A small `.dvc` file names a large file by its hash, size and relative path; the content lives in a cache. | **The core idea:** the project document names a source by its content digest, and finds its content separately. duet's digest is already ruled. | A content-addressed cache directory of duet's own. A take is few and large, and its name is the owner's, not a hash. |
| **git-annex** | Locked files are symlinks to content named by a key (`SHA256E` by default: a SHA-256 of the content and the extension). Unlocked files are pointer files behind git's clean and smudge filters, because symlinks get in the way of tools that edit files. | Identity by digest, and the lesson of its unlocked mode: **no symlinks.** They fail on Windows, in sync tools and in cloud sessions, and an audio app that follows one may write through it. | Any git integration. duet never runs git, hooks or filters. |
| **cargo** | `cargo new PATH` makes a directory; `cargo init` adopts the current one and refuses where `Cargo.toml` exists. It runs `git init` unless already inside a repository, and `--vcs none` turns that off. | **`init` works in an existing, non-empty directory and never overwrites.** One command for a new and an existing directory, as `uv init [PATH]` does. A refusal names what already exists. | Running git (ruled out, decision 7). |
| **uv** | `uv init` writes `pyproject.toml`, a `.gitignore`, a `.python-version` pin and a README; `--bare` writes only the manifest. | A minimal default: the project document and a commented config, and nothing that has to be deleted. The tool version is recorded in the document, as `.python-version` pins Python. | A README, a `.gitignore` and a source tree written by default. |
| **`temper init`** | A config wizard for the machine, and a separate project-setup flow in which an agent writes the project's prose in conversation. Temper ruled against scaffolding prose inside the binary, because an agent would still have to fill it. | **The split:** `duet init` writes structure and metadata only. Anything that needs words (the title is the one exception, and it defaults to the directory's name) is written by the owner or by Claude afterwards, through ordinary commands. | Creating a Temper context. That stays out of duet ([§9](#9-the-boundary-with-temper)). |

**Not verified first-hand:** Logic's internal package layout (Apple does not document it; only the
copy setting and the package-or-folder choice are from Apple), DVC's file details (its docs site was
out of reach from this session), and git-annex's repository-version details. None of the proposals
rests on them.

## 1. The layout

**Ruled:** takes in `takes/` at the project root (decision 8), the project document in `.duet/`
(decision 2), one folder per song under `songs/` (the new decision), and rebuildable renders in
`renders/` (decision 9). **Proposed:** the rest of the tree.

The jobs note's section 8 sorts a project's contents by what each thing is. This keeps that sorting
and changes its illustrative tree in four ways:
1. **Sources and the files derived from them share one directory per take,** under `takes/`. A
   clip of a take *is* the take, and a reading is rebuilt from it.
2. **The project document sits in `.duet/`,** so one marker (`.duet/`) finds a project and the
   root holds only the owner's work.
3. **Each song has a folder,** `songs/<song>/`, holding its song document (with its composition),
   its parts, its comps and its snapshots. A take stays at the root because it can serve several
   songs.
4. **Comps live in their song,** since a comp builds one of that song's parts from its takes.

```text
porch/                              the project: any directory, outside the duet checkout
  .duet/
    project.duet                    the project document (kind: project)        authored
    duet.toml                       project config, as today                    authored
    duet.local.toml                 this machine's config, as today             machine-local
    takes.local.toml                where named takes live on this machine      machine-local
    run/                            the server's socket and lock, when running  machine-local
    state/                          the view-state store, when a face needs it  machine-local
  .gitattributes                    one line, in tension with decision 7 (§3)
  journal.duet.jsonl                the domain-event journal                    the journal
  takes/
    wayfaring/                      one take: its source and everything from it
      wayfaring.flac                the source, when copied in (absent when named)
      wayfaring.performance.duet.jsonl
      wayfaring.hmm.capture.duet    a reading (a variant)
      wayfaring.hmm.duet            its notes, as a part
      wayfaring.hmm.abc             its score
      wayfaring.rules.capture.duet  another variant
      compare/                      compare's SVGs and WAVs
      review/                       a review sheet and its clips
  review/
    wayfaring.moments               moments: times and labels, no audio         authored
  songs/
    wayfaring-stranger/             one song
      song.duet                     the song and its composition (kind: song)   authored
      parts/
        melody.duet                 a part (kind: part), with its source        authored
        bass.duet
        kit.duet                    a drum part (kind: drum-grid)
      comps/
        melody.comp.duet            picks across takes, on the song's bars      authored
      snapshots/
        2026-11-20-first-verse/     a checkpoint: copies, never changed         authored
  renders/                          MIDI and audio rendered from parts and songs rebuildable
```

### What each thing is

| What | Where | Kept how |
|---|---|---|
| **Sources** | `takes/<take>/<file>`, or named where they are ([§4](#4-sources-named-without-copying)) | Never modified. Identified by digest |
| **Derived from a source** | `takes/<take>/` | Rebuilt from the source and the tool's version. A reading names its reader and passes, so variants sit side by side |
| **Songs** | `songs/<song>/song.duet` | The song and its composition. Changed in place, every change in the journal |
| **Parts, comps** | `songs/<song>/parts/`, `songs/<song>/comps/` | The song's working files. Every change in the journal |
| **Snapshots** | `songs/<song>/snapshots/<name>/` | Copies, written once and never changed |
| **Other authored files** | `.duet/project.duet`, `.duet/duet.toml`, `review/` | Small and precious |
| **The journal** | `journal.duet.jsonl` | Append-only |
| **Rebuildable** | `renders/` | Rebuilt from parts and songs. No special status |
| **Machine-local** | `.duet/*.local.toml`, `.duet/run/`, `.duet/state/` | This machine only |
| **Outside the project** | the user's config directory, the weights cache, soundfonts | Per user |

Every file in a project is the owner's. None has a sharing status ([§6](#6-no-sharing-status-and-no-git)).

### Rules the layout keeps

- **Every path a project file writes is relative to the project root** (or, inside a song, to the
  song's folder). An absolute path appears only in `.duet/takes.local.toml`. (REAPER's lesson.)
- **A take's name is the owner's,** a short lowercase name with hyphens (`wayfaring`,
  `blood-sport-v2`), unique in the project, and the directory and file stems under `takes/` use
  it. Nothing reads a file's name to learn what it is: the envelope says that, as today.
- **A song's name follows the same rule,** unique in the project, and is its folder's name. Its
  title for people is in `song.duet`.
- **A part's name is its stem,** unique in its song.
- **A reading's stem is `<take>.<label>`,** where the label is its reader (`hmm`, `rules`,
  `basic-pitch`). This is the variants design's own example (`wayfaring.hmm.capture.duet`), made
  the project's default. A second reading with the same label is refused, as every capture output
  is today; `-o` names another.
- **The performance is written once per take,** at `takes/<take>/<take>.performance.duet.jsonl`, if,
  as appears from the code, it does not depend on the reader. A capture that finds one already
  there checks it is of the same take (`performance` 1.1's `take`, ruled in the variants design) and
  the same duet version, and reuses it; otherwise it refuses, naming both. *To verify when built.*
- **Directories are made when first written,** not by `init`. A project with no songs has no
  `songs/`. The names are fixed by this spec, not by what exists.
- **Projects live outside the duet checkout.** The duet repository is not a project. Its
  `fixtures/` rule (the owner's takes are never committed there) is a rule for developing duet, not
  a property of a project ([§6](#6-no-sharing-status-and-no-git)).

### Where review sets, verdicts and comps go

The `duet init` task asks this for the workbench (its "Added 2026-11-16").
- **A moments file** is authored (times and labels, no audio) and belongs to a take:
  `review/<take>.moments`.
- **A review sheet** is HTML with clips cut from the take, so it is the take's:
  `takes/<take>/review/`. Today `duet capture review` writes `STEM.review/`; in a project its
  default `--dir` becomes this.
- **A verdict that picks or marks** is a comp entry (variants design §8, ruled), in the comp of the
  song and part it builds: `songs/<song>/comps/<part>.comp.duet`.
- **A verdict that picks nothing** ("no audible difference") lives in the journal, as the variants
  design says. Whether the review task also keeps answers beside the moments file
  (`review/<take>.answers`, say) is that task's call; `review/` is where it would go.

## 2. The project document: `project` 1.0

**Proposed. A new kind, `project` 1.0,** a `.duet` document at `.duet/project.duet` (ruled,
decision 2), with its schema and example committed under `crates/duet-schemas/schemas/project/`.

```json
{
  "$schema": "https://raw.githubusercontent.com/tasker-systems/duet/main/crates/duet-schemas/schemas/project/1.0.json",
  "duet": { "kind": "project", "version": "1.0", "tool": { "name": "duet", "version": "0.5.0" } },
  "body": {
    "title": "Porch",
    "created": "2026-11-17",
    "duet": "0.5",
    "defaults": { "tempo_bpm": 92.0, "meter": "3/4", "key": "Ab", "tuning": "+0c", "grid": "1/16" },
    "takes": [
      {
        "name": "wayfaring",
        "take": { "digest": "sha256:9f2c…", "rate": 44100, "channels": 1, "frames": 976080 },
        "file": "wayfaring-stranger.flac",
        "added": "2026-11-17",
        "given": { "tempo_bpm": 92.0 }
      },
      {
        "name": "blood-sport",
        "take": { "digest": "sha256:41ab…", "rate": 48000, "channels": 1, "frames": 1203200 },
        "file": "Blood Sport 3.m4a",
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
  their flags spell them. A new song's composition starts from them; renders take `tuning`; a new
  grid takes `grid` ([§5](#5-how-commands-find-and-use-a-project)). Capture never takes them.
  - **The lane map is not here.** Lanes are config, in `.duet/duet.toml`, as ruled in the
    documents design ("Lane maps and config are not kinds"). One place for each thing.
- **`takes`**, required, possibly empty: the project's sources
  ([§4](#4-sources-named-without-copying)).
  - `name`: as in the layout's rules. Unique.
  - `take`: the variants design's `take` object, the same type. The digest is the identity.
  - `file`: the source's own file name when added, without its directory. Information, for people
    and for finding it again; never a path.
  - `added`: the date it was added.
  - `given`, optional: values the owner gave this take (`tempo_bpm`, `meter`, `key`), remembered so
    every later capture of it uses them, stated as given. **Ruled** (decision 6: "a value the owner
    gives a take is remembered for that take"); its shape is proposed.
  - **No `kept` field.** Whether a take was copied in or named is found, not stored: a file at
    `takes/<take>/<file>`, else an entry in `.duet/takes.local.toml`. **No `restriction` and no
    `licence`.** There is no sharing status (ruled, decision 3), and licensing for release is a
    later question. A take's licence notice, when it has one, is a file in its directory like any
    other.
- **No `songs` list.** A song is a folder in `songs/` holding a `song.duet`. Listing them here too
  would be a second place for one fact. Which songs use a take is read from the songs, which link
  to their takes ([the songs design](2026-11-17-songs-and-composition-design.md#the-song)).

### The version

- **1.0, new.** The version policy is ruled; nothing about it changes here.
- **Expected minors:** a take's optional provenance (`from: "duet record" | "logic tap" | …`), more
  `given` fields, and new `defaults` fields. Each is an optional field whose absence means what 1.0
  meant.
- **Why `defaults` and `given` are closed objects** and not free tables: unknown fields are refused
  at every version (ruled), so a typo in one is caught, not silently ignored.

### The Rust type

`Project` beside the other kinds' types, with `Kind` implemented in `duet-schemas`, as for every
kind (ruled in the documents design: no crate below `duet-schemas` reads or writes an enveloped
document). `Take` is the variants design's type, shared, not copied.

## 3. What `duet init` writes

**Proposed.**

```sh
duet init                      # the working directory becomes a project
duet init ~/Music/porch        # makes the directory if it is missing
duet init --title "Porch"
```

It writes, and prints each path it wrote:
1. **`.duet/project.duet`**: `project` 1.0 with `title`, `created`, `duet`, empty `takes`, and no
   `defaults`.
2. **`.duet/duet.toml`**, only if absent: every line a comment, naming what may be set here
   (`[song] end`, `[play] soundfont`, `[lanes.*]`), so it changes nothing until edited. An existing
   one is left alone and named.
3. **`.gitattributes`**: `journal.duet.jsonl merge=union`, appended between marker lines to an
   existing file, so two clones that each appended to the journal merge without a conflict.
   **Ruled** (owner, 2026-11-17, the bus design's decision 12). **In tension with project decision
   7, for the owner to rule:** decision 7 rules that git is not duet's concern and `init` writes no
   git file. This line is kept as the bus design ruled it until the owner rules between the two.

It does not:
- **overwrite.** A directory that already holds `.duet/project.duet` is refused, naming it. A
  `.duet/` with only config in it is adopted: it gains a project document and keeps its config.
- **move, copy or delete anything.** Adopting files is a separate, explicit step
  ([§7](#7-adopting-scratch-work)).
- **write a `.gitignore`, run git, or print a git line.** **Ruled** (decision 7).
- **make directories** beyond `.duet/`.
- **make a song.** `duet song new <name>` does that
  ([the songs design](2026-11-17-songs-and-composition-design.md)).
- **touch the network or Temper.**
- **write prose,** beyond a title it can derive. (`temper init`'s split.)

**Nested projects.** `init` inside an existing project is refused, naming the project above,
unless `--nested` is given. A nested project is legal (the nearest `.duet/` wins, as today) but is
nearly always a mistake.

**A `.duet` in a home directory.** Today a `~/.duet/` would make every directory below it a
project for config. `init` refuses to make one in the home directory itself.

## 4. Sources named without copying

**Ruled:** `duet take add` copies by default, and `--named` leaves the file where it is (decision
4). **Proposed:** the rest. A take is added to the project explicitly.

```sh
duet take add ~/Downloads/idea.m4a --name idea              # copied into takes/idea/
duet take add "/Volumes/Phone/Blood Sport 3.m4a" --named    # left where it is, named by digest
duet take locate blood-sport "/Volumes/Archive/Blood Sport 3.m4a"   # found again elsewhere
duet take list                                              # every take, where it is found, or not
```

- **Adding decodes the whole file once** and computes the take digest (the variants design's
  digester), its rate, channels and frames.
- **Copied:** the file is copied byte for byte to `takes/<take>/<file>`. Never transcoded: the
  capture is never altered (ruled), and a byte copy needs no argument about whether a re-encode is
  the same take. Copying makes a project whole and survives a phone being unplugged.
- **Named:** nothing is copied. The absolute path goes into `.duet/takes.local.toml`, which is
  machine-local:

  ```toml
  [blood-sport]
  path = "/Volumes/Phone/Blood Sport 3.m4a"
  ```

  The project document holds only the digest and the file's name. (DVC's pointer file, with
  REAPER's lesson that a path is the machine's, not the project's.)
- **Finding a take** for any command: `takes/<take>/<file>` first, then `takes.local.toml`. A file
  found is checked against the digest before use, once per command. A missing or changed file is
  refused, naming the take, its digest, where duet looked, and `duet take locate`.
- **No symlinks, ever** (git-annex's lesson). A take is a file in the project or a path in the
  local map.
- **`duet record` in a project** records straight into `takes/<take>/<take>.flac`, adds the take,
  and records it in the journal. With `--song`, it plays the song's click and places the take on
  its bars ([the songs design, §4](2026-11-17-songs-and-composition-design.md#4-how-a-take-maps-onto-the-bars)).
- **Linking a take to a song and a part** is the song's: the song document names its takes
  ([the songs design, §1](2026-11-17-songs-and-composition-design.md#the-song)).
- **A take is never removed by a command.** `duet take forget <take>` drops the entry and its
  local path, and leaves every file in `takes/<take>/` for the owner to delete. A song that still
  links it says so.

## 5. How commands find and use a project

**Proposed, on top of the ruled walk-up.**

- **A project is the nearest directory, from the working directory up, whose `.duet/` holds
  `project.duet`.** A `.duet/` without one stays what it is today: a config directory, with no
  project behaviour. So nothing changes for anyone who has a `.duet/duet.toml` now.
- **`DUET_PROJECT=<dir>`** names the project outright, for scripts, tests and plugins, whose
  working directory means nothing. It mirrors `DUET_CONFIG_DIR`. duet-e2e clears it, as it already
  isolates config.
- **`duet project path`** prints the project found and how; **`duet project show`** prints the
  project document as text, every take with where it was found (or that it was not), and every
  song. They mirror `duet config path | show`.

**What changes inside a project:**
- **A take can be given by name.** `duet capture melody wayfaring` reads the take named
  `wayfaring`. A path still works.
- **Default output locations.** Without `-o`, a capture of a project take writes to
  `takes/<take>/<take>.<reader>.*`. `compare` and `review` default the same way. `comp apply`
  writes the part its comp names into the song's `parts/`.
- **A capture of a path that is not a project take** is refused inside a project, with the
  `duet take add` line to run. **Ruled** (decision 5). `-o` still wins: given outside `takes/`, the
  outputs go there, and duet says the take is not in the project.
- **Defaults.** Commands that make or render music without a recording use `defaults`, each
  stated: `duet play` and renders take `tuning`; a new song's composition and a new grid take
  tempo, meter, key and grid. Each says "tempo 92, from the project".
- **Capture estimates, then compares.** **Ruled** (decision 6). Capture always estimates tempo,
  meter and key from the take. When the take is linked to a song, it states where its estimate
  disagrees with that song's composition. A take is placed on the song's bars by the song's tempo
  map plus an offset (the click case). A value the owner gives a take (`--tempo`) is remembered on
  the take's `given`, and used by every later capture of it, stated as given. How a take recorded
  without a click is placed is open ([the songs design](2026-11-17-songs-and-composition-design.md#open-questions)).

**What never changes:** a given flag wins, every file still says what it is by its envelope, and
no command needs a project. Outside one, every command works exactly as today.

## 6. No sharing status, and no git

**Ruled** (owner, 2026-11-17, decisions 3, 7 and 9, temper `01a1eb60`).

- **There is no "private", "restricted" or sharing status in a project.** The owner asked: "private
  to whom?" A project is on the user's own machine, and every file in it is theirs. No field, no
  directory and no command marks a file as shareable or not.
- **Licensing for release is a later question,** not a project concept now.
- **The duet repository's `fixtures/` rule stays a rule for developing duet.** The owner's takes are
  never committed to the duet repository. That is the repository's concern, kept by its own
  `.gitignore`, and never a property of a project.
- **Git is not duet's concern.** Someone may keep a project in git, but duet neither assumes nor
  manages that: `init` writes no `.gitignore`, runs no git and prints no git line, and no command
  reads git's state. The one git file `init` still writes, `.gitattributes`, is in tension with
  this ruling and is for the owner to rule ([§3](#3-what-duet-init-writes)).
- **Renders are rebuildable files in `renders/`,** with no special status. Deleting `renders/`
  loses nothing that cannot be made again.
- **The journal names takes by name and digest, never by path,** so it holds no machine's path and
  stays true when the project moves.

## 7. Adopting scratch work

**Proposed. Adoption is a plan, then an explicit apply. It copies; it never moves.**

```sh
cd ~/Music/porch && duet init
duet project adopt ~/duet/fixtures/wayfaring --song wayfaring-stranger          # prints a plan, writes nothing
duet project adopt ~/duet/fixtures/wayfaring --song wayfaring-stranger --apply  # copies as planned
```

The plan reads every file in the directory given and sorts it:
- **Audio** becomes a take, added as copied (or `--named`), its name from its stem.
- **A `.duet` or `.duet.jsonl`** is sorted by its envelope's kind:
  - `capture-analysis`, `performance`, and a part or grid written beside a capture with the same
    stem: derived. It goes to the take whose digest it states. A document from before the take
    digest (`capture-analysis` below 1.13, `performance` 1.0) states none, so it is
    matched by rate and length, as `compare` matches today, and the plan says so. Such a reading is
    not a variant (ruled, the variants design): the plan lists it, and suggests capturing again
    when a comp is wanted.
  - `comp`: to the song's `comps/`.
  - any other part or grid: to the song's `parts/`.
- **A `.moments` file** goes to `review/`.
- **A `STEM.review/` directory** goes to `takes/<take>/review/`.
- **`.notes`, `.drums` and `.abc` text** goes to the song's `parts/` unless it sits beside a capture
  of the same stem, which makes it derived.
- **Without `--song`,** takes and derived files are planned as usual, and every part and comp is
  listed as needing a song. `--song` names one, made if it does not exist.
- **Anything else** is listed as left behind, never guessed at.

Then:
- **`--apply` copies** what the plan said, refusing any file that exists at the destination, and
  writes one act per take to the journal. The scratch directory is untouched: deleting it is the
  owner's.
- **Nothing is rewritten.** An adopted document's bytes are the same; its new path is the only
  change. That keeps every digest a comp or view header holds. A part adopted from before the
  `part` rename states no `source`; the plan says so.
- **The duet checkout's `fixtures/`** is the first thing to adopt.

## 8. A workspace, and composition as its own spec

**Ruled** (decisions 1 and 10). **A project is a workspace that holds songs,** not one piece, and
each song holds its composition. **Composition is folded into the domain model** and is its own spec,
[songs and composition](2026-11-17-songs-and-composition-design.md), which this layout uses.

- **`project` 1.0 holds no parts and no composition.** It is a registry: identity, defaults and
  takes.
- **A song's composition is in its `song.duet`**: sections, the tempo, meter and key maps, and where
  each part sits on the timeline. A song that changes tempo and key is still one song.
- **A take can serve several songs,** placed on each song's bars by that song's tempo map plus an
  offset.

## 9. The boundary with Temper

**Ruled:** no Temper dependency. **Proposed:** `duet init` never offers to create a Temper context.
The control-plane habit (cloud sessions design, the local session listens, Temper carries results)
needs nothing from duet's files, and an offer inside `init` is a dependency in all but name. A
project's title and digests are what a Temper note would cite, and they are plain text.

## 10. What the layout holds for the bus

**Ruled** (owner, 2026-11-17, temper `01a1e9ff-33f4-7123-85d9-7323abead248`): a server loop is
duet's event bus, built bus-and-CLI first. The CLI always stands alone. When a server is running for
the project, the CLI posts its events to it and can subscribe. The desktop face, later, is Tauri.

The bus, its events, the wire and the CLI's hand-off are
[the bus design](2026-11-17-bus-design.md)'s (#75), whose fifteen decisions are ruled. It used the
paths this section reserved and asked for four things more (its
["What this asks of the project layout"](2026-11-17-bus-design.md#what-this-asks-of-the-project-layout)).
What the layout holds:

1. **The journal is `journal.duet.jsonl` at the project root,** one stream per project, kind
   `journal` 1.0, read with `duet journal show | check`. **Ruled** (owner, 2026-11-17, the bus
   design's decision 15): it was `log.duet.jsonl`, but it is more than a log, and many things are
   logs. It is the durable record: a restarted server replays it, and a CLI with no server appends
   to it under the journal lock. It carries each song's history, since a session is not a duet
   entity (the domain model, ruling 1). It names takes by name and digest, never by path
   ([§6](#6-no-sharing-status-and-no-git)).
   - **Git merges it by union** (`.gitattributes`, written by `init`,
     [§3](#3-what-duet-init-writes)). **Ruled** (the bus design's decision 12). **In tension with
     project decision 7, for the owner to rule.**
   - **One file, never rotated, in v1.** A year of sessions is a few megabytes. Rotation, if ever
     needed, is a later change to the layout. **Proposed** (the bus design's layout ask 4;
     [decision 12](#still-open)).
2. **The server is found through a lock at a fixed path, `.duet/run/bus.lock`,** machine-local. It is
   a small TOML file naming the server's pid and **its socket's path**. The socket is usually in
   `.duet/run/` too, and falls back to `$TMPDIR` when the project's path is too long for a Unix
   socket (macOS allows 104 bytes). A CLI reads only the lock. **Ruled** (the bus design's decision
   6), refining this section's earlier "a fixed path for the socket".
   - **A stale lock is left for the next server to overwrite.** The CLI does not clear it: the
     `flock` already says it is stale, and a CLI that deleted it could race a server that is just
     starting. **Proposed** (the bus design's layout ask 2; [decision 13](#still-open)). This
     replaces this section's earlier "a stale lock is the CLI's to clear".
3. **The state store is `.duet/state/state.sqlite`,** machine-local, holding only view state (open
   take and moment, layout, each face's last-read cursor) and caches. It is **not built until a face
   needs it**, so the CLI-first build has none. Anything a person or Claude acts on (open asks, a
   review in progress, the current comp) is a projection of the journal and the documents, never
   store-only, so deleting `.duet/state/` loses where the windows were and nothing else. A cloud
   session or a fresh copy of the project loses nothing it needs. **Ruled** (the bus design's
   decision 1, and decision 11).
4. **A plugin** (the Later goal's tap and play-in) finds its project by `DUET_PROJECT` or a path set
   in its own UI, since a DAW's working directory means nothing, then finds the server through the
   lock as the CLI does. Whether a sandboxed AUv3 can reach a project directory at all is the plugin
   design's check.

## Witnesses

For the build tasks below, each in the crate CLAUDE.md names:
- **`crates/duet-schemas/tests/`:** a `project` document round-trips exactly through its reader,
  and validates against `project/1.0.json`; its committed example reads; an unknown field, a
  duplicate take name and an unknown `given` field are each refused by name.
- **`crates/duet-e2e`:**
  - `init` in an empty directory writes exactly the files [§3](#3-what-duet-init-writes) lists, and
    `init` again is refused naming `.duet/project.duet`.
  - `init` writes no `.gitignore` and prints no git line.
  - `init` in a directory with `.duet/duet.toml` keeps that file byte for byte.
  - `take add` with no flag copies, byte-identical; `--named` copies nothing and the take is found
    by the local map; a changed file at that path is refused naming the digest.
  - A capture of a take with `--tempo 92` records `given`, and the next capture of it with no flag
    states tempo 92 as given.
  - In a project, `capture melody <take>` writes under `takes/<take>/`, and a capture of a path not
    in the project is refused with the `take add` line.
  - A `.duet/` with no project document behaves exactly as today (the config precedence witness
    still passes unchanged).
  - `project adopt` without `--apply` writes nothing; with it, every adopted document's bytes are
    unchanged and the scratch directory is untouched.

## Build tasks this design names

Each names its witness. The decisions are ruled, so each can be filed once the songs design is
ruled for the parts that need it.

1. **`project` 1.0** in duet-schemas: the type, `Kind`, schema and example. **Needs:** the variants
   design's task 1 (the take digest and `Take` type), merged in #73. **Witnesses:** the
   duet-schemas ones above.
2. **`duet init` and project discovery:** `init`, the `project.duet` marker, `DUET_PROJECT`,
   `duet project path | show`. **Needs:** 1. **Witnesses:** the `init`, discovery and
   unchanged-config e2e tests.
3. **Takes in a project:** `duet take add | locate | list | forget`, `takes.local.toml`, `given`,
   `duet record` into a project. **Needs:** 2. **Witnesses:** the `take add` and `given` tests.
4. **Commands that default into a project:** capture, compare, review and renders take a take name
   and write under `takes/<take>/`; defaults applied and stated. **Needs:** 3. **Witnesses:** the
   capture-in-a-project tests. Capture's comparison with a song's composition is the songs design's
   task 3.
5. **Adoption:** `duet project adopt`, `--song` and `--apply`. **Needs:** 3, and the songs design's
   task 2 for `--song`. **Witness:** the adopt test, and the owner's `fixtures/` adopted into a
   project on the owner's machine.

Songs, their composition, parts, comps and snapshots are the
[songs design](2026-11-17-songs-and-composition-design.md#build-tasks-this-design-names)'s and the
[variants design](2026-11-17-variants-and-comps-design.md)'s tasks. The journal's kind, and the
server's lock, socket and state store under `.duet/`, are the bus design's build tasks.

## Decisions (ruled 2026-11-17)

**Ruled (owner, 2026-11-17, temper `01a1eb60-07f8-77a6-8a0d-fb3947f0ba0d`):** "yes, those stand",
read with the domain model (temper `01a1eb48-6b7a-7797-bb2b-68f335de87e9`).

1. **A project is a workspace, not one piece.** **Yes.** A project holds songs, and each song holds
   its composition ([§8](#8-a-workspace-and-composition-as-its-own-spec)).
2. **The project document is `.duet/project.duet`.** **Yes** ([§1](#1-the-layout)).
3. **A reading's notes stay "restricted" with their take.** **Dropped.** There is no sharing status
   in a project ([§6](#6-no-sharing-status-and-no-git)).
4. **`duet take add` copies by default, and `--named` leaves the file where it is.** **Yes**
   ([§4](#4-sources-named-without-copying)).
5. **Inside a project, capturing a file that is not a take is refused,** with the `take add` line.
   **Yes** ([§5](#5-how-commands-find-and-use-a-project)).
6. **Capture and the project's tempo, meter and key.** **Estimate, then compare.** Capture always
   estimates from the take and states where it disagrees with the linked song's composition. A take
   is placed on the song's bars by the song's tempo map plus an offset (the click case). A value the
   owner gives a take is remembered for that take ([§5](#5-how-commands-find-and-use-a-project)).
7. **`duet init` and git.** **Dropped.** `init` writes no `.gitignore` and prints no git line
   ([§3](#3-what-duet-init-writes), [§6](#6-no-sharing-status-and-no-git)).
8. **The takes directory is `takes/`.** **Yes.** Named for what it holds, and kept at the project
   root because a take belongs to the project and can serve several songs ([§1](#1-the-layout)).
9. **`renders/` is git-ignored.** **Becomes:** renders are rebuildable files in `renders/`, with no
   special status ([§6](#6-no-sharing-status-and-no-git)).
10. **Composition stays its own task.** **Folded into the domain model.** It becomes the
    [composition spec](2026-11-17-songs-and-composition-design.md), which the songs layout uses.
11. **The state store is never the only copy of authored work.** **Yes**
    ([§10](#10-what-the-layout-holds-for-the-bus)).
- **New: where a song's files live.** **One folder per song:** `songs/<song>/` holds `song.duet`
  (the song and its composition), `parts/`, `comps/` and `snapshots/` ([§1](#1-the-layout)).

**What fell out of the design:** `duet project check`, the `.gitignore` block, the `kept` and
`restriction` fields, the idea of "the authored half as its own repository", §6 as a licence
boundary, and the "Shared?" column of the layout table.

### Still open

Added from the bus design after the eleven were set out, and not among the owner's rulings:

12. **The journal is one file, never rotated, in v1**
    ([§10](#10-what-the-layout-holds-for-the-bus)), as the bus design asks. *Recommended: yes.*
13. **A stale `.duet/run/bus.lock` is left for the next server to overwrite,** not cleared by the
    CLI ([§10](#10-what-the-layout-holds-for-the-bus)), as the bus design asks. *Recommended: yes.*

And one tension, for the owner to rule:
- **`init` writes `.gitattributes` with `journal.duet.jsonl merge=union`** (the bus design's ruled
  decision 12), while decision 7 rules that git is not duet's concern. Kept as written until ruled
  ([§3](#3-what-duet-init-writes)).

## Not decided here

- **The journal's kind and events, the bus and the wire:**
  [the bus design](2026-11-17-bus-design.md)'s.
- **Songs, their composition, parts, snapshots and operations:**
  [the songs design](2026-11-17-songs-and-composition-design.md)'s, with its open questions.
- **A take kind of its own**, listing a take's variants (the variants design left it to "the project
  and session record"): the project document's `takes` entry carries the take's identity, and the
  readings in `takes/<take>/` are its variants. A separate kind is not needed until a take carries
  more than that.
- **Licensing for release:** later.
- **Converting a take** (a phone's M4A to FLAC) on adding: not proposed, since a byte copy is never
  in question.
- **Moving or renaming a take or a song,** and a project-wide cleanup of stale derived files: later,
  once there are projects to clean.

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
