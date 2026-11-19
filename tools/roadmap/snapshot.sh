#!/usr/bin/env bash
# Draw duet's roadmap snapshot from the Temper records (Temper goal 01a1f193, the ROAD rules).
#
#   tools/roadmap/snapshot.sh --dry-run   # print the snapshot, write nothing
#   tools/roadmap/snapshot.sh             # create it as a new research resource
#
# Reads every goal in +making-music/duet-dev and every task in +making-music/duet-dev-workflows
# with `temper resource list --all`, then every task's edges in one `temper graph traverse
# --depth 1`, seeded with every listed task. roadmap.jq holds the rule; this script only reads and
# writes.
#
# The one write is `temper resource create --type research`, tagged roadmap-snapshot: it never
# updates, moves or deletes anything, so no older snapshot is rewritten. Any failed, unparseable,
# truncated or partial read stops it with a non-zero exit before that create, naming the read. A
# traverse with no node for a task it was seeded with is partial (roadmap.jq checks it), and there
# is no fallback to per-task reads.
#
# TEMPER names the temper to run (default: temper on PATH). It needs a login that can read both
# contexts.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temper="${TEMPER:-temper}"
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
#   read_json <out> <the read, as an error names it> <temper arguments...>
read_json() {
    local out="$1" what="$2"
    shift 2
    local cmd=("$temper" "$@" --format json)
    if ! "${cmd[@]}" > "$out.raw" 2> "$out.err"; then
        fail "could not run \`temper $what\`: $(tr '\n' ' ' < "$out.err")"
    fi
    jq -c -n 'input' < "$out.raw" > "$out" 2> /dev/null ||
        fail "\`temper $what\` answered in a shape this tool does not read: its output is not JSON"
}
list() {
    local out="$1"
    shift
    read_json "$out" "$*" "$@"
}

started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
list "$work/goals.json" resource list --type goal --context "$goals_context" --all --with open-meta
list "$work/tasks.json" resource list --type task --context "$tasks_context" --all --with open-meta

# Every task's edges in one traverse, seeded with every listed task, so each task's goal link is
# read the same way. roadmap.jq refuses a reply with no node for a seed: a task the traverse left
# out is a partial read, never a task with no goal. With no task listed there is nothing to seed.
seeds=()
while IFS= read -r id; do
    seeds+=(--from "$id")
done < <(jq -r '(.rows // [])[] | .id // empty' "$work/tasks.json")
if [ "${#seeds[@]}" -gt 0 ]; then
    read_json "$work/traverse.json" "graph traverse --from <each of $((${#seeds[@]} / 2)) tasks> --depth 1" \
        graph traverse "${seeds[@]}" --depth 1
else
    echo '{"nodes": [], "edges": []}' > "$work/traverse.json"
fi
finished="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n --slurpfile goals "$work/goals.json" --slurpfile tasks "$work/tasks.json" \
    --slurpfile traverse "$work/traverse.json" '{goals: $goals[0], tasks: $tasks[0], traverse: $traverse[0]}' \
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
