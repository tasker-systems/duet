#!/usr/bin/env bash
# The pre-publish API check's bite test (TW-5; COMPAT-1, COMPAT-2), over fixture repositories with
# no network: real git, and fakes for curl (crates.io's versions API) and cargo (cargo-semver-checks
# and cargo package) from fixture/bin. Each refusal is shown biting, and its sanctioned form
# passing:
#   1. api-check.sh refuses a break the bump does not admit, and passes it when the bump does;
#   2. it judges against the last release from last_release, passed as --baseline-version;
#   3. a crates.io answer that is not a definite 404 or a versions list refuses, never reads as
#      new; a 404, or a crate listed at the candidate alone, is new;
#   4. a crate listed, but not at the last release, or with no release tag before it, refuses;
#   5. the last release fails closed: a shallow clone, origin's tags unlistable, a tag whose commit
#      is not in the clone, no one newest tag, or a candidate not newer than it; a tag off HEAD's
#      history is not taken, and an older tag is not taken when a newer one is there;
#   6. release-prepare.sh --dry-run runs the same check, refuses a crates.io failure and a VERSION
#      that is not the last release, and passes cargo-semver-checks the derived baseline;
#   7. release.yml's build-crates runs api-check.sh on the tagged candidate, from a full clone,
#      before it packages anything;
#   6b. release-prepare.sh --dry-run and publish-crates.sh --out verify-package in a fresh target
#      dir under the target dir, removed after (the bite with real cargo is test-fresh-package.sh);
#   8. a crate in HELD is not judged, packaged or published, and is named; a release that would
#      publish a crate depending on a held one, through a dependency that carries a version, is
#      refused, as is a HELD naming no crate.
#
# NOT CHECKED: that cargo-semver-checks finds a real break (the fake stands in for it), the real
# crates.io API, and a real release.yml run.
#
# RELEASE_SCRIPTS=<dir> runs the cases against another copy of tools/scripts/release, such as
# main's, to show they fail without this change, and RELEASE_WORKFLOW=<file> another release.yml. CI runs this in .github/workflows/ci.yml#gate.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
scripts="${RELEASE_SCRIPTS:-$here}"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/api-check-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
export PATH="$here/fixture/bin:$PATH"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid

