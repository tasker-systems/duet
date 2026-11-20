#!/usr/bin/env bash
# The release-commit check's bite test (TW-5): a hand-started release.yml publishes only a tag on
# main whose commit passed CI there. Real git over fixture repositories, and a fake gh
# (fixture/bin/gh) for GitHub's list of ci.yml's runs. Each refusal is shown biting, and its
# sanctioned form passing:
#   1. a dispatch of a tag on main whose commit passed CI on main passes;
#   2. a dispatch of a tag off main is refused, even when a successful run is listed for it;
#   3. a dispatch of a tag on main whose CI failed, was never run, or ran only for another event,
#      branch or workflow file is refused;
#   4. a GitHub answer it cannot get or read refuses;
#   5. the release-tag.yml door (event push) is not asked about CI, but its tag must be on main;
#      any other event is asked;
#   6. what it cannot decide refuses: a shallow clone, no origin/main, no such tag, a tag that is
#      not v<semver>, no event;
#   7. release.yml runs it in determine-version, from a full clone, with the run's event and an
#      Actions-reading token, before build-crates and publish-crates, and publish-crates runs in
#      the `release` environment; release-tag.yml#release grants the actions: read it needs.
#
# NOT CHECKED: GitHub's real API, the `release` environment's deployment rule, and crates.io's
# trusted publishers naming it. Those are settings, and nothing in the repo reads them
# (RELEASING.md, "Authentication").
#
# RELEASE_SCRIPTS=<dir> runs the cases against another copy of tools/scripts/release, such as
# main's, to show they fail without this change; RELEASE_WORKFLOW=<file> and
# RELEASE_TAG_WORKFLOW=<file> another release.yml and release-tag.yml. CI runs this in
# .github/workflows/ci.yml#gate.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
scripts="${RELEASE_SCRIPTS:-$here}"
release_yml="${RELEASE_WORKFLOW:-$root/.github/workflows/release.yml}"
release_tag_yml="${RELEASE_TAG_WORKFLOW:-$root/.github/workflows/release-tag.yml}"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/release-commit-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
export PATH="$here/fixture/bin:$PATH"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
export GITHUB_REPOSITORY=example/duet

failures=0
ok() { echo "ok   $1"; }
bad() {
    echo "FAIL $1" >&2
    failures=$((failures + 1))
}

# A fixture: an origin whose main has c1 then c2, tagged v0.4.0 at c2; a branch `side` off c1 with
# its own commit, tagged v0.5.0 and pushed as a tag only; and a full clone of origin at <dir>/work,
# as release.yml's checkout with fetch-depth: 0 makes it.
# fixture <name> → prints <dir>
fixture() {
    local dir="$scratch/$1"
    mkdir -p "$dir"
    git init -q --bare -b main "$dir/origin.git"
    git init -q -b main "$dir/seed"
    mkdir -p "$dir/seed/tools/scripts"
    cp -R "$scripts" "$dir/seed/tools/scripts/release"
    rm -rf "$dir/seed/tools/scripts/release/fixture" "$dir/seed/tools/scripts/release/"test-*.sh
    echo 0.4.0 > "$dir/seed/VERSION"
    git -C "$dir/seed" add -A
    git -C "$dir/seed" commit -q -m c1
    echo two > "$dir/seed/change"
    git -C "$dir/seed" add -A
    git -C "$dir/seed" commit -q -m c2
    git -C "$dir/seed" tag -a v0.4.0 -m v0.4.0
    git -C "$dir/seed" checkout -q -b side HEAD~1
    echo 0.5.0 > "$dir/seed/VERSION"
    git -C "$dir/seed" commit -q -am "side: never merged"
    git -C "$dir/seed" tag -a v0.5.0 -m v0.5.0
    git -C "$dir/seed" push -q "$dir/origin.git" main v0.4.0 v0.5.0
    git clone -q "file://$dir/origin.git" "$dir/work"
    echo "$dir"
}
# The commit a tag names in <dir>/work: sha_of <dir> <tag>
sha_of() { git -C "$1/work" rev-parse "$2^{commit}"; }
# Run the check in <dir>/work: run <dir> [args...]; output lands in <dir>.
run() {
    local dir="$1" status=0
    shift
    if [[ ! -e "$dir/work/tools/scripts/release/release-commit-check.sh" ]]; then
        echo "no release-commit-check.sh in these release scripts" > "$dir/stderr"
        : > "$dir/stdout"
        echo 127 > "$dir/status"
        return 0
    fi
    (cd "$dir/work" && FAKE_GH_RUNS="$dir/runs" bash tools/scripts/release/release-commit-check.sh "$@") \
        > "$dir/stdout" 2> "$dir/stderr" || status=$?
    echo "$status" > "$dir/status"
}
status() { cat "$1/status"; }
said() { grep -qF -- "$2" "$1/stdout" "$1/stderr"; }
asked() { [[ -s "$1/runs.log" ]]; }
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

# 1. The sanctioned dispatch.
d="$(fixture dispatch-passed)"
echo "$(sha_of "$d" v0.4.0) success" > "$d/runs"
run "$d" v0.4.0 --event workflow_dispatch
expect_pass "$d" "a dispatch of a tag on main whose commit passed CI on main passes"
if asked "$d" && grep -qF "repos/example/duet/actions/workflows/ci.yml/runs?head_sha=$(sha_of "$d" v0.4.0)&event=push&branch=main" "$d/runs.log"; then
    ok "it asks GitHub for ci.yml's push runs on main at the tag's commit"
