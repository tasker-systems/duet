#!/usr/bin/env bash
# The fresh verify dir's bite test, with real cargo: two packaging runs at one version, with a
# source change between them, verify against the second source.
#
# A fixture workspace holds every crate the release scripts name, at 9.9.9, with no dependency
# outside it: duet-capture calls duet_dsp::one. publish-crates.sh --out packages it. Then duet-dsp
# gains `two`, duet-capture calls it instead, and --out runs again at the same version, in the same
# target dir. Cargo fingerprints a crate in its local packaging registry by name, version and
# source alone, so a second run in a target dir the first one filled verifies duet-capture against
# the first run's build of duet-dsp and fails (`cannot find function two`), as the v0.4.0
# release-prepare re-run did. The cases:
#   1. the second --out run passes, and its duet-capture package calls `two`;
#   2. no verify dir is left in the target dir after either run;
#   3. the publish stage (--from --dry-run) re-packages the first crate in the ordinary target dir
#      and finds the bytes the build stage verified, as release.yml's publish-crates job requires.
#      This one guards the chain rather than bites: it passes without the fresh verify dir too.
#
# NOT IN CI: it compiles, and CI's `gate` job, where test-api-check.sh runs, compiles nothing
# (CI-1); the fakes there cannot show cargo's fingerprint reuse. `cargo make release-check` runs it
# before the dry run, so it runs with the cargo that will package the release. NOT CHECKED: the real
# crates.io (a fake curl answers 404, so every crate is new) and release-prepare.sh's own dry-run
# packaging, whose --target-dir test-api-check.sh checks with a fake cargo.
#
# RELEASE_SCRIPTS=<dir> runs it against another copy of tools/scripts/release, such as main's, to
# show it fails without this change.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
scripts="${RELEASE_SCRIPTS:-$here}"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/fresh-package-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

# Only the fake curl: cargo is the real one.
mkdir -p "$scratch/bin" "$scratch/crates.io"
ln -s "$here/fixture/bin/curl" "$scratch/bin/curl"
export PATH="$scratch/bin:$PATH" FAKE_CRATES_DIR="$scratch/crates.io"
export CARGO_NET_OFFLINE=true
unset CARGO_TARGET_DIR
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid

failures=0
ok() { echo "ok   $1"; }
bad() {
    echo "FAIL $1" >&2
    failures=$((failures + 1))
}

work="$scratch/work"
mkdir -p "$work/tools/scripts"
cp -R "$scripts" "$work/tools/scripts/release"
rm -rf "$work/tools/scripts/release/fixture"
CRATES=()
while IFS= read -r crate; do CRATES+=("$crate"); done < <(bash -c '
    source "$1/lib/common.sh"
    for c in "${CRATES[@]}"; do echo "$c"; done' _ "$scripts")
{
    printf '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n\n'
    printf '[workspace.package]\nversion = "9.9.9"\nedition = "2021"\nlicense = "MIT"\ndescription = "fixture"\n\n'
    printf '[workspace.dependencies]\n'
    for crate in "${CRATES[@]}"; do
        [[ "$crate" == duet-cli ]] || printf '%s = { path = "crates/%s", version = "9.9.9" }\n' "$crate" "$crate"
    done
} > "$work/Cargo.toml"
for crate in "${CRATES[@]}"; do
    mkdir -p "$work/crates/$crate/src"
    printf '[package]\nname = "%s"\nversion.workspace = true\nedition.workspace = true\n' "$crate" > "$work/crates/$crate/Cargo.toml"
    printf 'license.workspace = true\ndescription.workspace = true\n\n[dependencies]\n' >> "$work/crates/$crate/Cargo.toml"
    echo 'pub fn one() {}' > "$work/crates/$crate/src/lib.rs"
done
echo 'duet-dsp.workspace = true' >> "$work/crates/duet-capture/Cargo.toml"
echo 'pub fn f() { duet_dsp::one() }' > "$work/crates/duet-capture/src/lib.rs"
printf 'target/\ndist*/\n' > "$work/.gitignore"
git -C "$work" init -q -b main
git -C "$work" add -A
git -C "$work" commit -q -m one

verify_dirs_left() { find "$work/target" -maxdepth 1 -name 'release-verify.*' 2> /dev/null | wc -l | tr -d ' '; }

# 1st run, at the first source.
if (cd "$work" && ./tools/scripts/release/publish-crates.sh 9.9.9 --out dist1) > "$scratch/run1.log" 2>&1; then
    ok "the first --out run packages and verifies every crate"
else
    bad "the first --out run failed: $(tail -n 5 "$scratch/run1.log")"
fi
left1="$(verify_dirs_left)"

# 2nd run, at the same version, after duet-capture comes to need what duet-dsp has only now.
echo 'pub fn one() {} pub fn two() {}' > "$work/crates/duet-dsp/src/lib.rs"
echo 'pub fn f() { duet_dsp::two() }' > "$work/crates/duet-capture/src/lib.rs"
git -C "$work" commit -q -am two
if (cd "$work" && ./tools/scripts/release/publish-crates.sh 9.9.9 --out dist2) > "$scratch/run2.log" 2>&1 &&
    tar -xzOf "$work/dist2/duet-capture-9.9.9.crate" duet-capture-9.9.9/src/lib.rs | grep -q 'duet_dsp::two'; then
    ok "a second --out run at the same version verifies against the second source"
else
    bad "the second --out run at the same version did not verify the second source: $(grep -m 1 -E '^error' "$scratch/run2.log" || tail -n 3 "$scratch/run2.log")"
fi
left2="$(verify_dirs_left)"

if [[ "$left1" == 0 && "$left2" == 0 ]]; then
    ok "neither run leaves its verify dir in the target dir"
else
    bad "verify dirs left in the target dir: ${left1} after the first run, ${left2} after the second"
fi

# The publish stage re-packages duet-core in the ordinary target dir and compares its bytes.
if (cd "$work" && ./tools/scripts/release/publish-crates.sh 9.9.9 --from dist2 --dry-run) > "$scratch/from.log" 2>&1 &&
    grep -q 'matches the verified package' "$scratch/from.log"; then
    ok "the publish stage's re-package matches the bytes the build stage verified"
else
    bad "the publish stage's re-package did not match: $(tail -n 3 "$scratch/from.log")"
fi

if [[ "$failures" -gt 0 ]]; then
    echo "$failures failed" >&2
    exit 1
fi
echo "all passed"