LIBS=(duet-core duet-midi duet-abc duet-drums duet-dsp duet-capture duet-play duet-schemas duet-bus duet-actions)
# The library crates these release scripts judge: every one but those they hold back (HELD), read
# from the scripts under test, so a copy with no HELD (main's, before it) judges all of LIBS.
JUDGED=()
while IFS= read -r crate; do JUDGED+=("$crate"); done < <(bash -c '
    source "$1/lib/common.sh"
    if declare -p PUBLISHED > /dev/null 2>&1; then list=("${PUBLISHED[@]}"); else list=("${CRATES[@]}"); fi
    for c in "${list[@]}"; do [[ "$c" == duet-cli ]] || echo "$c"; done' _ "$scripts")

failures=0
ok() { echo "ok   $1"; }
bad() {
    echo "FAIL $1" >&2
    failures=$((failures + 1))
}

# A fixture: an origin with main at c1 (annotated tag v0.3.0, VERSION 0.3.0) then c2, a full clone
# of it at <dir>/work, and crates.io listing every library crate at 0.1.0 to 0.3.0.
# fixture <name> → prints <dir>
fixture() {
    local dir="$scratch/$1" crate
    mkdir -p "$dir/crates"
    git init -q --bare -b main "$dir/origin.git"
    git init -q -b main "$dir/seed"
    mkdir -p "$dir/seed/tools/scripts"
    cp -R "$scripts" "$dir/seed/tools/scripts/release"
    rm -rf "$dir/seed/tools/scripts/release/fixture" "$dir/seed/tools/scripts/release/test-api-check.sh"
    printf '[workspace.package]\nversion = "0.3.0"\n' > "$dir/seed/Cargo.toml"
    echo 0.3.0 > "$dir/seed/VERSION"
    git -C "$dir/seed" add -A
    git -C "$dir/seed" commit -q -m c1
    git -C "$dir/seed" tag -a v0.3.0 -m v0.3.0
    echo two > "$dir/seed/change"
    git -C "$dir/seed" add -A
    git -C "$dir/seed" commit -q -m c2
    git -C "$dir/seed" push -q "$dir/origin.git" main --tags
    git clone -q "file://$dir/origin.git" "$dir/work"
    for crate in "${LIBS[@]}"; do echo "versions: 0.1.0 0.2.0 0.3.0" > "$dir/crates/$crate"; done
    echo "$dir"
}
# Tag HEAD of <dir>/work as v<version> on origin, as release-tag.yml does: tag_head <dir> <version>
tag_head() {
    git -C "$1/work" tag "v$2"
    git -C "$1/work" push -q origin "v$2"
}
# A new commit on <dir>/work's main, pushed: commit_work <dir> <message>
commit_work() {
    echo "$2" >> "$1/work/change"
    git -C "$1/work" commit -q -am "$2"
    git -C "$1/work" push -q origin main
}
# Run a release script in <dir>/work: run <dir> <script> [args...]; output lands in <dir>.
run() {
    local dir="$1" script="$2" status=0
    shift 2
    if [[ ! -e "$dir/work/tools/scripts/release/$script" ]]; then
        echo "no $script in these release scripts" > "$dir/stderr"
        : > "$dir/stdout"
        echo 127 > "$dir/status"
        return 0
    fi
    (cd "$dir/work" && FAKE_CRATES_DIR="$dir/crates" bash "tools/scripts/release/$script" "$@") \
        > "$dir/stdout" 2> "$dir/stderr" || status=$?
    echo "$status" > "$dir/status"
}
status() { cat "$1/status"; }
said() { grep -qF -- "$2" "$1/stdout" "$1/stderr"; }
checks() { grep -c '^semver-checks check-release' "$1/crates/cargo.log" 2> /dev/null || true; }
# Passed: expect_pass <dir> <case>; refused naming <text>: expect_refused <dir> <case> <text>
expect_pass() {
    if [[ "$(status "$1")" == 0 ]]; then ok "$2"; else bad "$2: exit $(status "$1"): $(tail -n 3 "$1/stderr")"; fi
}
expect_refused() {
    if [[ "$(status "$1")" == 0 ]]; then
        bad "$2: passed"
    elif ! said "$1" "$3"; then
        bad "$2: refused, but without '$3': $(tail -n 3 "$1/stderr")"
    else
        ok "$2"
    fi
}

# 1, 2. The bump, and the baseline it is judged against.
d="$(fixture patch-clean)"
tag_head "$d" 0.3.1
run "$d" api-check.sh 0.3.1
expect_pass "$d" "a patch with no break passes"
if [[ "$(checks "$d")" == "${#JUDGED[@]}" ]] &&
    [[ "$(grep -c -- '--baseline-version 0.3.0 --release-type minor$' "$d/crates/cargo.log")" == "${#JUDGED[@]}" ]] &&
    ! grep -q -- '-p duet-cli' "$d/crates/cargo.log"; then
    ok "every library crate, and not duet-cli, is judged against 0.3.0 at a duet patch (--release-type minor)"
else
    bad "the check-release calls are not one per library crate against 0.3.0: $(cat "$d/crates/cargo.log" 2> /dev/null)"
fi

d="$(fixture patch-break)"
tag_head "$d" 0.3.1
FAKE_BREAKS=duet-midi run "$d" api-check.sh 0.3.1
expect_refused "$d" "a break at a patch is refused" "refused: duet-midi"

d="$(fixture minor-break)"
tag_head "$d" 0.4.0
FAKE_BREAKS=duet-midi run "$d" api-check.sh 0.4.0
expect_pass "$d" "the same break at a minor (below 1.0, the breaking bump) passes"

# 3. crates.io answers that are not a definite 404.
for answer in "status: 503" "status: 429" "net" "body: <html>maintenance</html>"; do
    d="$(fixture "probe-${answer%%[:<]*}-$RANDOM")"
    tag_head "$d" 0.3.1
    echo "$answer" > "$d/crates/duet-dsp"
    run "$d" api-check.sh 0.3.1
    case "$answer" in
        status:*) why="crates.io answered HTTP ${answer#status: } for duet-dsp" ;;
        net) why="could not ask crates.io about duet-dsp" ;;
        *) why="crates.io answered 200 for duet-dsp without a versions list" ;;
    esac
    if [[ "$(status "$d")" != 0 ]] && said "$d" "$why" && ! said "$d" "duet-dsp: not on crates.io"; then
        ok "crates.io answering '$answer' refuses, and duet-dsp is not read as new"
    else
        bad "crates.io answering '$answer': exit $(status "$d"), $(grep duet-dsp "$d/stdout" || true)"
    fi
done

d="$(fixture probe-404)"
tag_head "$d" 0.3.1
rm "$d/crates/duet-dsp"
run "$d" api-check.sh 0.3.1
expect_pass "$d" "a definite 404 is a new crate"
if said "$d" "new crates: duet-dsp" && ! grep -q -- '-p duet-dsp ' "$d/crates/cargo.log"; then
    ok "the 404 crate is reported new and not judged"
