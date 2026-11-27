"""Spike: Claude's side of the shared timeline. It reads and appends data/marks.jsonl directly.

  mark.py threads                      every thread: its root mark and what followed, in order
  mark.py inspect FROM TO              what each reading and the pitch trace hold in a span
  mark.py reply ON NOTE [FROM TO]      append Claude's reply to thread ON, optionally pointing at a span
  mark.py mark FROM TO NOTE            append a new mark of Claude's own

Usage: python3 -I tools/mark.py ...
"""
import datetime
import json
import sys
import uuid
from pathlib import Path

DATA = Path(__file__).resolve().parent.parent / "data"
MARKS = DATA / "marks.jsonl"
NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def name(key):
    return f"{NAMES[key % 12]}{key // 12 - 1}"


def marks():
    return [json.loads(l) for l in MARKS.read_text().splitlines() if l.strip()] if MARKS.exists() else []


def append(entry):
    entry.update(by="claude", id="c-" + uuid.uuid4().hex[:8],
                 at=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"))
    with MARKS.open("a") as f:
        f.write(json.dumps(entry) + "\n")
    print(json.dumps(entry))


def threads():
    ms = marks()
    for root in (m for m in ms if m["kind"] == "mark"):
        print(f"[{root['id']}] {root['by']} {root['from']:.3f}-{root['to']:.3f}: {root.get('note', '')}")
        for m in ms:
            if m.get("on") == root["id"]:
                span = f" {m['from']:.3f}-{m['to']:.3f}" if "from" in m else ""
                print(f"    {m['kind']} by {m['by']}{span}: {m.get('note', '')}")


def inspect(a, b):
    tl = json.loads((DATA / "timeline.json").read_text())
    for r in tl["readings"]:
        off = r["tuning_cents"] - 6900
        inside = [n for n in r["notes"] if n["to"] > a and n["from"] < b]
        print(f"{r['reader']}:")
        prev = None
        for n in inside:
            if prev is not None and n["from"] - prev["to"] > 0.001:
                print(f"    gap {prev['to']:.3f}-{n['from']:.3f} ({(n['from'] - prev['to']) * 1000:.0f} ms)")
            held = f", held from {n['held_from']:.3f}" if n["held_from"] else ""
            bk = " (between keys)" if n["between_keys"] else ""
            print(f"    {name(n['key'])}{bk} {n['from']:.3f}-{n['to']:.3f}{held}")
            prev = n
        if not inside:
            print("    no notes")
    off = tl["readings"][0]["tuning_cents"] - 6900
    print("trace (each ~50 ms: nearest key and cents off it, on the take's tuning):")
    pts = [p for p in tl["trace"] if a <= p[0] <= b]
    for i in range(0, len(pts), 4):
        t, c = pts[i][0], pts[i][1]
        if c is None:
            print(f"    {t:.3f} unvoiced")
        else:
            k = round((c - off) / 100)
            print(f"    {t:.3f} {name(k)} {c - off - k * 100:+.0f}c  voicing {pts[i][2]:.2f}")


cmd, *args = sys.argv[1:] or ["threads"]
if cmd == "threads":
    threads()
elif cmd == "inspect":
    inspect(float(args[0]), float(args[1]))
elif cmd == "reply":
    e = {"kind": "reply", "on": args[0], "note": args[1]}
    if len(args) == 4:
        e.update({"from": float(args[2]), "to": float(args[3])})
    append(e)
elif cmd == "mark":
    append({"kind": "mark", "from": float(args[0]), "to": float(args[1]), "note": args[2]})
else:
    sys.exit(__doc__)
