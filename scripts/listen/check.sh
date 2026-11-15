#!/usr/bin/env bash
# Witness for scripts/duet-listen.sh: captures the synthetic corpus melody with both readers,
# builds its clips from scripts/listen/synthetic-melody.moments, and checks that every clip,
# legato and gapped, for each reader, lasts exactly its window (the whole take for `full`).
# A clip at the wrong window, or a stale clip left in place of a new one, fails it.
#
#   scripts/listen/check.sh
#
# Needs what duet-listen.sh needs, and a built duet. Not run in CI.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
take="$root/crates/duet-capture/corpus/synthetic-melody.flac"
moments="$here/synthetic-melody.moments"
duet="${DUET_BIN:-$root/target/release/duet}"
[[ -x "$duet" ]] || { echo "check: no duet at $duet; build it or set DUET_BIN" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/duet-listen-check.XXXXXX")
trap 'rm -rf "$work"' EXIT
"$duet" capture melody "$take" --tempo 120 --reader rules -o "$work/rules" >/dev/null 2>&1
"$duet" capture melody "$take" --tempo 120 --reader hmm -o "$work/hmm" >/dev/null 2>&1
if ! DUET_BIN="$duet" "$root/scripts/duet-listen.sh" "$take" --stem "$work/rules" \
  --reading "hmm=$work/hmm.capture.duet" --moments "$moments" --out "$work/clips" --no-play \
  2>"$work/build.log"; then
  echo "FAIL duet-listen.sh did not build the clips:" >&2
  tr '\r' '\n' <"$work/build.log" | grep -v '^ *$' | tail -3 >&2
  exit 1
fi

seconds() { ffprobe -v error -show_entries format=duration -of csv=p=0 "$1"; }
whole=$(seconds "$take")
expect=("full $whole")
n=0
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
  read -r from to _ <<<"$line"
  n=$((n + 1))
  expect+=("m$n $(awk -v a="$from" -v b="$to" 'BEGIN { print b - a }')")
done <"$moments"

failed=0 checked=0
for e in "${expect[@]}"; do
  read -r key want <<<"$e"
  for reader in rules hmm; do
    for suffix in "" -gap; do
      clip="$work/clips/$key-$reader$suffix.wav"
      if [[ ! -f "$clip" ]]; then echo "FAIL $key-$reader$suffix: missing" >&2; failed=1; continue; fi
      got=$(seconds "$clip")
      # Within one sample at 22.05 kHz.
      if awk -v g="$got" -v w="$want" 'BEGIN { d = g - w; exit !(d < 0.00005 && d > -0.00005) }'; then
        checked=$((checked + 1))
      else
        echo "FAIL $key-$reader$suffix: ${got} s, expected ${want} s" >&2
        failed=1
      fi
    done
  done
done
((failed == 0)) || exit 1
echo "duet-listen: $checked clips at their windows (${#expect[@]} windows, two readers, legato and gapped)"