else
    bad "the 404 crate: $(grep -i 'new crates' "$d/stdout" || true)"
fi

d="$(fixture bootstrapped)"
tag_head "$d" 0.3.1
echo "versions: 0.3.1" > "$d/crates/duet-dsp"
run "$d" api-check.sh 0.3.1
expect_pass "$d" "a crate crates.io lists at the candidate alone is new"

# 4. Listed, but with nothing to judge it against.
d="$(fixture not-at-last)"
tag_head "$d" 0.3.1
echo "versions: 0.1.0 0.2.0" > "$d/crates/duet-dsp"
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a crate crates.io lists, but not at the last release, is refused" "but not at the last release 0.3.0"

# 5. The last release.
d="$(fixture shallow)"
tag_head "$d" 0.3.1
rm -rf "$d/work"
git clone -q --depth 1 "file://$d/origin.git" "$d/work"
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a shallow clone is refused" "shallow"

d="$(fixture unlistable)"
tag_head "$d" 0.3.1
git -C "$d/work" remote set-url origin "$d/no-such-origin.git"
run "$d" api-check.sh 0.3.1
expect_refused "$d" "origin's tags unlistable is refused" "cannot list origin's tags"

d="$(fixture unfetched)"
git clone -q "file://$d/origin.git" "$d/other"
echo three >> "$d/other/change"
git -C "$d/other" commit -q -am c3
git -C "$d/other" tag v0.3.5
git -C "$d/other" push -q origin main v0.3.5
tag_head "$d" 0.3.1
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a release tag on origin whose commit is not in the clone is refused" "fetch tags"

d="$(fixture side-tag)"
git -C "$d/work" checkout -q -b side HEAD~1
echo side > "$d/work/side"
git -C "$d/work" add side
git -C "$d/work" commit -q -m side
git -C "$d/work" tag v0.9.0
git -C "$d/work" push -q origin side v0.9.0
git -C "$d/work" checkout -q main
tag_head "$d" 0.3.1
run "$d" api-check.sh 0.3.1
if [[ "$(status "$d")" == 0 ]] && grep -q -- '--baseline-version 0.3.0 ' "$d/crates/cargo.log"; then
    ok "a release tag off HEAD's history is not taken as the last release"
else
    bad "a release tag off HEAD's history: exit $(status "$d"), $(head -n 1 "$d/crates/cargo.log" 2> /dev/null)"
fi

d="$(fixture newest)"
tag_head "$d" 0.3.1
commit_work "$d" c3
tag_head "$d" 0.3.2
for crate in "${LIBS[@]}"; do echo "versions: 0.1.0 0.2.0 0.3.0 0.3.1" > "$d/crates/$crate"; done
run "$d" api-check.sh 0.3.2
if [[ "$(status "$d")" == 0 ]] && [[ "$(grep -c -- '--baseline-version 0.3.1 ' "$d/crates/cargo.log")" == "${#JUDGED[@]}" ]]; then
    ok "the newest release tag, 0.3.1, is the baseline, not the older 0.3.0"
else
    bad "the newest release tag: exit $(status "$d"), $(head -n 1 "$d/crates/cargo.log" 2> /dev/null)"
fi

d="$(fixture two-newest)"
git -C "$d/work" tag v0.3.1 HEAD~1
git -C "$d/work" push -q origin v0.3.1
commit_work "$d" c3
tag_head "$d" 0.3.2
run "$d" api-check.sh 0.3.2
expect_refused "$d" "two release tags on one commit, neither newest, are refused" "no one release tag is the newest"

d="$(fixture not-newer)"
run "$d" api-check.sh 0.2.9
expect_refused "$d" "a candidate not newer than the last release is refused" "not newer than the last release 0.3.0"

d="$(fixture untagged)"
git -C "$d/work" push -q origin :refs/tags/v0.3.0
git -C "$d/work" tag -d v0.3.0 > /dev/null
run "$d" api-check.sh 0.3.1
expect_refused "$d" "crates on crates.io with no release tag before the candidate are refused" "no release tag precedes 0.3.1"
for crate in "${LIBS[@]}"; do rm "$d/crates/$crate"; done
run "$d" api-check.sh 0.3.1
expect_pass "$d" "with no release tag, every crate a definite 404 passes as new"

# 6. release-prepare.sh --dry-run: the same check, through the same derivation.
d="$(fixture prepare-clean)"
CARGO_TARGET_DIR="$d/target" run "$d" release-prepare.sh --dry-run
if [[ "$(status "$d")" == 0 ]] && ! said "$d" "(a real run refuses here)"; then
    ok "release-prepare --dry-run passes a clean patch"