else
    bad "it did not ask for ci.yml's push runs on main at the tag's commit: $(cat "$d/runs.log" 2> /dev/null)"
fi

# 2. A tag off main.
d="$(fixture dispatch-off-main)"
echo "$(sha_of "$d" v0.5.0) success" > "$d/runs"
run "$d" v0.5.0 --event workflow_dispatch
expect_refused "$d" "a dispatch of a tag off main is refused, even with a successful run listed for it" "is not on main"

# 3. On main, but CI did not pass there.
for runs in "failure" "cancelled" "none" "success pull_request" "success push side" \
    "success push main .github/workflows/other.yml" "other-sha"; do
    d="$(fixture "dispatch-ci-$RANDOM")"
    sha="$(sha_of "$d" v0.4.0)"
    case "$runs" in
        none) ;;
        other-sha) echo "$(sha_of "$d" v0.5.0) success" > "$d/runs" ;;
        *) echo "$sha $runs" > "$d/runs" ;;
    esac
    run "$d" v0.4.0 --event workflow_dispatch
    expect_refused "$d" "a dispatch of a tag on main whose CI runs are '$runs' is refused" "has no successful CI run on main"
done

# 4. GitHub answers it cannot use.
d="$(fixture gh-fails)"
echo fail > "$d/runs"
run "$d" v0.4.0 --event workflow_dispatch
expect_refused "$d" "GitHub not answering refuses" "could not ask GitHub"
d="$(fixture gh-garbage)"
echo "body: <html>unicorn</html>" > "$d/runs"
run "$d" v0.4.0 --event workflow_dispatch
expect_refused "$d" "an answer that is not a runs list refuses" "could not be read"

# 5. The release-tag.yml door, and any other event.
d="$(fixture push-on-main)"
run "$d" v0.4.0 --event push
expect_pass "$d" "the release-tag.yml door (push) with the tag on main passes"
if ! asked "$d"; then ok "the push door is not asked about CI"; else bad "the push door asked about CI"; fi
d="$(fixture push-off-main)"
run "$d" v0.5.0 --event push
expect_refused "$d" "the push door with a tag off main is refused" "is not on main"
d="$(fixture other-event)"
run "$d" v0.4.0 --event schedule
expect_refused "$d" "any event but push is asked about CI, and refused with no run" "has no successful CI run on main"

# 6. What it cannot decide.
d="$(fixture shallow)"
rm -rf "$d/work"
git clone -q --depth 1 --no-single-branch "file://$d/origin.git" "$d/work"
git -C "$d/work" fetch -q --depth 1 origin tag v0.4.0
echo "$(sha_of "$d" v0.4.0) success" > "$d/runs"
run "$d" v0.4.0 --event workflow_dispatch
expect_refused "$d" "a shallow clone is refused" "shallow"
d="$(fixture no-origin-main)"
git -C "$d/work" update-ref -d refs/remotes/origin/main
run "$d" v0.4.0 --event workflow_dispatch
expect_refused "$d" "a clone with no origin/main is refused" "origin/main is not in this clone"
d="$(fixture no-tag)"
run "$d" v0.9.0 --event workflow_dispatch
expect_refused "$d" "a tag not in the clone is refused" "there is no tag v0.9.0"
d="$(fixture bad-tag)"
run "$d" main --event workflow_dispatch
expect_refused "$d" "a tag that is not v<semver> is refused" "is not v<semver>"
d="$(fixture no-event)"
run "$d" v0.4.0
expect_refused "$d" "no event is refused" "usage"

# 7. The workflows.
job() { awk -v j="  $2:" '$0 == j {p=1; print; next} p && /^  [a-z-]+:/{exit} p' "$1"; }
determine="$(job "$release_yml" determine-version)"
build="$(job "$release_yml" build-crates)"
publish="$(job "$release_yml" publish-crates)"
check_line="$(grep -n 'release-commit-check.sh "\$TAG" --event "\$EVENT"' <<< "$determine" | head -n 1 | cut -d: -f1 || true)"
if [[ -n "$check_line" ]] && grep -q 'fetch-depth: 0' <<< "$determine" &&
    grep -qF 'EVENT: ${{ github.event_name }}' <<< "$determine" &&
    grep -qF 'GH_TOKEN: ${{ github.token }}' <<< "$determine" &&
    grep -qE '^      actions: read$' <<< "$determine"; then
    ok "release.yml#determine-version runs the check from a full clone, with the run's event and an Actions-reading token"
else
    bad "release.yml#determine-version does not run release-commit-check.sh with the run's event, a full clone and actions: read"
fi
if grep -qE '^    needs: (determine-version|\[determine-version.*\])$' <<< "$build" &&
    grep -qE '^    needs: \[determine-version, build-crates\]$' <<< "$publish"; then
    ok "build-crates and publish-crates need determine-version, so a refusal builds and publishes nothing"
else
    bad "build-crates or publish-crates does not need determine-version"
fi
if grep -qE '^    environment: release$' <<< "$publish" && grep -qE '^      id-token: write$' <<< "$publish"; then
    ok "release.yml#publish-crates, the job holding id-token: write, runs in the release environment"
else
    bad "release.yml#publish-crates does not run in the release environment"
fi
caller="$(job "$release_tag_yml" release)"
if grep -qE '^      actions: read$' <<< "$caller"; then
    ok "release-tag.yml#release grants the actions: read determine-version asks for"
else
    bad "release-tag.yml#release does not grant actions: read, so the call to release.yml cannot start"
fi

if [[ "$failures" -gt 0 ]]; then
    echo "$failures failed" >&2
    exit 1
fi
echo "all passed"
