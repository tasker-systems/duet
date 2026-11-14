# CLAUDE.md

Guidance for agents working in this repository. It is a router: each topic lives in one place.

| Topic | Where |
|---|---|
| What this is, the text format, building and testing | [README.md](README.md) |
| The shared engineering baseline: every rule, why it exists, and whether a gate enforces it here | the projected skill, [`.claude/skills/tasker-systems-conventions`](.claude/skills/tasker-systems-conventions/SKILL.md) |
| This repository's pin, statuses and exceptions | [`.conventions/`](.conventions/) |
| What CI runs | [`.github/workflows/ci.yml`](.github/workflows/ci.yml) |

## Working here

- **Read the skill before claiming a rule is enforced.** Its table gives each rule's status here.
  Only `gated` means a check in CI fails on a violation. *Why:* guidance that claims a gate which
  does not exist misleads the next agent (AG-3).
- **Never edit `.conventions/baseline/` or the projected skill by hand.** Move the pin with
  `python3 -I .conventions/baseline/tools/conventions.py bump --revision <sha|main>`. *Why:* the
  conventions check compares both with the pinned baseline and fails on any edit (CONS-2, CONS-6).
- **Change a canonical config file (`clippy.toml`, `deny.toml`, `.cargo/audit.toml`, the
  `[workspace.lints]` table, `conventions-bump.yml`) in the baseline, not here.** A local
  departure is an entry in `.conventions/exceptions.toml` with its reason. *Why:* the conventions
  check fails on an unrecorded difference (CONS-3, CONS-4).
- **The format promises exact round-trips.** A change to reading or writing MIDI, text, JSON or
  TOML comes with a round-trip test in `crates/duet-midi/tests/roundtrip.rs` that fails without it,
  and a change to drum grids with one in `crates/duet-drums/tests/roundtrip.rs`.
  *Why:* a silent change to a note is the one failure a user cannot see. Enforced only by review.
- **Tests that run the `duet` binary go in `crates/duet-e2e`.** It builds the binary for each
  run, so a local run never tests a stale one. To show a new test fails on `main`, build `main`'s
  binary in a separate checkout with its own `CARGO_TARGET_DIR` and set `DUET_E2E_BIN` to it.
  *Why:* a test run against an old binary passes or fails for code that is not in the tree.

Commits are conventional (`feat:`, `fix:`, `docs:`, `chore:`).
