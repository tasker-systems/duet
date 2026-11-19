# duet's roadmap snapshot, drawn from the Temper records: the whole rule, as jq definitions.
# recipe.md says the same in words, and ends every snapshot. snapshot.jq calls `snapshot`;
# test.sh calls the parts.
#
# `snapshot($started; $finished; $recipe)` takes one object, as snapshot.sh assembles it from `temper` (and as fixture/records.json holds):
#   {"goals": <resource list page>, "tasks": <resource list page>,
#    "traverse": <graph traverse --depth 1, seeded with every listed task>}
# $started and $finished are YYYY-MM-DDTHH:MM:SSZ, when the reads began and ended; $recipe is recipe.md.
# It answers the snapshot's markdown body. Nothing in it depends on the order of the input
# rows, and only the stated date and read times depend on the arguments.
#
# Any read that is partial or out of shape is an error: jq exits non-zero, prints nothing on
# stdout, and snapshot.sh writes nothing.

# The values of open_meta.workstream, in the order CLAUDE.md writes them (ROAD-3). test.sh checks
# this list against CLAUDE.md.
def workstreams:
  ["songs-and-comps", "bus-and-project", "workbench", "capture-and-readings", "models-and-polyphony", "ci-and-release"];
def goal_statuses: ["active", "paused", "completed", "cancelled"];
def task_stages: ["backlog", "in-progress", "done", "cancelled"];

# A listing's rows, only if it is whole: not marked truncated, and holding as many rows as it says
# it returned and as its total.
def whole($what):
  if (.rows | type) != "array" then error("the \($what) listing answered in a shape this tool does not read: it has no `rows` list")
  elif (.total | type) != "number" or (.returned | type) != "number" or (.truncated | type) != "boolean"
    then error("the \($what) listing answered in a shape this tool does not read: it lacks `total`, `returned` or `truncated`")
  elif .truncated then error("the \($what) listing was truncated: it says so (`truncated: true`), holding \(.rows | length) of \(.total) rows; refusing to draw from a partial page")
  elif .returned != (.rows | length) then error("the \($what) listing was truncated: it says it returned \(.returned) rows but holds \(.rows | length); refusing to draw from a partial page")
  elif (.rows | length) != .total then error("the \($what) listing was truncated: it returned \(.rows | length) of \(.total) rows; refusing to draw from a partial page")
  else .rows end;

def need($row; $key; $what):
  if ($row[$key] | type) == "string" then $row[$key]
  else error("a \($what) row has no `\($key)`") end;

