# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's private vulnerability reporting:
[open a draft advisory](https://github.com/tasker-systems/making-music/security/advisories/new)
(the repository's **Security** tab, then **Report a vulnerability**). Do not open a public issue,
pull request or discussion for a vulnerability.

Include what you found, how to reproduce it (a MIDI, text or JSON input that triggers it is
ideal), and the commit or version you tested.

## What to expect

- An acknowledgement within 7 days.
- An assessment, and either a fix plan or a reason it is not treated as a vulnerability, within
  30 days.
- Credit in the advisory when it is published, unless you ask not to be named.

## Scope

In scope: the `notefmt` library and CLI in this repository, on the `main` branch. That includes
crashes, hangs or unbounded memory use on crafted MIDI, text, JSON or TOML input, and anything
that makes a round-trip silently change a note.

Out of scope:

- Releases other than the latest commit on `main`. There are no maintained release branches.
- Vulnerabilities in dependencies with no path to exploit through `notefmt`. Report those
  upstream. `cargo audit` and `cargo deny` already run in CI.
- Files under `.conventions/baseline/`, which are projected from
  [tasker-systems/conventions](https://github.com/tasker-systems/conventions). Report those
  there.
- Findings that need an attacker who can already write to your files or run code as you.
