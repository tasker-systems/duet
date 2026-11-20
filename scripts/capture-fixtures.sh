#!/usr/bin/env bash
# Regenerates generated audio for duet-capture's tests. Usage: capture-fixtures.sh [fixtures|corpus|all]
# (default corpus). It never touches tests/fixtures unless asked.
#
# fixtures: crates/duet-capture/tests/fixtures, a one-second 440 Hz sine, mono at 44.1 kHz
#   (tone-mono44) and stereo at 48 kHz (tone-stereo48), as WAV, FLAC, MP3, Ogg Vorbis and AAC-LC
#   M4A, plus he-aac.m4a, a declared HE-AAC file, and notes22.flac and notes44.flac, five short
#   tones at 22.05 and 44.1 kHz for the tests that damage a file. The WAV files are byte-stable (plain
#   arithmetic); the FLAC files are for a given flac version; the MP3, Ogg and M4A files
#   depend on the encoder versions (lame, oggenc, afconvert) and came out different bytes
#   on another machine.
#   Needs lame, flac, oggenc and macOS's afconvert (-d aac for AAC-LC, -d aach for HE-AAC).
# corpus: the reference corpus's synthetic takes in crates/duet-capture/corpus:
#   synthetic-melody.flac (eight quarter-note sine tones at 120 bpm) and
#   synthetic-beatbox.flac (kick, snare and hat noise bursts, two bars at 120 bpm), and
#   synthetic-disagree.flac (the witness take of `duet capture disagree`, not a manifest take). All are
#   made by arithmetic and a seeded generator, then encoded losslessly by flac, so they are
#   byte-stable for a given flac version, and were byte-identical across two runs here. Needs flac and python3.
#
# The tests use the committed files; run this only to change them. macOS only.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixtures="$root/crates/duet-capture/tests/fixtures"
corpus="$root/crates/duet-capture/corpus"
target="${1:-corpus}"

[[ "$(uname -s)" == Darwin ]] || { echo "error: capture-fixtures runs on macOS only (afconvert)" >&2; exit 1; }
case "$target" in
  fixtures) tools=(lame flac oggenc afconvert python3) ;;
  corpus) tools=(flac python3) ;;
  all) tools=(lame flac oggenc afconvert python3) ;;
  *) echo "usage: capture-fixtures.sh [fixtures|corpus|all]" >&2; exit 2 ;;
esac
for tool in "${tools[@]}"; do
  command -v "$tool" >/dev/null || { echo "error: no $tool; run scripts/dev-setup.sh" >&2; exit 1; }
done

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
  sine "$fixtures/$stem.wav" "$rate" "$channels" "$width"
  flac --silent --force -o "$fixtures/$stem.flac" "$wav"
  lame --silent -b 128 "$wav" "$fixtures/$stem.mp3"
  oggenc --quiet -q 3 -o "$fixtures/$stem.ogg" "$wav"
  afconvert -f m4af -d aac "$wav" "$fixtures/$stem.m4a"
}