def goal:
  . as $r
  | need($r; "id"; "goal") as $id | need($r; "title"; "goal") as $title
  | ($r.managed_meta["temper-status"] // "") as $status
  | if (goal_statuses | any(. == $status) | not) then
      error("goal \($id) (\($title)) has status \($status | tojson), which is none of \(goal_statuses | tojson); the lanes cannot tell whether it is open")
    else
      {id: $id, title: $title, status: $status, updated: need($r; "updated"; "goal"),
       workstream: (if ($r.open_meta.workstream | type) == "string" then $r.open_meta.workstream else null end),
       open: ($status == "active" or $status == "paused")}
    end;

# The traverse, checked before it is trusted:
#   {nodes: {<id>: <its degree>}, edges: {<id>: <edges the reply holds at it>}, links: {<task id>: [<target id>]}}.
# A task's goal links are the targets of the `advances` edges whose source is that task; an edge
# into a task, from a session or anything else, is no task's link.
def traversed:
  if (.nodes | type) != "array" or (.edges | type) != "array" then
    error("the traverse answered in a shape this tool does not read: it lacks a `nodes` or `edges` list")
  elif any(.nodes[]; (.id | type) != "string") then
    error("the traverse answered in a shape this tool does not read: a node has no string `id`")
  elif any(.edges[]; (.source | type) != "string" or (.target | type) != "string") then
    error("the traverse answered in a shape this tool does not read: an edge has no string `source` or `target`")
  else
    {nodes: (.nodes | map({(.id): .degree}) | add // {}),
     edges: ([.edges[] | if .source == .target then .source else .source, .target end]
             | group_by(.) | map({(.[0]): length}) | add // {}),
     links: (.edges | map(select(.label == "advances")) | group_by(.source)
             | map({(.[0].source): map(.target)}) | add // {})}
  end;

# A task, with its goal links. It must have a node in the traverse it was seeded into, and the reply
# must hold exactly as many edges at it as that node's `degree` says it has: a task the traverse
# left out, or whose edges it did not all return, is a partial read, never a task with no goal.
def task($graph):
  . as $r
  | need($r; "id"; "task") as $id | need($r; "title"; "task") as $title
  | ($r.managed_meta["temper-stage"] // "") as $stage
  | if (task_stages | any(. == $stage) | not) then
      error("task \($id) (\($title)) has stage \($stage | tojson), which is none of \(task_stages | tojson); the counts cannot tell whether it is finished")
    elif ($r.open_meta | type) != "object" then
      error("task \($id) has no `open_meta`, so its claim and ready markers cannot be read")
    elif ($graph.nodes | has($id) | not) then
      error("the traverse has no node for task \($id) (\($title)), which it was seeded with: its edges were not read, so it cannot be counted as having no goal")
    elif ($graph.nodes[$id] | type) != "number" then
      error("the traverse gives task \($id) (\($title)) no numeric `degree`, so it cannot show that every one of its edges was read")
    elif $graph.nodes[$id] != ($graph.edges[$id] // 0) then
      error("the traverse holds \($graph.edges[$id] // 0) of the \($graph.nodes[$id]) edges its node gives task \($id) (\($title)), which it was seeded with: a partial read, so its goal links cannot be trusted")
    else
      {id: $id, title: $title, stage: $stage, updated: need($r; "updated"; "task"),
       unfinished: ($stage != "done" and $stage != "cancelled"),
       claimed: ($r.open_meta | has("claim_id") and .claim_id != null),
       ready: ($r.open_meta.state == "ready"),
       links: ($graph.links[$id] // [] | unique)}
    end;

def lane_of: if .claimed + .ready_unclaimed > 0 then "Now" elif .open > 0 then "Next" else "Later" end;
def lane_rank: {"Now": 0, "Next": 1, "Later": 2}[.];

# Text for a table cell or a link's text: on one line, with no `|` or bracket to break it.
def cell:
  [splits("\\s+") | select(length > 0)] | join(" ")
  | gsub("\\\\"; "\\\\") | gsub("\\|"; "\\|") | gsub("\\["; "\\[") | gsub("\\]"; "\\]");

def plural($n; $one; $many): "\($n) \(if $n == 1 then $one else $many end)";

def table($with_ws):
  "| Lane | Goal |\(if $with_ws then " Workstream |" else "" end) Status | Claimed | Ready, unclaimed | Open | Done |\n"
  + "|---|---|\(if $with_ws then "---|" else "" end)---|---:|---:|---:|---:|\n"
  + (map("| \(.lane) | [\(.title | cell)](./\(.id)) |"
         + (if $with_ws then
              (if .workstream == null then " none |" else " `\(.workstream | cell)` (not in the vocabulary) |" end)
            else "" end)
         + " \(.status) | \(.claimed) | \(.ready_unclaimed) | \(.open) | \(.done) |\n")
     | join(""));

def order: sort_by([(.lane | lane_rank), .title, .id]);

def snapshot($started; $finished; $recipe):
  . as $in
  | ($in.goals | whole("goal") | map(goal) | sort_by(.id)) as $goals
  | ($in.traverse | traversed) as $graph
  | ($in.tasks | whole("task") | map(task($graph)) | sort_by(.id)) as $tasks
  | ($goals | map(select(.open)) | map({key: .id, value: .}) | from_entries) as $open
  # Each task counts toward the one open goal it links to, or toward nothing.
  | ($tasks | map(. + {goal: (if (.links | length) == 1 and $open[.links[0]] then .links[0] else null end)})) as $placed
  | ($goals | map(select(.open)) | map(. as $g
      | ($placed | map(select(.goal == $g.id and (.links | length) == 1))) as $work
      | {id: $g.id, title: $g.title, status: $g.status, workstream: $g.workstream,
         claimed: ($work | map(select(.unfinished and .claimed)) | length),
         ready_unclaimed: ($work | map(select(.unfinished and (.claimed | not) and .ready)) | length),
         open: ($work | map(select(.unfinished)) | length),
         done: ($work | map(select(.stage == "done")) | length)}
      | . + {lane: lane_of})) as $lines
  | ($placed | map(select((.links | length) <= 1 and .goal == null))) as $unlinked
  | ($placed | map(select((.links | length) > 1 and .stage != "cancelled")) | sort_by(.id)) as $multiple
  | ($lines | map(select(.workstream as $w | workstreams | any(. == $w) | not)) | order) as $ungrouped
  | ($started | split("T")[0]) as $date
  | ([$goals[].updated, $tasks[].updated] | max) as $newest
  | "# duet roadmap snapshot, \($date)\n\n"
    + "- **Date described:** \($date), in UTC.\n"
    + "- **Records read:** from \($started) to \($finished), by `tools/roadmap/snapshot.sh`.\n"
    + (if $newest == null then "- **Newest record read:** none; no records were listed.\n"
       else "- **Newest record read:** last changed at \($newest).\n" end)
    + "- **Sources:** \(plural($goals | length; "goal"; "goals")) in `+making-music/duet-dev`, of every status; "
    + "\(plural($tasks | length; "task"; "tasks")) in `+making-music/duet-dev-workflows`; and each task's edges. "
    + "Every listing was read whole.\n"
    + "- **How each lane was read:** Now, Next and Later were each read from the records, by the recipe at the end. "
    + "No lane was assigned by hand.\n\n"
    + "Goals are grouped by `open_meta.workstream`, and ordered within a group by lane, then title. Nothing is scored.\n\n"
    + (workstreams | map(. as $w | ($lines | map(select(.workstream == $w)) | order) as $g
        | "## \($w)\n\n" + (if ($g | length) == 0 then "No open goal.\n\n" else ($g | table(false)) + "\n" end))
       | join(""))
    + "## What this snapshot leaves out\n\n"
    + "- **Work that advances no open goal:** \(plural($unlinked | map(select(.unfinished)) | length; "unfinished task"; "unfinished tasks")) "
    + "and \(plural($unlinked | map(select(.stage == "done")) | length; "done task"; "done tasks")) in `+making-music/duet-dev-workflows`. "
    + "A cancelled task is counted nowhere.\n"
    + "- **Goal statuses omitted:** "
    + (["completed", "cancelled"] | map(. as $s | "`\($s)` (\(plural($goals | map(select(.status == $s)) | length; "goal"; "goals")))") | join(" and "))
    + ".\n"
    + (if ($multiple | length) == 0 then "- **Tasks with more than one goal link:** none.\n"
       else "- **Tasks with more than one goal link,** counted for no goal:\n"
         + ($multiple | map("  - [\(.title | cell)](./\(.id))\n") | join("")) end)
    + "\n### Open goals with no workstream\n\n"
    + (if ($ungrouped | length) == 0 then "None: every open goal carries one of the \(workstreams | length) workstreams.\n\n"
       else "Each carries no `open_meta.workstream`, or one outside `CLAUDE.md`'s vocabulary, so it sits in no group above.\n\n"
         + ($ungrouped | table(true)) + "\n" end)
    + $recipe;
