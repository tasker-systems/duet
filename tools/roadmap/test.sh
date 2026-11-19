#!/usr/bin/env bash
# The roadmap snapshot's witnesses (Temper tasks 01a1f1ac and 01a1f236), over fixed records with
# no network:
#   1. the fixture draws the expected lanes and counts (fixture/expected.md, checked by hand), and
#      an `advances` edge into a task, from a session or a goal, is no task's goal link;
#   2. two draws over the same records differ only in the stated date and read times;
#   3. a failed, unparseable, truncated or partial read exits non-zero and writes nothing, and a
#      traverse with no node for a task it was seeded with, or holding other than its node's
#      `degree` of edges at it, is refused, naming the task;
#   4. the only write is one create, every task's edges come from one `graph traverse` and no
#      `resource show`, and a dry run writes nothing;
#   5. the workstreams are CLAUDE.md's vocabulary, in its order.
# snapshot.sh runs against fixture/temper, a fake that answers from the fixture, logs each call,
# and refuses `resource show`.
# CI runs this in .github/workflows/ci.yml#gate.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
records="$here/fixture/records.json"
expected="$here/fixture/expected.md"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/roadmap-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

failures=0
ok() { echo "ok   $1"; }
bad() {
    echo "FAIL $1" >&2
    failures=$((failures + 1))
}

# Draw the body from a records file: draw <started> <finished> <records> > body
draw() {
    jq -j -L "$here" --arg started "$1" --arg finished "$2" --rawfile recipe "$here/recipe.md" \
        -f "$here/snapshot.jq" "$3"
}
# The body without its three dated lines: the only lines two draws may differ in.
without_date() {
    grep -v -e '^# duet roadmap snapshot, ' -e '^- \*\*Date described:\*\*' -e '^- \*\*Records read:\*\*' "$1"
}
# A directory the fake temper serves, from the fixture after a jq edit: fake <name> <jq filter>
fake() {
    local dir="$scratch/$1"
    mkdir -p "$dir"
    jq "$2" "$records" > "$dir/records.json"
    jq '.goals' "$dir/records.json" > "$dir/goals.json"
    jq '.tasks' "$dir/records.json" > "$dir/tasks.json"
    jq '.traverse' "$dir/records.json" > "$dir/graph.json"
    echo "$dir"
}
# Run snapshot.sh against a fake: snapshot <dir> [--dry-run]; stdout, stderr and status land in <dir>.
snapshot() {
    local dir="$1"
    shift
    local status=0
    TEMPER="$here/fixture/temper" FAKE_TEMPER_DIR="$dir" bash "$here/snapshot.sh" "$@" \
        > "$dir/stdout" 2> "$dir/stderr" || status=$?
    echo "$status" > "$dir/status"
}
# How many creates the fake was asked for; 0 when it was never run.
creates() {
    if [ -e "$1/calls.log" ]; then
        grep -c '^resource create' "$1/calls.log" || true
    else
        echo 0
    fi
}
# Why a run's call log is not one depth-1 traverse seeded with every fixture task and no per-task
# read; empty when it is: traversed_once <dir>
traversed_once() {
    local log="$1/calls.log" traverses want got
    traverses="$(grep -c '^graph traverse ' "$log" || true)"
    want="$(jq -r '.tasks.rows[].id' "$records" | sort)"
    got="$(grep '^graph traverse ' "$log" | head -n 1 | grep -o -e '--from [^ ]*' | cut -d ' ' -f 2 | sort)"
    if grep -q '^resource show ' "$log"; then
        echo "it made a per-task \`resource show\`: $(grep -c '^resource show ' "$log")"
    elif [ "$traverses" != 1 ]; then
        echo "expected one \`graph traverse\`, saw $traverses"
    elif ! grep -q '^graph traverse .*--depth 1\( \|$\)' "$log"; then
        echo "the traverse is not at --depth 1: $(grep '^graph traverse ' "$log")"
    elif [ "$want" != "$got" ]; then
        echo "the traverse was not seeded with exactly the fixture's $(echo "$want" | wc -l | tr -d ' ') tasks"
    fi
}

