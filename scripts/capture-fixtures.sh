#!/usr/bin/env bash
# Regenerates crates/duet-capture/tests/fixtures: a one-second 440 Hz sine, mono at 44.1 kHz
# (tone-mono44) and stereo at 48 kHz (tone-stereo48), as WAV, FLAC, MP3, Ogg Vorbis and AAC-LC
# M4A, plus he-aac.m4a, a declared HE-AAC file. It also regenerates the reference corpus's
# synthetic take, crates/duet-capture/corpus/synthetic-melody.flac: eight quarter-note sine
# tones at 120 bpm, starting a quarter of a second in. It is made by arithmetic alone, so a
# rerun gives the same samples. The tests use the committed files; run this only to change them.
#
# Needs lame, flac and oggenc (scripts/dev-setup.sh, or `brew bundle`), and macOS's afconvert,
# which makes the AAC-LC (-d aac) and HE-AAC (-d aach) files. macOS only.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/crates/duet-capture/tests/fixtures"

[[ "$(uname -s)" == Darwin ]] || { echo "error: capture-fixtures runs on macOS only (afconvert)" >&2; exit 1; }
for tool in lame flac oggenc afconvert python3; do
  command -v "$tool" >/dev/null || { echo "error: no $tool; run scripts/dev-setup.sh" >&2; exit 1; }
done

mkdir -p "$out"
rm -f "$out"/*
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# sine <path> <rate> <channels> <bytes per sample>: one second of 440 Hz at half scale.
sine() {
  python3 -I - "$1" "$2" "$3" "$4" <<'PY'
import math, struct, sys, wave
path, rate, channels, width = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
with wave.open(path, "wb") as w:
    w.setnchannels(channels)
    w.setsampwidth(width)
    w.setframerate(rate)
    # 8-bit WAV is unsigned, centred on 128; 16-bit is signed.
    def sample(n):
        x = math.sin(2 * math.pi * 440 * n / rate)
        return struct.pack("<h", round(16384 * x)) if width == 2 else struct.pack("B", 128 + round(64 * x))
    frames = b"".join(sample(n) * channels for n in range(rate))
    w.writeframes(frames)
PY
}

# make <stem> <rate> <channels> <bytes per sample in the committed WAV>: the other formats are
# encoded from a 16-bit source. The 8-bit WAV keeps the stereo 48 kHz file under 100 KB.
make() {
  local stem="$1" rate="$2" channels="$3" width="$4" wav="$work/$1.wav"
  sine "$wav" "$rate" "$channels" 2
  sine "$out/$stem.wav" "$rate" "$channels" "$width"
  flac --silent --force -o "$out/$stem.flac" "$wav"
  lame --silent -b 128 "$wav" "$out/$stem.mp3"
  oggenc --quiet -q 3 -o "$out/$stem.ogg" "$wav"
  afconvert -f m4af -d aac "$wav" "$out/$stem.m4a"
}

make tone-mono44 44100 1 2
make tone-stereo48 48000 2 1

# Declared HE-AAC: the AAC-LC source is encoded as HE-AAC v1 at a low bit rate.
afconvert -f m4af -d aach -b 48000 "$work/tone-stereo48.wav" "$out/he-aac.m4a"

# The corpus's synthetic melody: C4 E4 G4 C5 G4 E4 C4 C4 as quarter notes at 120 bpm (0.5 s
# each, the last 50 ms of each silent so repeated pitches are two notes), from 0.25 s in, with
# a 5 ms fade at each end of a tone. 16 kHz mono, 16-bit. The manifest gives the tempo, the
# meter and the downbeat (0.25 s), so only the pitches and onsets are read from the audio.
corpus="$root/crates/duet-capture/corpus"
mkdir -p "$corpus"
python3 -I - "$work/synthetic-melody.wav" <<'PY'
import math, struct, sys, wave
rate = 16000
midis = [60, 64, 67, 72, 67, 64, 60, 60]
lead, quarter, gap, fade = 0.25, 0.5, 0.05, 0.005
total = round((lead + quarter * len(midis) + 0.25) * rate)
samples = [0] * total
for i, midi in enumerate(midis):
    hz = 440 * 2 ** ((midi - 69) / 12)
    start = round((lead + i * quarter) * rate)
    length = round((quarter - gap) * rate)
    ramp = round(fade * rate)
    for n in range(length):
        envelope = min(1.0, n / ramp, (length - 1 - n) / ramp)
        samples[start + n] = round(16384 * envelope * math.sin(2 * math.pi * hz * n / rate))
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b"".join(struct.pack("<h", x) for x in samples))
PY
flac --silent --force -o "$corpus/synthetic-melody.flac" "$work/synthetic-melody.wav"

ls -l "$out" "$corpus"