else
    bad "release-prepare --dry-run on a clean patch: exit $(status "$d"): $(grep -h 'refuses here' "$d/stdout" "$d/stderr" || tail -n 3 "$d/stderr")"
fi
if [[ "$(grep -c -- '--baseline-version 0.3.0 --release-type minor$' "$d/crates/cargo.log" 2> /dev/null || true)" == "${#JUDGED[@]}" ]]; then
    ok "release-prepare passes cargo-semver-checks the derived last release as --baseline-version"
else
    bad "release-prepare's check-release calls carry no derived baseline: $(grep check-release "$d/crates/cargo.log" | head -n 1)"
fi
# fresh_verify <dir> <label>: the run's one package call verified in a fresh dir under <dir>/target,
# which is gone now.
fresh_verify() {
    local line dir
    line="$(grep '^target-dir ' "$1/crates/cargo.log" 2> /dev/null || true)"
    dir="$(awk '{print $2}' <<< "$line")"
    if [[ "$(wc -l <<< "$line" | tr -d ' ')" == 1 && "$line" == *" fresh" &&
        "$(dirname "$dir")" == "$1/target" && "$(basename "$dir")" == release-verify.* && ! -e "$dir" ]]; then
        ok "$2 verify-packages in a fresh dir under the target dir, and removes it"
    else
        bad "$2 did not verify-package in a fresh dir under the target dir, removed after: ${line:-no --target-dir}$([[ -n "$dir" && -e "$dir" ]] && echo ', left behind')"
    fi
}
fresh_verify "$d" "release-prepare --dry-run"

d="$(fixture prepare-503)"
echo "status: 503" > "$d/crates/duet-dsp"
run "$d" release-prepare.sh --dry-run
if said "$d" "(a real run refuses here)" && ! said "$d" "duet-dsp: not on crates.io"; then
    ok "release-prepare refuses a crates.io 503, and does not read duet-dsp as new"
else
    bad "release-prepare on a crates.io 503: $(grep -h duet-dsp "$d/stdout" "$d/stderr" || true)"
fi

d="$(fixture prepare-version)"
echo 0.2.0 > "$d/work/VERSION"
git -C "$d/work" commit -q -am "VERSION by hand"
git -C "$d/work" push -q origin main
run "$d" release-prepare.sh --dry-run
if said "$d" "but VERSION says 0.2.0 (a real run refuses here)"; then
    ok "release-prepare refuses a VERSION that is not the last release"
else
    bad "release-prepare with VERSION 0.2.0 after v0.3.0: $(tail -n 3 "$d/stderr")"
fi

# 7. release.yml runs the check on the tagged candidate.
job="$(awk '/^  build-crates:/{p=1; print; next} p && /^  [a-z-]+:/{exit} p' "${RELEASE_WORKFLOW:-$root/.github/workflows/release.yml}")"
check_line="$(grep -n 'api-check.sh "\$VERSION"' <<< "$job" | head -n 1 | cut -d: -f1 || true)"
package_line="$(grep -n 'publish-crates.sh "\$VERSION" --out' <<< "$job" | head -n 1 | cut -d: -f1 || true)"
if [[ -n "$check_line" && -n "$package_line" && "$check_line" -lt "$package_line" ]] &&
    grep -q 'fetch-depth: 0' <<< "$job" &&
    grep -qE 'tool: cargo-semver-checks@[0-9]+\.[0-9]+\.[0-9]+$' <<< "$job"; then
    ok "release.yml#build-crates runs api-check.sh from a full clone, with a pinned cargo-semver-checks, before packaging"
else
    bad "release.yml#build-crates does not run api-check.sh before packaging, from a full clone, with a pinned cargo-semver-checks"
fi

# 8. Held crates.
# The library crates left when duet-bus and duet-actions (and duet-cli) are held, as the cases hold them.
UNHELD=(duet-core duet-midi duet-abc duet-drums duet-dsp duet-capture duet-play duet-schemas)
# set_held <dir> <crates...>: the scripts under test, in <dir>/work, hold exactly these crates.
set_held() {
    local dir="$1"
    shift
    sed -i.bak -E "s/^HELD=\(.*\)$/HELD=($*)/" "$dir/work/tools/scripts/release/lib/common.sh"
}
# manifest <dir> <crate> <line>: crates/<crate>/Cargo.toml in <dir>/work, with that dependency.
manifest() {
    mkdir -p "$1/work/crates/$2"
    printf '[package]\nname = "%s"\n\n[dependencies]\n%s\n' "$2" "$3" > "$1/work/crates/$2/Cargo.toml"
}