# 1. The fixture draws the expected lanes and counts.
if draw 2026-11-19T03:00:00Z 2026-11-19T03:00:01Z "$records" > "$scratch/body.md" &&
    diff -u "$expected" "$scratch/body.md"; then
    ok "fixed records draw fixture/expected.md"
else
    bad "fixed records draw fixture/expected.md"
fi

# 1b. Only an edge from a task is its goal link. The fixture holds `advances` edges into tasks,
# from sessions and from a goal; give every task more of each, and the draw does not move.
into_tasks="$(jq '[.traverse as $g | ($g.nodes | map({(.id): .doc_type}) | add) as $type
    | $g.edges[] | select(.label == "advances" and $type[.target] == "task" and $type[.source] == "session")] | length' "$records")"
jq '[.tasks.rows[].id] as $tasks
    | .traverse.nodes |= map(if .doc_type == "task" then .degree += 2 else . end)
    | .traverse.nodes += [{id: "60000000-0000-7000-8000-000000000099", title: "a busy session", doc_type: "session", degree: ($tasks | length)}]
    | .traverse.edges += [$tasks | to_entries[] | {id: "71000000-0000-7000-8000-\(1000000000000 + .key | tostring | .[1:])",
        source: "60000000-0000-7000-8000-000000000099", target: .value, edge_kind: "leads_to", polarity: "forward",
        label: "advances", weight: 1.0}]
    | .traverse.edges += [$tasks | to_entries[] | {id: "72000000-0000-7000-8000-\(1000000000000 + .key | tostring | .[1:])",
        source: "10000000-0000-7000-8000-000000000002", target: .value, edge_kind: "leads_to", polarity: "forward",
        label: "advances", weight: 1.0}]' "$records" > "$scratch/into.json"
if [ "$into_tasks" -lt 2 ]; then
    bad "an advances edge into a task is no task's goal link: the fixture holds $into_tasks from a session"
elif draw 2026-11-19T03:00:00Z 2026-11-19T03:00:01Z "$scratch/into.json" > "$scratch/into.md" &&
    diff -u "$expected" "$scratch/into.md"; then
    ok "an advances edge into a task, from a session or a goal, is no task's goal link"
else
    bad "an advances edge into a task, from a session or a goal, is no task's goal link"
fi

# 2. The same records, served in the opposite order and read on another day.
jq '.goals.rows |= reverse | .tasks.rows |= reverse | .traverse.nodes |= reverse | .traverse.edges |= reverse' \
    "$records" > "$scratch/reversed.json"
if draw 2026-12-25T18:30:00Z 2026-12-25T18:32:10Z "$scratch/reversed.json" > "$scratch/other.md" &&
    [ -s "$scratch/body.md" ] && ! cmp -s "$scratch/body.md" "$scratch/other.md" &&
    diff -u <(without_date "$scratch/body.md") <(without_date "$scratch/other.md"); then
    ok "two draws differ only in the stated date and read times"
else
    bad "two draws differ only in the stated date and read times"
fi

# 3a. The rule refuses a partial or out-of-shape read, and says which.
refuses() {
    local name="$1" edit="$2" names="$3"
    jq "$edit" "$records" > "$scratch/edited.json"
    if draw 2026-11-19T03:00:00Z 2026-11-19T03:00:01Z "$scratch/edited.json" > "$scratch/out" 2> "$scratch/err"; then
        bad "$name: drew a snapshot"
    elif [ -s "$scratch/out" ]; then
        bad "$name: printed a partial snapshot"
    elif ! grep -q -- "$names" "$scratch/err"; then
        bad "$name: the error does not name '$names': $(cat "$scratch/err")"
    else
        ok "$name: refused, naming '$names'"
    fi
}
refuses "a truncated goal listing" '.goals.truncated = true' "goal listing was truncated"
refuses "a short task listing" '.tasks.rows |= .[1:] | .tasks.returned = 18' "returned 18 of 19 rows"
refuses "a task listing whose returned disagrees" '.tasks.returned = 20' "says it returned 20 rows but holds 19"
refuses "a traverse with no node for a seeded task" \
    '.traverse.nodes |= map(select(.id != "20000000-0000-7000-8000-000000000009"))' \
    "no node for task 20000000-0000-7000-8000-000000000009"
