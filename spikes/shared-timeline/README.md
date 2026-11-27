# Spike: a shared timeline (throwaway)

**This is a throwaway spike.** It has no tests and it is not robust. It exists to learn one
thing: does "we both point at the same moment" feel right? It has no bus, server framework,
Tauri, GPUI or nsh, and it does not touch duet's crates. Whether to keep any of it is the
owner's decision. The brief is Temper task `01a21e09-9d0d-718e-bdec-d405d21eb35a`, and the
reasons behind it are in the ideation record `01a21e07-2487-72ac-9256-ea23c3511e2c`.

## Open it

```bash
spikes/shared-timeline/build.sh                 # runs `duet capture melody` twice: hmm and rules
python3 -I spikes/shared-timeline/tools/serve.py # then open http://127.0.0.1:8765/
```

- **Use your own browser** (Chrome, or Safari, which also decodes FLAC). The in-app browser
  shows local pages as static snapshots without audio.
- **Opening `index.html` as a file does not work.** The page needs the helper both to fetch
  the take and to write marks.
- `build.sh` uses the `duet` on your `PATH`. Set `DUET=target/release/duet` to use the one in
  the tree. The page was built with duet 0.4.0 from this branch.

## What's on the page

There is one time axis over the whole take, *Wayfaring Stranger*
(`crates/duet-capture/corpus/wayfaring-stranger.flac`). Stacked on it, from top to bottom:

- **take:** the waveform.
- **rules reading** and **hmm reading:** each reader's notes, drawn as a piano roll. The
  performance's pitch trace is drawn in grey behind both, so you can see where each reader
  departs from the voice. A pale bar means the note is between keys. The thin line under a note
  runs from its start to where it is held from.
- **moments:** the 14 moments from `wayfaring-stranger.moments`.
- **marks:** the threads in `data/marks.jsonl`. Yours are purple; Claude's are pink and carry
  a ✦.

## Try this

1. **Drag across a stretch to mark a span.** The span shows what each reading and moment holds
   there.
2. **Play it.** Press **space**, or click *play span*. Under *hear*, choose the take, the rules
   sine, the hmm sine, or any mix of them. Tick *loop* to repeat it. The *nudge* buttons move
   each edge by 10 ms while it plays.
3. **Mark it.** Type a note and click *mark this span*.
4. **Get an answer.** Ask Claude to answer the marks, or run its tool yourself (below). Its
   replies appear within 2 s without a reload. A thread that gets a new reply is labelled
   *new*.
5. **Look at a thread's history.** Click a thread. Its history is the mark, then each reply and
   each edit in order. After nudging a thread's span, *save nudged span to selected thread*
   records the new span as an edit, so the thread keeps its old spans.

The two threads already in `marks.jsonl` are the loop the spike ran once (see below). Clicking
the one at 8.8 s is a good start.

## The marks file

`data/marks.jsonl` sits beside the take's readings. It holds one JSON object per line and is
only ever appended to. JSONL was chosen over the duet text style because both sides can append
a line without parsing the file, and appending is how the two sides avoid clobbering each other.

```json
{"kind":"mark","id":"m-…","by":"owner","at":"…","from":8.8,"to":9.45,"note":"…"}
{"kind":"reply","id":"c-…","by":"claude","at":"…","on":"m-…","note":"…","from":8.911,"to":8.992}
{"kind":"edit","id":"m-…","by":"owner","at":"…","on":"m-…","from":8.81,"to":9.40}
```

- A `mark` starts a thread.
- A `reply` or an `edit` names its thread with `on`.
- A thread's current span is its latest edit's span.
- A reply can point at a span of its own.

**How each side writes:**

- **The page** posts to the helper, `tools/serve.py`. It is the standard library's file server
  with one extra route, `/marks`, and the helper stamps each post `by: "owner"`.
- **Claude** writes to the file directly with `tools/mark.py`:
  - `threads` lists every thread;
  - `inspect FROM TO` prints both readings' notes, gaps and the pitch trace for a span, which
    is what Claude reads before answering;
  - `reply ON NOTE [FROM TO]` appends a reply;
  - `mark FROM TO NOTE` appends a mark of Claude's own.
- **The page polls** the file every 2 s. That is the whole file watch.

## The loop, run once

1. **The first mark was posted to the helper with curl, as the owner.** It covers 8.8–9.45 s,
   with the note "something catches before the D#3". The second mark was made in the page
   itself, by dragging, typing and clicking: 16.1–16.76 s, "two F#3s or one?"
2. **Claude ran `mark.py inspect` on both spans and replied.**
   - At 8.8 s the HMM leaves an 81 ms gap at 8.911–8.992, and the rules reader bridges it with
     a between-keys C3. The trace shows the voice does unvoice (voicing 0.01 at 8.951) during
     a slide down of about 8 semitones. This is the gap the brief guessed at, and it is real.
   - At 16.39 s both readers split the F#3. Neither voicing nor level breaks there; the pitch
     steps up about 40 cents within 25 ms. Claude also added a mark of its own on the step, so
     it can be looped.
3. **The page was reloaded.** The replies appeared under their threads, and Claude's mark
   appeared pink in the marks lane.
4. **The owner's thread was nudged and saved as an edit.** The thread's history now shows the
   mark, Claude's reply, then the edit with its new span.

## What it taught

- **Pointing at time works, and it pays at once.** `inspect` on a span gave a concrete answer
  in seconds. That answer is the listening sheet's verdict box made two-way. The brief's
  example, an 80 ms HMM gap near 9.0 s, turned out to be real once the span was looked at.
- **The moments file is already stale.** Its comment on 8.80–9.45 says "the hmm hears only a
  scoop here", but today's HMM reads a C3 and leaves a gap. Any mark or moment quoting a
  reading needs to say which reading, from which version of duet. A mark should carry the
  reading's digest or version, not just a time.
- **The page's sine is not the sine `duet capture compare` makes.** Compare holds the sine
  through a gap under 0.2 s; this page goes silent in any gap, so it makes gaps louder. Two
  renderers of one reading disagree. A shared timeline should get its renders from duet, not
  re-derive them.
- **One time axis, but not one zoom.** The take is 22 s, and the events worth arguing about are
  25–80 ms long. The whole-take view is good for finding a place; work happens zoomed in. The
  wheel zoom is enough to feel this, but a minimap or overview strip is the missing piece.
- **Alternatives stacked works** for two readers. With variants, comps and more readers the
  lanes would need folding, or a "compare these two" mode.
- **History as a list works** for a thread's edits. It does not yet scrub the *piece's* history.
  That would need readings and notes versioned over sessions, which is the other meaning of
  "change state", and the spike only shows its shape.
- **A reply that points at a span is not drawn.** Claude's 8.911–8.992 reply is the most precise
  pointer in the file, but it shows only as text. A reply's span should be drawn inside its
  thread's mark, so the reply points in the picture too.
- **What happens after a mark is still open,** and the spike makes the question concrete. When
  Claude says "the split at 16.39 is the readers', not yours", the next act is a change: rejoin
  those two notes in the reading. The file has no `kind` for a proposed change yet.
- **The UI rough edges are the spike's own:**
  - the span panel grows when a span is chosen, which pushes the note box down mid-click;
  - the pitch lanes are short, about 2 px per cent;
  - nothing guards a mark against a take that has changed underneath it.
- **The helper is about 80 lines of standard-library Python, and polling every 2 s was enough.**
  So nothing here yet argues for a bus or server. The argument for one comes when there are
  several devices or several takes, not one page and one file.
