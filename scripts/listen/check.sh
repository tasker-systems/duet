#!/usr/bin/env bash
# Witness for scripts/duet-listen.sh and the moments files it reads. For each take below, it
# captures the take with both readers, checks that every moment in its moments file is a window
# inside the take (0 <= FROM < TO <= the take's length), builds its clips, and checks that every
# clip, bridged, legato, gapped, steps and bends, for each reader, lasts exactly its window (the
# whole take for `full`). A moment outside the take, a clip at the wrong window, or a stale clip
# left in place of a new one, fails it. It also checks that each reader's steps and bends clips of
# the whole take are built from their own renderings: steps is the bridged clip with each
# excursion's frames changed, so it differs from it exactly when the reading states an excursion,
# and bends differs from it always.
#
# The takes:
#   - the synthetic corpus melody (scripts/listen/synthetic-melody.moments), at its given tempo;
#   - "Wayfaring Stranger" (crates/duet-capture/corpus/wayfaring-stranger.moments), with
#     nothing given, as the corpus test reads it. It checks no excursion count: that waits for
#     the owner's rulings by ear.
#
#   scripts/listen/check.sh
#
# Needs what duet-listen.sh needs, and a built duet. Not run in CI.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
corpus="$root/crates/duet-capture/corpus"
duet="${DUET_BIN:-$root/target/release/duet}"
[[ -x "$duet" ]] || { echo "check: no duet at $duet; build it or set DUET_BIN" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/duet-listen-check.XXXXXX")
trap 'rm -rf "$work"' EXIT
seconds() { ffprobe -v error -show_entries format=duration -of csv=p=0 "$1"; }

failed=0 summary=()

# check NAME TAKE MOMENTS [capture options...]
check() {
  local name="$1" take="$2" moments="$3"
  shift 3
  local dir="$work/$name"
  mkdir -p "$dir"
  "$duet" capture melody "$take" "$@" --reader rules -o "$dir/rules" >/dev/null 2>&1
  "$duet" capture melody "$take" "$@" --reader hmm -o "$dir/hmm" >/dev/null 2>&1

  local whole from to n=0 line expect=() bad=0
  whole=$(seconds "$take")
  expect=("full $whole")
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    read -r from to _ <<<"$line"
    n=$((n + 1))
    if ! awk -v a="$from" -v b="$to" -v w="$whole" 'BEGIN { exit !(a >= 0 && a < b && b <= w) }'; then
      echo "FAIL $name m$n: $from–$to s is not a window inside the take (0–$whole s)" >&2
      bad=1
    fi
    expect+=("m$n $(awk -v a="$from" -v b="$to" 'BEGIN { print b - a }')")
  done <"$moments"
  ((bad == 0)) || { failed=1; return; }

  if ! DUET_BIN="$duet" "$root/scripts/duet-listen.sh" "$take" --stem "$dir/rules" \
    --reading "hmm=$dir/hmm.capture.duet" --moments "$moments" --out "$dir/clips" --no-play \
    2>"$dir/build.log"; then
    echo "FAIL $name: duet-listen.sh did not build the clips:" >&2
    tr '\r' '\n' <"$dir/build.log" | grep -v '^ *$' | tail -3 >&2
    failed=1
    return
  fi

  local e key want reader suffix clip got checked=0
  for e in "${expect[@]}"; do
    read -r key want <<<"$e"
    for reader in rules hmm; do
      for suffix in "" -legato -gap -steps -bends; do
        clip="$dir/clips/$key-$reader$suffix.wav"
        if [[ ! -f "$clip" ]]; then echo "FAIL $name $key-$reader$suffix: missing" >&2; failed=1; continue; fi
        got=$(seconds "$clip")
        # Within one sample at 22.05 kHz.
        if awk -v g="$got" -v w="$want" 'BEGIN { d = g - w; exit !(d < 0.00005 && d > -0.00005) }'; then
          checked=$((checked + 1))
        else
          echo "FAIL $name $key-$reader$suffix: ${got} s, expected ${want} s" >&2
          failed=1
        fi
      done
    done
  done

  local cap count
  same() { cmp -s "$dir/clips/full-$reader.wav" "$dir/clips/full-$reader$1.wav"; }
  for reader in rules hmm; do
    cap="$dir/$reader.capture.duet"
    count=$(python3 -I -c 'import json, sys
notes = json.load(open(sys.argv[1]))["body"]["reading"]["notes"]
print(sum(len(n.get("residual", {}).get("excursions", [])) for n in notes))' "$cap")
    if ((count == 0)) && ! same -steps; then
      echo "FAIL $name full-$reader-steps: no excursion is read, and it differs from the melody" >&2; failed=1
    elif ((count > 0)) && same -steps; then
      echo "FAIL $name full-$reader-steps: $count excursions are read, and it is the melody" >&2; failed=1
    fi
    if same -bends; then echo "FAIL $name full-$reader-bends: it is the melody" >&2; failed=1; fi
  done
  summary+=("$name: $checked clips at their windows (${#expect[@]} windows)")
}

check synthetic-melody "$corpus/synthetic-melody.flac" "$here/synthetic-melody.moments" --tempo 120
check wayfaring-stranger "$corpus/wayfaring-stranger.flac" "$corpus/wayfaring-stranger.moments"

((failed == 0)) || exit 1
printf 'duet-listen: %s\n' "${summary[@]}"
echo "duet-listen: two readers each; bridged, legato, gapped, steps and bends"
