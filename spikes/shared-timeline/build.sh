#!/usr/bin/env bash
# Spike: build the shared timeline's data from one take, using the duet CLI as it is.
# Reads the corpus take; writes only under spikes/shared-timeline/data/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
take="$root/crates/duet-capture/corpus/wayfaring-stranger.flac"
moments="$root/crates/duet-capture/corpus/wayfaring-stranger.moments"
duet="${DUET:-$(command -v duet)}"
data="$here/data"
mkdir -p "$data"

"$duet" capture melody "$take" --reader hmm   -o "$data/ws.hmm"   2>"$data/ws.hmm.stderr"
"$duet" capture melody "$take" --reader rules -o "$data/ws.rules" 2>"$data/ws.rules.stderr"
ln -sf "$take" "$data/take.flac"
touch "$data/marks.jsonl"
python3 -I "$here/tools/build_timeline.py" "$data" "$moments"
echo "built $data/timeline.json; now: python3 -I $here/tools/serve.py"
