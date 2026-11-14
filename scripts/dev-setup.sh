#!/usr/bin/env bash
# Sets a developer's machine up beyond what cargo installs: the Brewfile's tools (abcmidi
# for duet-abc's oracle, FluidSynth for scripts/port-check.sh) and the pinned General MIDI
# SoundFont that `duet play` sounds songs with. Safe to run again.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v brew >/dev/null; then
  brew bundle --file="$root/Brewfile"
else
  echo "Homebrew is not installed, so the Brewfile's tools were skipped:" >&2
  grep '^brew ' "$root/Brewfile" >&2
fi

"$root/scripts/fetch-soundfont.sh"
