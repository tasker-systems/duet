#!/usr/bin/env bash
# Draw duet's roadmap snapshot from the Temper records (Temper goal 01a1f193, the ROAD rules).
#
#   tools/roadmap/snapshot.sh --dry-run   # print the snapshot, write nothing
#   tools/roadmap/snapshot.sh             # create it as a new research resource
#
# Reads every goal in +making-music/duet-dev and every task in +making-music/duet-dev-workflows
# with `temper resource list --all`, and each task's edges with `temper resource show --with edges`
# (ROADMAP_JOBS at a time, default 8). roadmap.jq holds the rule; this script only reads and writes.
#
# The one write is `temper resource create --type research`, tagged roadmap-snapshot: it never
# updates, moves or deletes anything, so no older snapshot is rewritten. Any failed, unparseable or
# truncated read stops it with a non-zero exit before that create, naming the read.
#
# TEMPER names the temper to run (default: temper on PATH). It needs a login that can read both
# contexts.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temper="${TEMPER:-temper}"
jobs="${ROADMAP_JOBS:-8}"
goals_context="+making-music/duet-dev"
tasks_context="+making-music/duet-dev-workflows"

dry_run=false
case "${1:-}" in
    --dry-run) dry_run=true ;;
    "") ;;
    *)
        echo "usage: $0 [--dry-run]" >&2
        exit 2
        ;;
esac

fail() {
    echo "roadmap-snapshot: $*" >&2
    echo "roadmap-snapshot: no snapshot was drawn, and nothing was written" >&2
    exit 1
}

command -v jq > /dev/null 2>&1 || fail "jq is not on PATH"
command -v "$temper" > /dev/null 2>&1 || fail "could not run \`$temper\`: not found"

work="$(mktemp -d "${TMPDIR:-/tmp}/roadmap-snapshot.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# Run one temper read and keep the first JSON value it prints: temper may print more after it.
read_json() {
    local out="$1"
    shift
    local cmd=("$temper" "$@" --format json)
    if ! "${cmd[@]}" > "$out.raw" 2> "$out.err"; then
        fail "could not run \`temper $*\`: $(tr '\n' ' ' < "$out.err")"
    fi
    jq -c -n 'input' < "$out.raw" > "$out" 2> /dev/null ||
        fail "\`temper $*\` answered in a shape this tool does not read: its output is not JSON"
}

started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
read_json "$work/goals.json" resource list --type goal --context "$goals_context" --all --with open-meta
read_json "$work/tasks.json" resource list --type task --context "$tasks_context" --all --with open-meta

# Each task's edges, read the same way for every task: its goal is the peer of its outgoing
# `advances` edge, and a task without one is counted, never dropped (the trap in the task's body).
mkdir "$work/shows"
jq -r '(.rows // [])[] | .id // empty' "$work/tasks.json" > "$work/ids"
# shellcheck disable=SC2016 # the single-quoted script is bash's, run by xargs with $0 to $2 set
if ! xargs -P "$jobs" -I '{}' bash -c '
    set -euo pipefail
    if ! "$0" resource show "$1" --without body --without open-meta --with edges --format json \
        > "$2/$1.raw" 2> "$2/$1.err"; then
        exit 1
    fi
    jq -c -n input < "$2/$1.raw" > "$2/$1.json" 2> /dev/null
' "$temper" '{}' "$work/shows" < "$work/ids"; then
    while IFS= read -r id; do
        if [ ! -s "$work/shows/$id.json" ]; then
            fail "could not read the edges of task $id (\`temper resource show $id --with edges\`): $(tr '\n' ' ' < "$work/shows/$id.err")"
        fi
    done < "$work/ids"
    fail "an edge read failed"
fi
# Keyed by the id each read was asked for, so an answer for another id is caught by roadmap.jq.
if [ -s "$work/ids" ]; then
    jq -n 'reduce inputs as $show ({}; . + {(input_filename | split("/") | last | rtrimstr(".json")): $show})' \
        "$work/shows"/*.json > "$work/shows.json"
else
    echo '{}' > "$work/shows.json"
fi
finished="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n --slurpfile goals "$work/goals.json" --slurpfile tasks "$work/tasks.json" \
    --slurpfile shows "$work/shows.json" '{goals: $goals[0], tasks: $tasks[0], shows: $shows[0]}' \
    > "$work/records.json"
if ! jq -j -L "$here" --arg started "$started" --arg finished "$finished" \
    --rawfile recipe "$here/recipe.md" -f "$here/snapshot.jq" "$work/records.json" \
    > "$work/body.md" 2> "$work/body.err"; then
    fail "$(sed 's/^jq: error (at [^)]*): //' "$work/body.err" | tr '\n' ' ')"
fi

if [ "$dry_run" = true ]; then
    cat "$work/body.md"
    exit 0
fi

title="duet roadmap snapshot, ${started%%T*}"
meta="$(jq -c -n --arg date "${started%%T*}" --arg from "$started" --arg to "$finished" \
    '{tags: ["roadmap-snapshot"], date: $date, records_read_from: $from, records_read_to: $to}')"
create=("$temper" resource create --type research --context "$goals_context" --title "$title"
    --open-meta "$meta" --body - --format json)
uncertain() {
    echo "roadmap-snapshot: the create did not report a new resource: $*" >&2
    echo "roadmap-snapshot: it may still have landed, so look for a new roadmap-snapshot in $goals_context before drawing again" >&2
    exit 1
}
if ! "${create[@]}" < "$work/body.md" > "$work/created.raw" 2> "$work/created.err"; then
    uncertain "$(tr '\n' ' ' < "$work/created.err")"
fi
id="$(jq -r -n 'input | .id // empty' < "$work/created.raw" 2> /dev/null || true)"
[ -n "$id" ] || uncertain "its answer names no id"
echo "created $id: $title"
