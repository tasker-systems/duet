#!/usr/bin/env bash
# Hears a take against its readings, one clip at a time: the take on the left, each reading's sine
# on the right, for the whole take and for each named moment, from a numbered menu.
#
#   scripts/duet-listen.sh TAKE --stem STEM [--reading LABEL=ANALYSIS.capture.duet]...
#                          [--moments FILE] [--out DIR] [--gain X] [--rebuild] [--no-play]
#
#   TAKE       the recording, as `duet capture melody` read it
#   --stem     the stem it was captured to (STEM.capture.duet, STEM.performance.duet.jsonl);
#              its own reading is listed under the reader that wrote it (rules or hmm)
#   --reading  another analysis of the same take, e.g. hmm=take-hmm.capture.duet (repeatable)
#   --moments  one moment per line, FROM TO LABEL in seconds; a line starting with # is a comment
#   --out      where the clips go (default STEM.listen). A clip made from a take is a copy of it:
#              keep clips of a licence-restricted take out of git
#   --gain     the sine's level over compare's (default 2.0, +6 dB), so it stands out
#   --rebuild  rebuild clips that already exist
#   --no-play  build the clips, then stop
#
# Every clip comes three ways, each as `duet capture compare` renders it: bridged, its default (the
# sine holds through a gap under 0.2 s and is silent at a longer one, so a dropped note is heard as
# silence); legato (--legato: held through every gap); and gapped (--gapped: silent wherever the
# reading has no note). In the menu, N plays clip N bridged, Nl legato, Ng gapped, q quits; while a
# clip plays, scripts/listen/play.py shows where it is in the clip and the take, and the note read.
#
# duet is DUET_BIN, else this checkout's target/release/duet, else duet on PATH. Needs ffmpeg,
# ffprobe, python3 (3.9 or later: macOS's own), and afplay (macOS) or ffplay.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

die() { echo "duet-listen: $*" >&2; exit 1; }
usage() { sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

take="" stem="" moments="" out="" gain="2.0" rebuild=0 play=1
labels=() analyses=()
while (($#)); do
  case "$1" in
    --stem) stem="$2"; shift 2 ;;
    --reading)
      [[ "$2" == *=* ]] || die "--reading takes LABEL=PATH, not $2"
      labels+=("${2%%=*}"); analyses+=("${2#*=}"); shift 2 ;;
    --moments) moments="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --gain) gain="$2"; shift 2 ;;
    --rebuild) rebuild=1; shift ;;
    --no-play) play=0; shift ;;
    -h | --help) usage; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) [[ -z "$take" ]] || die "one take only, not $take and $1"; take="$1"; shift ;;
  esac
done
[[ -n "$take" && -n "$stem" ]] || { usage >&2; exit 2; }
[[ -f "$take" ]] || die "no such take: $take"
for part in capture.duet performance.duet.jsonl; do
  [[ -f "$stem.$part" ]] || die "no $stem.$part: capture the take with duet capture melody -o $stem"
