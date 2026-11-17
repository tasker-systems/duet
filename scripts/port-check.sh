#!/usr/bin/env bash
# Checks `duet play --port` against a real MIDI port: FluidSynth opens a CoreMIDI port and
# sounds what it receives through the speakers with the pinned SoundFont, while logging
# every event. Then this checks what arrived.
#
#   scripts/port-check.sh                        # play samples/sketch.notes to the end
#   scripts/port-check.sh --interrupt 4          # Ctrl-C duet after 4 seconds
#   scripts/port-check.sh --interrupt 4 part.mid # any part duet reads
#
# Played to the end, it passes when every note of the part arrived as a note-on and no key
# is left on. Interrupted, it passes when duet says it stopped, the port's last messages are
# sustain off (CC64 0) and all notes off (CC123 0) on each channel it used, and no key is
# left on. macOS only: FluidSynth's port here is CoreMIDI. Run scripts/dev-setup.sh first.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
interrupt=""
part="$root/samples/sketch.notes"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --interrupt) interrupt="${2:?--interrupt needs a number of seconds}"; shift 2 ;;
    -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) part="$1"; shift ;;
  esac
done

[[ "$(uname -s)" == Darwin ]] || { echo "error: port-check runs on macOS only (CoreMIDI)" >&2; exit 1; }
command -v fluidsynth >/dev/null || { echo "error: no fluidsynth; run scripts/dev-setup.sh" >&2; exit 1; }
data_dir="${DUET_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/duet}"
font="$data_dir/default.sf2"
[[ -f "$font" ]] || { echo "error: no SoundFont at $font; run scripts/fetch-soundfont.sh" >&2; exit 1; }
[[ -f "$part" ]] || { echo "error: no part at $part" >&2; exit 1; }

echo "building duet"
duet="$(cd "$root" && cargo build --quiet --locked -p duet-cli --bin duet --message-format=json \
  | python3 -I -c 'import json,sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable"):
        print(m["executable"])')"

work="$(mktemp -d)"
port="duet-check-$$"
log="$work/fluidsynth.log"
stop_fluidsynth() { pkill -f -- "-p $port " 2>/dev/null || true; }
trap 'stop_fluidsynth; rm -rf "$work"' EXIT

# script gives FluidSynth a terminal, and -F flushes the log as each event arrives.
script -q -F "$log" fluidsynth -a coreaudio -m coremidi -p "$port" -d -s -i "$font" \
  </dev/null >/dev/null 2>&1 &
disown  # stopped by stop_fluidsynth; bash need not report its end

# The port is there once duet lists it. Naming a port that is not there is how duet lists
# them, and that exits 1, so its output is caught before it is searched.
port_listed() {
  local ports
  ports="$("$duet" play "$part" --port "$port-is-not-a-port" 2>&1 || true)"
  grep -qx "  $port" <<<"$ports"
}
for _ in $(seq 50); do
  port_listed && break
  sleep 0.2
done
port_listed || { echo "error: FluidSynth's port $port never appeared" >&2; exit 1; }
before="$(wc -l <"$log")"

echo "playing $(basename "$part") to $port${interrupt:+, Ctrl-C after ${interrupt}s}"
status=0
if [[ -n "$interrupt" ]]; then
  "$duet" play "$part" --port "$port" 2>"$work/duet.err" &
  pid=$!
  sleep "$interrupt"
  kill -INT "$pid"
  wait "$pid" || status=$?
else
  "$duet" play "$part" --port "$port" 2>"$work/duet.err" || status=$?
fi
cat "$work/duet.err" >&2
sleep 1
stop_fluidsynth

tail -n "+$((before + 1))" "$log" | tr -d '\r' | grep '^event_pre_' >"$work/received" || true
"$duet" part show "$part" --to json >"$work/part.json"

python3 -I - "$work/received" "$work/part.json" "$status" "${interrupt:+interrupted}" \
  "$work/duet.err" <<'EOF'
import collections, json, sys

received, part, status, interrupted, err = sys.argv[1:]
events = [line.split() for line in open(received)]
kinds = collections.Counter(e[0].removeprefix("event_pre_") for e in events)
notes = sum(len(t["notes"]) for t in json.load(open(part))["tracks"])
held, channels = collections.Counter(), set()
for kind, *v in events:
    channels.add(v[0])
    if kind == "event_pre_noteon" and int(v[2]) > 0:
        held[(v[0], v[1])] += 1
    elif kind in ("event_pre_noteoff", "event_pre_noteon"):
        held[(v[0], v[1])] = 0
left = sorted(k for k, n in held.items() if n)

print("received:", ", ".join(f"{n} {k}" for k, n in sorted(kinds.items())) or "nothing")
failures = []
if left:
    failures.append(f"keys left on (channel, key): {left}")
if interrupted:
    if status != "1" or "stopped; every note on" not in open(err).read():
        failures.append(f"duet did not say it stopped (exit {status})")
    want = {(ch, cc) for ch in channels for cc in ("64", "123")}
    tail = {(v[0], v[1]) for kind, *v in events[-len(want):] if kind == "event_pre_cc" and v[2] == "0"}
    if tail != want:
        failures.append(f"the last messages are not CC64 0 and CC123 0 on channels {sorted(channels)}")
else:
    if status != "0":
        failures.append(f"duet exited {status}")
    if kinds["noteon"] != notes:
        failures.append(f"{kinds['noteon']} note-ons arrived for the part's {notes} notes")
if failures:
    print("FAIL:", *failures, sep="\n  ")
    sys.exit(1)
print("PASS:", "stopped, and every channel was reset" if interrupted
      else f"all {notes} notes arrived", "- no key left on")
EOF