d="$(fixture held-api)"
tag_head "$d" 0.3.1
set_held "$d" duet-bus duet-actions duet-cli
run "$d" api-check.sh 0.3.1
if [[ "$(status "$d")" == 0 ]] && said "$d" "duet-bus: held back from this release" &&
    said "$d" "duet-actions: held back from this release" &&
    ! grep -qE -- '-p duet-(bus|actions) ' "$d/crates/cargo.log"; then
    ok "api-check names duet-bus and duet-actions as held, and does not judge them"
else
    bad "api-check judged a held crate, or did not name it: $(grep -E 'duet-(bus|actions)' "$d/stdout" "$d/crates/cargo.log" 2> /dev/null | head -n 3)"
fi

d="$(fixture held-dependent)"
tag_head "$d" 0.3.1
set_held "$d" duet-bus
manifest "$d" duet-actions "duet-bus.workspace = true"
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a published crate that depends on a held one is refused" "duet-actions is published by this release, but depends on duet-bus, which is held"
manifest "$d" duet-actions 'duet-bus = { path = "../duet-bus", version = "0.3.0" }'
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a versioned path dependency on a held crate is refused too" "depends on duet-bus, which is held"
manifest "$d" duet-actions 'duet-bus = { path = "../duet-bus" }'
run "$d" api-check.sh 0.3.1
expect_pass "$d" "a path-only dependency on a held crate, which cargo strips, passes"

d="$(fixture held-unknown)"
tag_head "$d" 0.3.1
set_held "$d" duet-nope
run "$d" api-check.sh 0.3.1
expect_refused "$d" "a HELD naming no crate is refused" "HELD names duet-nope, which is not one of CRATES"

# publish-crates.sh at 0.4.0, its versions written as update-versions.sh writes them, with a target
# dir holding a package for every crate (the fake cargo packages nothing).
publish_fixture() {
    local d crate
    d="$(fixture "$1")"
    {
        printf '[workspace.package]\nversion = "0.4.0"\n\n[workspace.dependencies]\n'
        for crate in "${LIBS[@]}"; do printf '%s = { path = "crates/%s", version = "0.4.0" }\n' "$crate" "$crate"; done
    } > "$d/work/Cargo.toml"
    mkdir -p "$d/target/package"
    for crate in "${LIBS[@]}" duet-cli; do echo "$crate" > "$d/target/package/$crate-0.4.0.crate"; done
    echo "$d"
}

d="$(publish_fixture held-build)"
set_held "$d" duet-bus duet-actions duet-cli
CARGO_TARGET_DIR="$d/target" run "$d" publish-crates.sh 0.4.0 --out "$d/dist"
line="$(grep '^package ' "$d/crates/cargo.log" 2> /dev/null || true)"
if [[ "$(status "$d")" == 0 ]] && [[ -n "$line" ]] && ! grep -qE -- '-p duet-(bus|actions|cli)( |$)' <<< "$line" &&
    grep -q -- '-p duet-schemas' <<< "$line" && [[ ! -e "$d/dist/duet-bus-0.4.0.crate" ]] &&
    [[ "$(find "$d/dist" -name '*.crate' | wc -l | tr -d ' ')" == "${#UNHELD[@]}" ]]; then
    ok "the build stage packages every crate but the held ones, and leaves them out of dist"
else
    bad "the build stage packaged a held crate, or not the rest: exit $(status "$d"), $line, dist: $(find "$d/dist" -name '*.crate' -exec basename {} \; 2> /dev/null | tr '\n' ' ')"
fi
fresh_verify "$d" "the build stage (publish-crates --out)"

d="$(publish_fixture held-publish)"
set_held "$d" duet-bus duet-actions duet-cli
for crate in "${UNHELD[@]}"; do echo "versions: 0.3.0 0.4.0" > "$d/crates/$crate"; done
rm -f "$d/crates/duet-bus" "$d/crates/duet-actions"
mkdir -p "$d/dist"
CARGO_TARGET_DIR="$d/target" run "$d" publish-crates.sh 0.4.0 --from "$d/dist" --dry-run
if [[ "$(status "$d")" == 0 ]] && said "$d" "duet-bus is held back from this release" &&
    ! grep -q '^publish' "$d/crates/cargo.log" 2> /dev/null; then
    ok "the publish stage skips the held crates, and finishes when the rest are on crates.io"
else
    bad "the publish stage tried a held crate: exit $(status "$d"): $(tail -n 2 "$d/stderr")"
fi

if [[ "$failures" -gt 0 ]]; then
    echo "$failures failed" >&2
    exit 1
fi
echo "all passed"
