"""Re-render a comparison's sine so it sounds only inside each note.

`duet capture compare` plays the reading as one legato sine that holds each key until the next
note starts, through any gap. This writes the same comparison with the sine silent wherever the
reading has no note, so a dropped note is heard as a gap rather than as the note before it held.

    python3 -I render_gapped.py COMPARE.wav ANALYSIS.capture.duet OUT.wav FROM_SECS GAIN

COMPARE.wav is a 16-bit PCM comparison (the take on the left), FROM_SECS where its window starts
in the take, and GAIN the sine's level over compare's. The left channel is copied unchanged.
"""

import array
import json
import math
import sys
import wave

SINE_LEVEL = 0.3  # compare's own sine level, before GAIN
FADE_SECS = 0.01  # each note fades in and out over this, so it starts and stops without a click


def main(argv: list) -> int:
    if len(argv) != 6:
        print(__doc__, file=sys.stderr)
        return 2
    src, cap, dst = argv[1], argv[2], argv[3]
    offset, gain = float(argv[4]), float(argv[5])

    with wave.open(src) as w:
        rate, frames, channels = w.getframerate(), w.getnframes(), w.getnchannels()
        if channels != 2 or w.getsampwidth() != 2:
            raise SystemExit(f"{src}: expected 16-bit stereo, got {channels} channels")
        take = array.array("h", w.readframes(frames))

    with open(cap) as f:
        body = json.load(f)["body"]
    if body["rate"] != rate:
        raise SystemExit(f"{cap} is at {body['rate']} Hz, {src} at {rate} Hz")
    reading = body["reading"]
    tuning = reading["tuning"]["a4"]
    a4 = tuning["given"] if "given" in tuning else tuning["estimated"]["value"]

    first = round(offset * rate)
    fade = int(FADE_SECS * rate)
    right = [0.0] * frames
    for note in reading["notes"]:
        start, end = note["start"] - first, note["end"] - first
        if end <= 0 or start >= frames:
            continue
        hz = 440 * 2 ** ((a4 + 100 * (note["key"] - 69) - 6900) / 1200)
        for i in range(max(0, start), min(frames, end)):
            envelope = min(1.0, (i - start) / fade, (end - i) / fade)
            right[i] = (
                SINE_LEVEL * gain * envelope * math.sin(2 * math.pi * hz * (i - start) / rate)
            )

    out = array.array("h", [0] * (2 * frames))
    for i in range(frames):
        out[2 * i] = take[2 * i]
        out[2 * i + 1] = max(-32767, min(32767, int(right[i] * 32767)))
    with wave.open(dst, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(out.tobytes())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
