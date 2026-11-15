## What

<!-- The change, present tense, in a sentence or two: what a reader will find different. -->

## Why

<!-- The defect or the need. If a measurement motivated it, give the number, not the
     investigation that found it. Link the Temper task or decision it builds, if there is one. -->

## How

<!-- The shape of the change, briefly. Say why this shape and not another only where a reviewer
     would otherwise propose the other.

     Then, as short lists:
     - **Formats:** every document kind or schema version this adds or changes (song,
       drum-grid, capture-analysis, performance, the envelope), each marked additive (a minor)
       or breaking (a major). A change to the envelope's shape is a major of every kind. Write
       `none` when no file format changes.
     - **Verified:** what was run, and what a reviewer can re-run. Name the test that fails if
       this regresses: the round trip for a format change (CLAUDE.md), and the duet-e2e test for
       a change to the binary. -->

## Next

<!-- What this leaves for later, in scope terms: the follow-up it unblocks or needs, and one line
     on what it deliberately leaves alone. Write `nothing` when the thread closes here. -->

<!-- ─────────────────────────────────────────────────────────────────────────────────────────
     Three rules, adapted from temper's.

     1. THE CODE CARRIES THE DETAIL. A rationale longer than a paragraph belongs in the doc
        comment or the spec, next to the thing it explains, where it survives this PR being
        merged and forgotten. Link to it; don't restate it.

     2. NO SESSION NARRATIVE. Not what was tried first, which probe was wrong, or what an agent
        reported. The reviewer needs the change, not the path to it.

     3. SAY WHAT IT DOESN'T DO, IN SCOPE TERMS. Reviewers infer coverage from silence. One line
        naming what this leaves alone is worth more than a paragraph of caveats.
     ───────────────────────────────────────────────────────────────────────────────────────── -->
