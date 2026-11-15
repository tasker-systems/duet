"""Play a clip and show where it is: in the clip, in the whole take, and the note being read.

    python3 -I play.py CLIP.wav FROM_SECS TAKE_SECS ANALYSIS.capture.duet

FROM_SECS is where the clip starts in the take, TAKE_SECS the take's length. Ctrl-C stops the
clip; the last position stays on screen until Return, so it can be read off.
"""

import json
import shutil
import subprocess
import sys
import time
import wave

NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def player(path: str) -> list:
    """The command that plays `path`: afplay on macOS, else ffplay."""
    if shutil.which("afplay"):
        return ["afplay", path]
    if shutil.which("ffplay"):
        return ["ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet", path]
    raise SystemExit("needs afplay (macOS) or ffplay to play audio")


def notes_of(path: str) -> list:
    """The reading's notes as (start, end, name, centre, between keys), in seconds."""
    with open(path) as f:
        body = json.load(f)["body"]
    rate = body["rate"]
    return [
        (
            n["start"] / rate,
            n["end"] / rate,
            f"{NAMES[n['key'] % 12]}{n['key'] // 12 - 1}",
            n["centre"],
            n["between_keys"],
        )
        for n in body["reading"]["notes"]
    ]


def note_at(notes: list, t: float) -> str:
    for start, end, name, centre, between in notes:
        if start <= t < end:
            return f"{name:<3} {centre:+4.0f}c{' between keys' if between else ''}"
    return "—  (no note)"


def bar(fraction: float, width: int) -> str:
    full = int(min(max(fraction, 0.0), 1.0) * width)
    return "█" * full + "░" * (width - full)


def take_bar(t: float, clip_from: float, clip_secs: float, take_secs: float, width: int) -> str:
    """The whole take: the clip's span shaded and the playhead marked."""
    cells = []
    for i in range(width):
        lo, hi = i / width * take_secs, (i + 1) / width * take_secs
        if lo <= t < hi:
            cells.append("┃")
        elif hi > clip_from and lo < clip_from + clip_secs:
            cells.append("▒")
        else:
            cells.append("·")
    return "".join(cells)


def main(argv: list) -> int:
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    clip, clip_from, take_secs, cap = argv[1], float(argv[2]), float(argv[3]), argv[4]
    with wave.open(clip) as w:
        clip_secs = w.getnframes() / w.getframerate()
    notes = notes_of(cap)
    width = max(10, min(60, shutil.get_terminal_size((80, 24)).columns - 30))

    playing = subprocess.Popen(player(clip))
    began = time.monotonic()
    print("\n" * 3, end="")
    try:
        while playing.poll() is None:
            elapsed = min(time.monotonic() - began, clip_secs)
            t = clip_from + elapsed
            sys.stdout.write(
                "\x1b[3A"
                f"\r\x1b[K  clip     {bar(elapsed / clip_secs, width)}"
                f"  {elapsed:5.2f} / {clip_secs:.2f} s\n"
                f"\r\x1b[K  take     {take_bar(t, clip_from, clip_secs, take_secs, width)}"
                f"  {t:6.2f} / {take_secs:.1f} s\n"
                f"\r\x1b[K  reading  {note_at(notes, t)}\n"
            )
            sys.stdout.flush()
            time.sleep(0.05)
    except KeyboardInterrupt:
        playing.terminate()
    print("  (Return for the menu)", end="", flush=True)
    sys.stdin.readline()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
