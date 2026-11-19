# duet roadmap snapshot, 2026-11-19

- **Date described:** 2026-11-19, in UTC.
- **Records read:** from 2026-11-19T03:00:00Z to 2026-11-19T03:00:01Z, by `tools/roadmap/snapshot.sh`.
- **Newest record read:** last changed at 2026-11-18T10:00:00Z.
- **Sources:** 10 goals in `+making-music/duet-dev`, of every status; 19 tasks in `+making-music/duet-dev-workflows`; and each task's edges. Every listing was read whole.
- **How each lane was read:** Now, Next and Later were each read from the records, by the recipe at the end. No lane was assigned by hand.

Goals are grouped by `open_meta.workstream`, and ordered within a group by lane, then title. Nothing is scored.

## songs-and-comps

| Lane | Goal | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---:|---:|---:|---:|
| Now | [Songs are written in the journal](./10000000-0000-7000-8000-000000000001) | active | 2 | 0 | 2 | 1 |

## bus-and-project

| Lane | Goal | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---:|---:|---:|---:|
| Now | [The bus carries asks between faces](./10000000-0000-7000-8000-000000000002) | active | 0 | 1 | 2 | 0 |

## workbench

| Lane | Goal | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---:|---:|---:|---:|
| Next | [The workbench \| shows the take](./10000000-0000-7000-8000-000000000003) | paused | 0 | 0 | 1 | 0 |

## capture-and-readings

| Lane | Goal | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---:|---:|---:|---:|
| Now | [Zither capture is being claimed](./10000000-0000-7000-8000-000000000010) | active | 1 | 0 | 1 | 0 |
| Later | [A capture whose work is all done](./10000000-0000-7000-8000-000000000004) | active | 0 | 0 | 0 | 1 |

## models-and-polyphony

| Lane | Goal | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---:|---:|---:|---:|
| Later | [Models wait for the next release](./10000000-0000-7000-8000-000000000005) | active | 0 | 0 | 0 | 0 |

## ci-and-release

No open goal.

## What this snapshot leaves out

- **Work that advances no open goal:** 4 unfinished tasks and 2 done tasks in `+making-music/duet-dev-workflows`. A cancelled task is counted nowhere.
- **Goal statuses omitted:** `completed` (1 goal) and `cancelled` (1 goal).
- **Tasks with more than one goal link,** counted for no goal:
  - [two goals: open](./20000000-0000-7000-8000-000000000016)

### Open goals with no workstream

Each carries no `open_meta.workstream`, or one outside `CLAUDE.md`'s vocabulary, so it sits in no group above.

| Lane | Goal | Workstream | Status | Claimed | Ready, unclaimed | Open | Done |
|---|---|---|---|---:|---:|---:|---:|
| Now | [A goal outside the vocabulary](./10000000-0000-7000-8000-000000000009) | `not-a-workstream` (not in the vocabulary) | active | 0 | 1 | 1 | 0 |
| Next | [A goal nobody grouped](./10000000-0000-7000-8000-000000000006) | none | active | 0 | 0 | 1 | 0 |

## Recipe

Every line above was drawn by this recipe, so a reader with the same records draws the same lanes and counts.

1. **Read.** Every goal in `+making-music/duet-dev`, of every status, and every task in `+making-music/duet-dev-workflows`, each listed whole (`temper resource list --all --with open-meta`). A listing that says it is truncated, or holds fewer rows than its total, stops the draw, as does any read that fails: nothing is written.
2. **Link.** A task's goal is the target of an `advances` edge whose source is that task, read the same way for every task, in one `temper graph traverse --depth 1` seeded with every task listed. An `advances` edge into a task counts for nothing. A traverse that fails, is not JSON, has no node for a task it was seeded with, or holds more or fewer edges at that task than its node's `degree` stops the draw: nothing is written. A task with no such edge, or whose edge points at a goal that is not open or not in `+making-music/duet-dev`, advances no open goal. A task with more than one is named, and counted for no goal.
3. **Open goals.** A goal is open when its `temper-status` is `active` or `paused`. `completed` and `cancelled` goals are left out, and counted.
4. **Count.** For each open goal, over the tasks that advance it:
   - *open*: its `temper-stage` is not `done` or `cancelled`;
   - *claimed*: open, and carrying `open_meta.claim_id`;
   - *ready, unclaimed*: open, carrying `open_meta.state = "ready"`, and no `claim_id`. A ready item that is claimed counts as claimed;
   - *done*: its `temper-stage` is `done`, whether or not it was claimed or ready.

   A cancelled task counts nowhere.
5. **Place.** *Now* when claimed plus ready-unclaimed is above zero; otherwise *Next* when open is above zero; otherwise *Later*. Done work never places a goal.
6. **Group and order.** By `open_meta.workstream`, in the order `CLAUDE.md` writes the vocabulary: `songs-and-comps`, `bus-and-project`, `workbench`, `capture-and-readings`, `models-and-polyphony`, `ci-and-release`. Every workstream is named, and one with no open goal says so. Within a group, goals go by lane (Now, Next, Later), then title, then id. An open goal with no workstream, or one outside the vocabulary, is listed by name under its own heading, in the same order. No goal is ordered by a score.
7. **Leave out, and say so.** The tasks that advance no open goal are counted, unfinished and done apart. The goal statuses omitted are named with their counts, and so is every task with more than one goal link.

Two draws over the same records differ only in the stated date and the times the records were read.
