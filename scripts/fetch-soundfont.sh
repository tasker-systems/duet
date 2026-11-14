#!/usr/bin/env bash
# Fetches the SoundFont pinned in scripts/soundfont.env, checks its sha256, and puts it where
# `duet play` finds it: default.sf2 in duet's data directory.
#
#   scripts/fetch-soundfont.sh            # to the data directory duet uses
#   scripts/fetch-soundfont.sh PATH.sf2   # somewhere else
#
# The data directory is DUET_DATA_DIR, else $XDG_DATA_HOME/duet, else ~/.local/share/duet,
# as duet reads it on macOS and Linux. A file already there with the pinned sha256 is kept;
# one with any other content is refused, never overwritten.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
source "$here/soundfont.env"

data_dir="${DUET_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/duet}"
dest="${1:-$data_dir/default.sf2}"

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

if [[ -e "$dest" ]]; then
  if [[ "$(sha256 "$dest")" == "$SOUNDFONT_SHA256" ]]; then
    echo "$dest is already $SOUNDFONT_NAME"
    exit 0
  fi
  echo "error: $dest exists and is not $SOUNDFONT_NAME (sha256 differs); move it first" >&2
  exit 1
fi

mkdir -p "$(dirname "$dest")"
tmp="$(mktemp "$(dirname "$dest")/.soundfont.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

echo "fetching $SOUNDFONT_NAME ($((SOUNDFONT_BYTES / 1000000)) MB)"
curl --fail --location --silent --show-error --proto '=https' --output "$tmp" "$SOUNDFONT_URL"

got="$(sha256 "$tmp")"
if [[ "$got" != "$SOUNDFONT_SHA256" ]]; then
  echo "error: the download's sha256 is $got, not the pinned $SOUNDFONT_SHA256" >&2
  exit 1
fi

mv "$tmp" "$dest"
trap - EXIT
echo "$SOUNDFONT_NAME is at $dest"