# tone_notes <path> <rate>: five 0.1 s tones of 440 Hz, a 5 ms rise and fall each, starting
# at 0.25, 0.75, 1.25, 1.75 and 2.25 s, in 2.5 s of mono 16-bit audio. The tests damage the
# FLAC of it and look for the later tones' onsets where they are in the undamaged file.
tone_notes() {
  python3 -I - "$1" "$2" <<'PY'
import math, struct, sys, wave
path, rate = sys.argv[1], int(sys.argv[2])
samples = [0] * round(2.5 * rate)
ramp, length = round(0.005 * rate), round(0.1 * rate)
for start in (0.25, 0.75, 1.25, 1.75, 2.25):
    first = round(start * rate)
    for n in range(length):
        envelope = min(1.0, n / ramp, (length - 1 - n) / ramp)
        samples[first + n] = round(16384 * envelope * math.sin(2 * math.pi * 440 * n / rate))
with wave.open(path, "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b"".join(struct.pack("<h", x) for x in samples))
PY
}

make_fixtures() {
  mkdir -p "$fixtures"
  rm -f "$fixtures"/*
  make tone-mono44 44100 1 2
  make tone-stereo48 48000 2 1
  # Tones to damage: at the analysis rate (passed through) and at twice it (resampled).
  tone_notes "$work/notes22.wav" 22050
  flac --silent --force -o "$fixtures/notes22.flac" "$work/notes22.wav"
  tone_notes "$work/notes44.wav" 44100
  flac --silent --force -o "$fixtures/notes44.flac" "$work/notes44.wav"
  # Declared HE-AAC: the AAC-LC source is encoded as HE-AAC v1 at a low bit rate.
  afconvert -f m4af -d aach -b 48000 "$work/tone-stereo48.wav" "$fixtures/he-aac.m4a"
  ls -l "$fixtures"
}

# The corpus's synthetic melody: C4 E4 G4 C5 G4 E4 C4 C4 as quarter notes at 120 bpm (0.5 s
# each, the last 50 ms of each silent so repeated pitches are two notes), from 0.25 s in, with
# a 5 ms fade at each end of a tone. 16 kHz mono, 16-bit. The manifest gives the tempo, the
# meter and the downbeat (0.25 s), so only the pitches and onsets are read from the audio.
make_synthetic_melody() {
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
}

# The corpus's synthetic beatbox: two bars of 4/4 at 120 bpm, eighth notes from 0.25 s in,
# kick hat snare hat kick hat snare hat each bar. A kick is noise limited to 40-150 Hz
# (0.1 s), a snare 1-4 kHz (0.1 s), a hat 6-10 kHz (0.05 s), each a sum of 48 sine partials
# at frequencies and phases from a seeded generator, with a 2 ms attack and 30 ms decay.
# 22.05 kHz mono, 16-bit. The manifest gives the tempo, the meter and the downbeat (0.25 s).
make_synthetic_beatbox() {
  python3 -I - "$work/synthetic-beatbox.wav" <<'PY'
import math, random, struct, sys, wave
rate = 22050
kinds = {"kick": (40.0, 150.0, 0.1), "snare": (1000.0, 4000.0, 0.1), "hat": (6000.0, 10000.0, 0.05)}
bar = ["kick", "hat", "snare", "hat", "kick", "hat", "snare", "hat"]
lead, eighth, amplitude, partials = 0.25, 0.25, 0.4, 48
rng = random.Random(20261114)
total = round((lead + eighth * len(bar) * 2 + 0.25) * rate)
samples = [0.0] * total
for i, kind in enumerate(bar * 2):
    low, high, length = kinds[kind]
    parts = [(low + (high - low) * rng.random(), 2 * math.pi * rng.random()) for _ in range(partials)]
    first = round((lead + i * eighth) * rate)
    count = round(length * rate)
    attack, decay = round(0.002 * rate), round(0.03 * rate)
    for n in range(count):
        t = (first + n) / rate
        total_sum = sum(math.sin(2 * math.pi * f * t + p) for f, p in parts)
        envelope = min(n / attack, (count - 1 - n) / decay, 1.0)
        samples[first + n] += total_sum / math.sqrt(partials) * envelope * amplitude
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b"".join(struct.pack("<h", max(-32768, min(32767, round(32767 * x)))) for x in samples))
PY
  flac --silent --force -o "$corpus/synthetic-beatbox.flac" "$work/synthetic-beatbox.wav"
}

# The witness take for `duet capture disagree`: a sung-like tone (six harmonics, phase continuous
# within a note, a 10 ms fade at each end) on A440, 16 kHz mono 16-bit, from 0.25 s in with 0.3 s
# of silence after each note: a plain C4; a G3 held 1.04 s with a turn in its middle (up to A3 over
# 0.04 s, held 0.06 s, back over 0.04 s, with no onset); a plain E3; a G3 held 0.6 s at +50 cents
# with vibrato of 8 cents at 5 Hz; a plain D4. Read with --tuning 440, the rules split the turn
# and the HMM does not, and the two readers' medians fall either side of the G3/G#3 midpoint.
make_synthetic_disagree() {
  python3 -I - "$work/synthetic-disagree.wav" <<'PY'
import math, struct, sys, wave
rate = 16000
G3 = 55
# Each note is a list of (seconds, cents at its start, cents at its end), in absolute cents.
def held(midi, secs):
    return [(secs, midi * 100.0, midi * 100.0)]
turn = [(0.45, 5500, 5500), (0.04, 5500, 5700), (0.06, 5700, 5700), (0.04, 5700, 5500), (0.45, 5500, 5500)]
notes = [held(60, 0.5), turn, held(52, 0.5), "vibrato", held(62, 0.5)]
lead, gap, fade = 0.25, 0.3, 0.01
amps = [1.0, 0.5, 0.33, 0.25, 0.2, 0.16]
samples = [0.0] * round(lead * rate)
for note in notes:
    curve = []
    if note == "vibrato":
        for i in range(round(0.6 * rate)):
            curve.append(G3 * 100 + 50.0 + 8 * math.sin(2 * math.pi * 5 * i / rate))
    else:
        for secs, a, b in note:
            n = round(secs * rate)
            curve.extend(a + (b - a) * i / n for i in range(n))
    ramp = round(fade * rate)
    phase = [0.0] * len(amps)
    for i, cents in enumerate(curve):
        hz = 440 * 2 ** ((cents / 100 - 69) / 12)
        env = min(1.0, i / ramp, (len(curve) - 1 - i) / ramp)
        x = 0.0
        for k, amp in enumerate(amps):
            phase[k] += 2 * math.pi * hz * (k + 1) / rate
            x += amp * math.sin(phase[k])
        samples.append(0.25 * env * x)
    samples += [0.0] * round(gap * rate)
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b"".join(struct.pack("<h", round(32767 * x)) for x in samples))
PY
  flac --silent --force -o "$corpus/synthetic-disagree.flac" "$work/synthetic-disagree.wav"
}

make_corpus() {
  mkdir -p "$corpus"
  make_synthetic_melody
  make_synthetic_beatbox
  make_synthetic_disagree
  ls -l "$corpus"
}

case "$target" in
  fixtures) make_fixtures ;;
  corpus) make_corpus ;;
  all) make_fixtures; make_corpus ;;
esac