refuses "a traverse missing one edge at a seeded task" \
    '(.traverse.edges | map(.target == "20000000-0000-7000-8000-000000000005") | index(true)) as $i | del(.traverse.edges[$i])' \
    "the traverse holds [0-9]* of the [0-9]* edges its node gives task 20000000-0000-7000-8000-000000000005"
refuses "a traverse repeating an edge at a seeded task" \
    '.traverse.edges += [.traverse.edges[] | select(.source == "20000000-0000-7000-8000-000000000003")][:1]' \
    "edges its node gives task 20000000-0000-7000-8000-000000000003"
refuses "a traverse with no degree for a seeded task" \
    '.traverse.nodes |= map(if .id == "20000000-0000-7000-8000-000000000011" then del(.degree) else . end)' \
    "no numeric \`degree\`"
refuses "a traverse with no edge list" 'del(.traverse.edges)' "lacks a \`nodes\` or \`edges\` list"
refuses "a traverse edge with no source" '.traverse.edges[0] |= del(.source)' "no string \`source\`"
refuses "an unknown goal status" '.goals.rows[0].managed_meta["temper-status"] = "dormant"' "none of"
refuses "an unknown task stage" '.tasks.rows[0].managed_meta["temper-stage"] = "parked"' "none of"

# 3b. snapshot.sh, against the fake, writes nothing when a read fails.
writes_nothing() {
    local name="$1" dir="$2" names="$3"
    if [ "$(cat "$dir/status")" = 0 ]; then
        bad "$name: exited 0"
    elif [ "$(creates "$dir")" != 0 ] || [ -e "$dir/created.md" ]; then
        bad "$name: wrote a snapshot"
    elif [ -s "$dir/stdout" ]; then
        bad "$name: printed a snapshot"
    elif ! grep -q -- "$names" "$dir/stderr" || ! grep -q "nothing was written" "$dir/stderr"; then
        bad "$name: stderr does not name '$names' and say nothing was written: $(cat "$dir/stderr")"
    else
        ok "$name: exits non-zero and writes nothing"
    fi
}
dir="$(fake truncated '.tasks.truncated = true')"
snapshot "$dir"
writes_nothing "snapshot.sh, a truncated task listing" "$dir" "task listing was truncated"

dir="$(fake failing-traverse '.')"
touch "$dir/traverse.fail"
snapshot "$dir"
writes_nothing "snapshot.sh, a failing traverse" "$dir" "could not run \`temper graph traverse"

dir="$(fake garbage-traverse '.')"
touch "$dir/traverse.garbage"
snapshot "$dir"
writes_nothing "snapshot.sh, a traverse that is not JSON" "$dir" "graph traverse.*not JSON"

dir="$(fake partial-traverse '.traverse.nodes |= map(select(.id != "20000000-0000-7000-8000-000000000007"))')"
snapshot "$dir"
writes_nothing "snapshot.sh, a traverse with no node for a seeded task" "$dir" \
    "no node for task 20000000-0000-7000-8000-000000000007"

dir="$(fake short-traverse '(.traverse.edges | map(.source == "20000000-0000-7000-8000-000000000012" or .target == "20000000-0000-7000-8000-000000000012") | index(true)) as $i | del(.traverse.edges[$i])')"
snapshot "$dir"
writes_nothing "snapshot.sh, a traverse missing one edge at a seeded task" "$dir" \
    "edges its node gives task 20000000-0000-7000-8000-000000000012"

dir="$(fake garbage '.')"
echo "not json at all" > "$dir/goals.json"
snapshot "$dir"
writes_nothing "snapshot.sh, a listing that is not JSON" "$dir" "not JSON"

