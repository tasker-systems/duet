---
name: tasker-systems-conventions
description: The tasker-systems engineering baseline as this repository is pinned to it — the rules,
  why each exists, and whether a gate enforces it here. Use before adding a lint suppression, an
  unsafe block, a dependency, a CI workflow, a test exception, or agent guidance; and whenever you
  are about to claim a rule is enforced.
---

# tasker-systems conventions

This skill is **projected** into this repository from the `tasker-systems/conventions` baseline at
the revision pinned in `.conventions/consumer.toml`. Do not edit it here: `conventions.py bump`
rewrites it, and CI fails if it differs from the projection (CONS-6).

The doctrine itself is in `.conventions/baseline/doctrine/`. Read the file for the area you are
touching; each rule states its *why* beside it, and its id is what you cite in a PR or an
`#[expect(reason)]`.

| Touching | Read |
|---|---|
| The rule register: every rule, and its status in each consumer | `.conventions/baseline/doctrine/rules.md` |
| The short list everything follows from | `.conventions/baseline/doctrine/tenets.md` |
| Rust: lints, formatting, API shape, unsafe, errors, panics, toolchain | `.conventions/baseline/doctrine/rust.md` |
| Python | `.conventions/baseline/doctrine/python.md` |
| TypeScript | `.conventions/baseline/doctrine/typescript.md` |
| Shell scripts and CI gate scripts | `.conventions/baseline/doctrine/shell.md` |
| Markdown | `.conventions/baseline/doctrine/docs.md` |
| HTTP API JSON and OpenAPI | `.conventions/baseline/doctrine/api.md` |
| Dependencies, advisories, licenses, Actions, tool pins | `.conventions/baseline/doctrine/supply-chain.md` |
| Credentials, redaction, SQL, auth | `.conventions/baseline/doctrine/security.md` |
| Channels, spawned work, locks, database coordination | `.conventions/baseline/doctrine/concurrency.md` |
| Tests and CI shape | `.conventions/baseline/doctrine/testing.md` |
| Writing a tripwire | `.conventions/baseline/doctrine/tripwires.md` |
| Agent guidance | `.conventions/baseline/doctrine/agent-guidance.md` |
| This repository's pin, exceptions and statuses | `.conventions/baseline/doctrine/consuming.md` |

## Before you claim a rule is enforced

Check the status below. `gated: <gate>` means a named check fails on a violation **here**. Anything
else — `partial`, `practiced`, `convention`, `uncovered`, `unexamined` — means nothing here would
catch a violation, and saying otherwise is the defect tenet 1 names. An unlisted rule is
unexamined in this repository, not covered.

Local departures from the baseline, each with its reason, are in `.conventions/exceptions.toml`.

<!-- consumer-status:begin -->

Statuses for **duet**:

