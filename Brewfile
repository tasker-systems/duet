# Developer tools mise cannot install (neither has a mise registry entry). Install with
# `brew bundle` or scripts/dev-setup.sh. Nothing in CI uses this file.
# Homebrew installs a formula's current version, so each tool's version is checked where it
# matters, not here.

# abc2midi, for duet-abc's local oracle. The oracle fails on any version other than
# crates/duet-abc/corpus/manifest.toml's abc2midi_version.
brew "abcmidi"

# FluidSynth, a SoundFont synth that opens a MIDI port: scripts/port-check.sh sends it
# `duet play --port` and checks what it received.
brew "fluid-synth"

# lame, flac and vorbis-tools (oggenc): encode the MP3, FLAC and Ogg Vorbis fixtures for
# duet-capture's decode tests, in scripts/capture-fixtures.sh. The AAC and HE-AAC fixtures use
# macOS's own afconvert. Run the script only to regenerate fixtures; CI uses the committed files.
brew "lame"
brew "flac"
brew "vorbis-tools"