dir="$scratch/no-temper"
mkdir -p "$dir"
status=0
TEMPER="$dir/temper" bash "$here/snapshot.sh" --dry-run > "$dir/stdout" 2> "$dir/stderr" || status=$?
echo "$status" > "$dir/status"
writes_nothing "snapshot.sh, no temper to run" "$dir" "could not run"

# 4. The only write is one create; a dry run writes nothing.
dir="$(fake write '.')"
snapshot "$dir"
others="$(grep -v -e '^resource list ' -e '^graph traverse ' -e '^resource create ' "$dir/calls.log" || true)"
if [ "$(cat "$dir/status")" != 0 ]; then
    bad "a write: exited $(cat "$dir/status"): $(cat "$dir/stderr")"
elif [ "$(creates "$dir")" != 1 ] || [ -n "$others" ]; then
    bad "a write: expected exactly one create and only reads: $(cat "$dir/calls.log")"
elif ! grep -q '^resource create --type research --context +making-music/duet-dev --title duet roadmap snapshot, .* --open-meta {"tags":\["roadmap-snapshot"\]' "$dir/calls.log"; then
    bad "a write: the create is not a tagged research resource in duet-dev: $(grep '^resource create' "$dir/calls.log")"
elif ! grep -q '^created 50000000-0000-7000-8000-000000000001: duet roadmap snapshot, ' "$dir/stdout"; then
    bad "a write: stdout does not name the created resource: $(cat "$dir/stdout")"
elif [ -n "$(traversed_once "$dir")" ]; then
    bad "a write: $(traversed_once "$dir")"
else
    ok "a write makes one create, of a tagged research resource, and otherwise only reads"
    ok "every task's goal link comes from one traverse, and no resource show"
fi

dry="$(fake dry-run '.')"
snapshot "$dry" --dry-run
if [ "$(cat "$dry/status")" != 0 ] || [ "$(creates "$dry")" != 0 ]; then
    bad "a dry run: exited $(cat "$dry/status") with $(creates "$dry") creates"
elif [ -n "$(traversed_once "$dry")" ]; then
    bad "a dry run: $(traversed_once "$dry")"
elif ! diff -u <(without_date "$dry/stdout") <(without_date "$dir/created.md"); then
    bad "a dry run prints what a write creates"
elif ! diff -u <(without_date "$dry/stdout") <(without_date "$expected"); then
    bad "snapshot.sh draws the fixture as roadmap.jq does"
else
    ok "a dry run prints what a write creates, and writes nothing"
fi

# No temper command in snapshot.sh writes anything but the create.
writes="$(grep -v '^[[:space:]]*#' "$here/snapshot.sh" |
    grep -n -E 'resource (update|delete|meta|annotate|facet|reblock)|edge (assert|fold|retype|reweight|facet)|data-artifact|blob ' || true)"
if [ ! -s "$here/snapshot.sh" ]; then
    bad "snapshot.sh does not exist"
elif [ -z "$writes" ]; then
    ok "snapshot.sh names no temper write but resource create"
else
    bad "snapshot.sh names another temper write: $writes"
fi

# 5. The workstreams are CLAUDE.md's, in its order.
vocabulary="$(awk '/\*\*The grouping key \(ROAD-3\):\*\*/ { on = 1 } on && /^$/ { exit } on' "$root/CLAUDE.md" |
    grep -o '`[^`]*`' | tr -d '`' | grep -v '^open_meta\.workstream$' || true)"
ours="$(jq -n -r -L "$here" 'include "roadmap"; workstreams[]' || true)"
if [ -n "$vocabulary" ] && [ "$vocabulary" = "$ours" ]; then
    ok "the workstreams are CLAUDE.md's vocabulary, in its order"
else
    bad "the workstreams differ from CLAUDE.md's: '$vocabulary' vs '$ours'"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures roadmap snapshot witness(es) failed" >&2
    exit 1
fi
echo "every roadmap snapshot witness passed"
