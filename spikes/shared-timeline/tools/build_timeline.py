"""Spike: fold what `duet capture melody` wrote into one timeline.json the page reads.

Everything on it is in seconds from the start of the take, so the page has one time axis.
Usage: build_timeline.py DATA_DIR MOMENTS_FILE
"""
import json
import sys
from pathlib import Path

data = Path(sys.argv[1])
moments_path = Path(sys.argv[2])


def reading(stem):
    body = json.loads((data / f"{stem}.capture.duet").read_text())["body"]
    rate = body["rate"]
    r = body["reading"]
    a4 = r["tuning"]["a4"]
    a4_cents = next(iter(a4.values()))["value"]  # estimated or given; absolute cents (6900 = 440 Hz)
    notes = []
    for n in r["notes"]:
        notes.append({
            "from": round(n["start"] / rate, 4),
            "to": round(n["end"] / rate, 4),
            "held_from": round(n["held_from"] / rate, 4) if n.get("held_from") is not None else None,
            "key": n["key"],
            "between_keys": n.get("between_keys", False),
            "confidence": round(n.get("confidence", 0), 3),
        })
    return {"reader": r["reader"]["name"], "tuning_cents": a4_cents, "notes": notes}


def pitch_trace(stem):
    lines = (data / f"{stem}.performance.duet.jsonl").read_text().splitlines()
    head = json.loads(lines[0])["head"]
    rate, hop = head["rate"], head["hop"]
    points = []  # [secs, cents|null, voicing, level]
    for line in lines[1:]:
        chunk = json.loads(line)
        for i, p in enumerate(chunk["pitch"]):
            t = (chunk["start"] + i * hop) / rate
            points.append([round(t, 4), p, round(chunk["voicing"][i], 3), round(chunk["level"][i], 4)])
    return {"rate": rate, "hop": hop, "frames": head["take"]["frames"], "points": points}


def moments():
    out = []
    for line in moments_path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        a, b, label = line.split(" ", 2)
        out.append({"from": float(a), "to": float(b), "label": label})
    return out


trace = pitch_trace("ws.hmm")
timeline = {
    "take": "take.flac",
    "duration": trace["frames"] / trace["rate"],
    "trace": trace["points"],
    "readings": [reading("ws.rules"), reading("ws.hmm")],
    "moments": moments(),
}
(data / "timeline.json").write_text(json.dumps(timeline))
print(f"{len(trace['points'])} frames, " + ", ".join(
    f"{r['reader']}: {len(r['notes'])} notes" for r in timeline["readings"]) + f", {len(timeline['moments'])} moments")
