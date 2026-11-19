## Recipe

Every line above was drawn by this recipe, so a reader with the same records draws the same lanes and counts.

1. **Read.** Every goal in `+making-music/duet-dev`, of every status, and every task in `+making-music/duet-dev-workflows`, each listed whole (`temper resource list --all --with open-meta`). A listing that says it is truncated, or holds fewer rows than its total, stops the draw, as does any read that fails: nothing is written.
2. **Link.** A task's goal is the target of an `advances` edge whose source is that task, read the same way for every task, in one `temper graph traverse --depth 1` seeded with every task listed. An `advances` edge into a task counts for nothing. A traverse that fails, is not JSON, or has no node for a task it was seeded with stops the draw: nothing is written. A task with no such edge, or whose edge points at a goal that is not open or not in `+making-music/duet-dev`, advances no open goal. A task with more than one is named, and counted for no goal.
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