| Rule | Status here |
|---|---|
| AG-1 | practiced: CLAUDE.md is a short router |
| AG-2 | convention: judged in review |
| AG-3 | convention: judged in review |
| AG-4 | practiced: CLAUDE.md lists no facts that mirror the code |
| AG-5 | gated: .github/workflows/ci.yml#gate |
| AG-6 | n/a: no session hooks |
| API-JSON-1 | n/a: no HTTP API |
| API-JSON-2 | n/a: no HTTP API |
| API-JSON-3 | n/a: no HTTP API |
| API-JSON-4 | n/a: no HTTP API |
| API-JSON-5 | n/a: no HTTP API |
| CI-1 | gated: .github/workflows/ci.yml#gate |
| CI-2 | gated: .github/workflows/ci.yml#ci-success |
| CI-3 | gated: .github/workflows/ci.yml#gate |
| CI-4 | uncovered: no pre-commit hook |
| CI-5 | practiced: ci.yml triggers on merge_group; no merge queue is configured |
| CI-6 | practiced: every job sets timeout-minutes with room for a cold cache |
| CONC-BOUND-1 | gated: .github/workflows/ci.yml#check |
| CONC-BOUND-2 | n/a: no channels or queues |
| CONC-BOUND-3 | n/a: no channels or queues |
| CONC-BOUND-4 | n/a: no channels or queues |
| CONC-DB-1 | n/a: no database |
| CONC-DB-2 | n/a: no database |
| CONC-DB-3 | n/a: no database |
| CONC-FAIL-1 | n/a: no spawned work |
| CONC-FAIL-2 | n/a: no spawned work |
| CONC-FAIL-3 | n/a: no locks |
| CONS-1 | gated: .github/workflows/ci.yml#gate |
| CONS-2 | gated: .github/workflows/ci.yml#gate |
| CONS-3 | gated: .github/workflows/ci.yml#gate |
| CONS-4 | gated: .github/workflows/ci.yml#gate |
| CONS-5 | gated: .github/workflows/ci.yml#gate |
| CONS-6 | gated: .github/workflows/ci.yml#gate |
| CONS-7 | gated: .github/workflows/ci.yml#gate |
| CONS-8 | gated: .github/workflows/ci.yml#gate |
| CONS-9 | gated: .github/workflows/conventions-bump.yml#bump |
| DOC-1 | uncovered: no Markdown linter |
| DOC-2 | uncovered: no Markdown linter |
| DOC-3 | uncovered: no Markdown linter; README.md fits 100 columns today |
| DOC-4 | uncovered: no link checker |
| PY-DEF-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-ERR-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-ERR-2 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-GEN-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-LINT-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-LINT-2 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-LINT-3 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-LOG-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-TYPE-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-TYPE-2 | n/a: no Python in this repository (the projected tool is the baseline's) |
| PY-VER-1 | n/a: no Python in this repository (the projected tool is the baseline's) |
| RUST-API-1 | convention: duet-core, duet-midi, duet-drums, duet-abc, duet-play and duet-capture are library crates, duet-cli is the application, and duet-e2e is an unpublished test crate; review applies the API rules to the libraries' public API |
| RUST-API-2 | convention: the libraries are published to crates.io; the open sets are #[non_exhaustive] (the six Error enums, Control, Song, Track, Note, DrumGrid, LaneDef, duet-capture's Event, HitLane, LaneMode, Timing, BeatTempo, Kind, Captured, Analysis, QuantizeOffset, Quantized, Merge and KeyEstimate, and duet-abc's Refusal, Notice, Reading, ReadOptions, WriteOptions, Dynamics) and the closed ones are exhaustive (duet-capture's Chunk and Mono, deliberately, so other crates can build them for their own sources and stages; duet-capture's Origin, Given and LeadIn, deliberately, since a value is given or estimated, the command line builds a Given, and a lead-in is none, a pickup or rest bars; Position, TimeSig, Key, Mode, the three *Change events, Lane, Bar, Levels, Span, and duet-play's Event); Book, Tune, and duet-play's Message, Schedule and Renderer have private fields; a new public type is judged in review |
| RUST-API-3 | partial: should_implement_trait and derivable_impls gate the detectable part (.github/workflows/ci.yml#check); the rest is review |
| RUST-API-4 | partial: ptr_arg and borrowed_box gate the detectable part (.github/workflows/ci.yml#check); the rest is review |
| RUST-API-5 | partial: new_without_default and wrong_self_convention gate the detectable part (.github/workflows/ci.yml#check); the rest is review |
| RUST-API-6 | uncovered: no #[must_use] audit, and must_use_candidate is not on |
| RUST-DOC-1 | uncovered: missing_docs is not on; the public API is documented by habit |
| RUST-ERR-1 | practiced: the Error types of duet-core, duet-midi, duet-drums, duet-abc, duet-play and duet-capture derive thiserror::Error behind Result aliases, and duet-cli returns anyhow::Result with context on file errors; nothing checks that a new error type does the same |
| RUST-ERR-2 | convention: judged in review |
| RUST-ERR-3 | convention: judged in review; the 13 expect calls in library code each state the invariant they rely on: five in duet-core's time map, two in duet-play's tempo segments, two in duet-drums (the built-in gm map, a grid's lanes) and four in duet-capture (the pitch smoothing, and three in timing); duet-e2e's harness is test code |
| RUST-ERR-4 | practiced: no unwrap outside tests; unwrap_used is held off in the table, so nothing would catch a new one |
| RUST-FMT-1 | gated: .github/workflows/ci.yml#gate |
| RUST-INIT-1 | convention: no ctor-style crates; judged in review |
| RUST-LINT-1 | partial: the workspace table is pinned to the projection by the conventions check (.github/workflows/ci.yml#gate) and applied by clippy (.github/workflows/ci.yml#check); nothing checks that a new crate sets [lints] workspace = true |
| RUST-LINT-2 | gated: .github/workflows/ci.yml#check |
| RUST-LINT-3 | gated: .github/workflows/ci.yml#check |
| RUST-LINT-4 | practiced: the table was adopted after an audit that found no findings to fix |
| RUST-LINT-5 | convention: the held-off groups stay off until a ruling opens one lint at a time |
| RUST-MACRO-1 | convention: judged in review; the one macro_rules (string_serde in time.rs) stamps out four identical serde impls |
| RUST-ORG-1 | uncovered: wildcard_imports is held off in the table; today only test modules glob-import super |
| RUST-ORG-2 | uncovered: judged in review only |
| RUST-PANIC-1 | n/a: no catch_unwind |
| RUST-READ-1 | convention: judged in review |
| RUST-SQL-1 | n/a: no database |
| RUST-TC-1 | gated: .github/workflows/ci.yml#msrv |
| RUST-TC-2 | practiced: edition is set once in [workspace.package]; nothing checks a crate does not override it |
| RUST-TC-3 | gated: .github/workflows/ci.yml#check |
| RUST-TYPE-1 | gated: .github/workflows/ci.yml#check |
| RUST-TYPE-2 | convention: judged in review |
| RUST-UNSAFE-1 | partial: every crate root (duet-core, duet-midi, duet-drums, duet-abc, duet-play, duet-capture, duet-cli, duet-e2e) declares #![forbid(unsafe_code)] and the table denies unsafe_code; nothing checks that a new crate root declares the forbid |
| RUST-UNSAFE-2 | n/a: no unsafe code and no registered exceptions |
| RUST-UNSAFE-3 | n/a: no FFI |
| SC-ADV-1 | gated: .github/workflows/ci.yml#gate |
| SC-ADV-2 | convention: .cargo/audit.toml ignores nothing; an entry is reviewed by hand |
| SC-DENY-1 | gated: .github/workflows/ci.yml#gate |
| SC-DENY-2 | gated: .github/workflows/ci.yml#gate |
| SC-DENY-3 | gated: .github/workflows/ci.yml#gate |
| SC-GHA-1 | practiced: every uses: is a commit SHA with its tag; nothing refuses a new tag or branch ref |
| SC-GHA-2 | practiced: every workflow sets top-level permissions: contents: read, and codeql.yml#analyze adds security-events: write with its reason; nothing refuses a workflow without it |
| SC-GHA-3 | n/a: no release chain |
| SC-IN-1 | practiced: install-action, its tool versions, gitleaks (module hash asserted) and the Go toolchain that builds it are pinned, each at least 7 days old when pinned; nothing checks it |
| SC-IN-2 | practiced: Dependabot covers cargo and github-actions with a 7-day cooldown; nothing checks a new ecosystem is added |
| SC-IN-3 | gated: .github/workflows/ci.yml#gate |
| SC-SEC-1 | partial: gitleaks scans the full history on every pull request, docs-only changes included, with the reviewed allowlist in .gitleaks.toml (.github/workflows/ci.yml#secrets, required through CI Success); CodeQL runs per pull request and weekly (.github/workflows/codeql.yml#analyze) and reports without blocking, by the owner's decision while the repo is early; the pre-commit half is out of scope, as there is no pre-commit hook (CI-4) |
| SEC-AUTH-1 | n/a: no authentication |
| SEC-AUTH-2 | n/a: no authentication |
| SEC-AUTH-3 | n/a: no authentication |
| SEC-CRED-1 | n/a: no credentials |
| SEC-CRED-2 | n/a: no credentials |
| SEC-CRED-3 | n/a: no credentials |
| SEC-CRED-4 | n/a: no credentials |
| SEC-RED-1 | n/a: no secrets or personal data to redact |
| SEC-RED-2 | n/a: no secrets or personal data to redact |
| SEC-RPT-1 | partial: SECURITY.md routes reports to GitHub private vulnerability reporting, with response times and an out-of-scope list; private reporting is enabled in repo settings; it names no email address, and nothing checks the file |
| SEC-SQL-1 | n/a: no database |
| SEC-SQL-2 | n/a: no database |
| SEC-TEST-1 | n/a: no test doors; nothing ships a server |
| SH-1 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-2 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-3 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-4 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-5 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-6 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| SH-7 | n/a: no shell scripts; workflow run: blocks are a few lines each |
| TEST-1 | uncovered: no #[ignore] tests today, and nothing would catch an unregistered one |
| TEST-2 | convention: judged in review |
| TEST-3 | partial: CI runs every tier but one (.github/workflows/ci.yml#check). duet-abc's oracle feature runs abc2midi, which by the owner's decision runs only on a developer's machine: CI compiles and lints it (clippy --all-features) but never runs it, and gates drift against its committed recordings instead |
| TEST-4 | n/a: no test retries |
| TEST-5 | uncovered: no coverage floor |
| TS-CFG-1 | n/a: no TypeScript |
| TS-CFG-2 | n/a: no TypeScript |
| TS-DOC-1 | n/a: no TypeScript |
| TS-ERR-1 | n/a: no TypeScript |
| TS-GEN-1 | n/a: no TypeScript |
| TS-MOD-1 | n/a: no TypeScript |
| TS-NAME-1 | n/a: no TypeScript |
| TS-SUP-1 | n/a: no TypeScript |
| TS-TOOL-1 | n/a: no TypeScript |
| TS-TOOL-2 | n/a: no TypeScript |
| TS-TOOL-3 | n/a: no TypeScript |
| TS-TYPE-1 | n/a: no TypeScript |
| TS-TYPE-2 | n/a: no TypeScript |
| TW-1 | n/a: this repository has no tripwires of its own |
| TW-2 | n/a: this repository has no tripwires of its own |
| TW-3 | n/a: this repository has no tripwires of its own |
| TW-4 | n/a: this repository has no tripwires of its own |
| TW-5 | n/a: this repository has no tripwires of its own |
| TW-6 | n/a: this repository has no tripwires of its own |
| TW-7 | n/a: this repository has no tripwires of its own |

<!-- consumer-status:end -->