done
[[ "$gain" =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "--gain takes a number, not $gain"

duet="${DUET_BIN:-}"
if [[ -z "$duet" ]]; then
  if [[ -x "$root/target/release/duet" ]]; then duet="$root/target/release/duet"
  elif command -v duet >/dev/null; then duet=duet
  else die "no duet: build it (cargo build --release -p duet-cli) or set DUET_BIN"; fi
fi
for tool in ffmpeg ffprobe python3; do command -v "$tool" >/dev/null || die "needs $tool"; done

out="${out:-$stem.listen}"
mkdir -p "$out"
take_path="$(cd "$(dirname "$take")" && pwd)/$(basename "$take")"

# The stem's own reading comes first, named after the reader that wrote it.
own=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["body"]["reading"]["reader"]["name"])' \
  "$stem.capture.duet" 2>/dev/null) || own=stem
labels=("$own" ${labels[@]+"${labels[@]}"})
analyses=("" ${analyses[@]+"${analyses[@]}"})
for a in "${analyses[@]}"; do [[ -z "$a" || -f "$a" ]] || die "no such analysis: $a"; done

# The windows: the whole take, then each moment.
win_from=("") win_to=("") win_key=(full) win_label=("whole take")
if [[ -n "$moments" ]]; then
  [[ -f "$moments" ]] || die "no such moments file: $moments"
  n=0
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    read -r from to label <<<"$line"
    [[ "$from" =~ ^[0-9.]+$ && "$to" =~ ^[0-9.]+$ ]] || die "$moments: not FROM TO LABEL: $line"
    n=$((n + 1))
    win_from+=("$from") win_to+=("$to") win_key+=("m$n")
    win_label+=("${label:-moment $n} (${from}–${to} s)")
  done <"$moments"
fi

# Each rendering is made in a directory of its own: compare never overwrites a file, so one made
# beside an older one would fail and leave the older one in its place.
renderings=("" -legato -gap) flags=("" --legato --gapped)
items=() item_from=() item_cap=() item_label=()
for w in "${!win_key[@]}"; do
  for r in "${!labels[@]}"; do
    name="${win_key[$w]}-${labels[$r]}"
    cap="${analyses[$r]:-$stem.capture.duet}"
    items+=("$name") item_from+=("${win_from[$w]:-0}") item_cap+=("$cap")
    item_label[${#items[@]}]="${win_label[$w]} · ${labels[$r]}"
    if ((rebuild == 0)) && [[ -f "$out/$name.wav" && -f "$out/$name-legato.wav" \
      && -f "$out/$name-gap.wav" ]]; then continue; fi
    printf 'building %-40s\r' "$name" >&2
    for k in "${!renderings[@]}"; do
      work=$(mktemp -d "${TMPDIR:-/tmp}/duet-listen.XXXXXX")
      cp "$stem.capture.duet" "$stem.performance.duet.jsonl" "$work/"
      cmd=("$duet" capture compare "$take_path" -o "$work/$(basename "$stem")")
      if [[ -n "${analyses[$r]}" ]]; then
        cp "${analyses[$r]}" "$work/reading.capture.duet"
        cmd+=(--reading "$work/reading.capture.duet")
      fi
      [[ -n "${win_from[$w]}" ]] && cmd+=(--from "${win_from[$w]}" --to "${win_to[$w]}")
      [[ -n "${flags[$k]}" ]] && cmd+=("${flags[$k]}")
      "${cmd[@]}" >/dev/null || die "duet capture compare failed for $name${renderings[$k]}"
      wav=("$work"/*.compare*.wav) svg=("$work"/*.compare*.svg)
      ((${#wav[@]} == 1)) && [[ -f "${wav[0]}" ]] || die "compare wrote no one WAV for $name"
      ffmpeg -v error -y -i "${wav[0]}" -af "pan=stereo|c0=c0|c1=${gain}*c1" -c:a pcm_s16le \
        "$out/$name${renderings[$k]}.wav"
      [[ -z "${renderings[$k]}" ]] && cp "${svg[0]}" "$out/$name.svg"
      rm -rf "$work"
    done
  done
done
printf '%-60s\r' "" >&2
echo "clips in $out" >&2
((play)) || exit 0

take_secs=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$out/full-${labels[0]}.wav")
while true; do
  clear
  echo "duet-listen — $(basename "$take")"
  echo "the take LEFT · the reading's sine RIGHT (×$gain)"
  echo "  N  = bridged (compare's default: held through a gap under 0.2 s, silent at a rest)"
  echo "  Nl = legato (the sine holds through every gap)"
  echo "  Ng = gapped (the sine is silent wherever the reading has no note)"
  echo "  q  = quit · Ctrl-C stops a clip"
  echo
  for i in "${!items[@]}"; do printf '  %2d) %s\n' $((i + 1)) "${item_label[$((i + 1))]}"; done
  echo
  read -r -p "> " choice || exit 0
  [[ "$choice" == q ]] && exit 0
  suffix="" shown=""
  if [[ "$choice" == *g ]]; then suffix="-gap" shown=" [gapped]"; choice="${choice%g}"
  elif [[ "$choice" == *l ]]; then suffix="-legato" shown=" [legato]"; choice="${choice%l}"; fi
  [[ "$choice" =~ ^[0-9]+$ ]] && ((choice >= 1 && choice <= ${#items[@]})) || continue
  i=$((choice - 1))
  echo
  echo "▶ ${item_label[$choice]}${shown}"
  python3 -I "$here/listen/play.py" "$out/${items[$i]}$suffix.wav" "${item_from[$i]}" "$take_secs" \
    "${item_cap[$i]}" || true
done
