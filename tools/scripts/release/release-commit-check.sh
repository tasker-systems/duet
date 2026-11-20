#!/usr/bin/env bash
# tools/scripts/release/release-commit-check.sh TAG --event EVENT
#
# Refuses a release whose tag is not on main, and, for every EVENT but `push`, one whose commit did
# not pass CI on main. EVENT is the run's github.event_name. release.yml#determine-version runs it
# before anything is built, so a refusal publishes nothing and mints no crates.io credential:
# publish-crates needs determine-version.
#
# Why both, and why a push is spared the CI question (RELEASING.md, "Authentication"):
#   - The release-tag.yml door tags the commit a push to main just landed, and ruleset 27260518
#     (a PR and a strict CI Success, no bypass) is what let it land. CI's own run on that push is
#     still going when release-tag.yml calls release.yml, so it cannot be asked for yet.
#     (Inside a workflow_call, github.event_name is the caller's: `push`.)
#   - The workflow_dispatch door takes any tag. A tag on a commit no PR merged, or a tag on a
#     commit that is on main but whose CI failed, would otherwise publish. So for any event but
#     `push`, it asks GitHub for a successful run of ci.yml on a push to main at the tag's commit.
#
# Everything it cannot decide refuses: a shallow clone, no origin/main in the clone, a tag that is
# not there, an API answer it cannot read.
#
# Needs a full clone with origin/main (actions/checkout with fetch-depth: 0). For any event but
# `push` it also needs GITHUB_REPOSITORY and a GH_TOKEN that may read Actions (actions: read).

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

usage() { echo "usage: release-commit-check.sh v<VERSION> --event <github.event_name>" >&2; exit 2; }

TAG="" EVENT=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --event) [[ $# -ge 2 && -n "$2" ]] || usage; EVENT="$2"; shift 2 ;;
        -*) usage ;;
        *) [[ -z "$TAG" ]] || usage; TAG="$1"; shift ;;
    esac
done
[[ -n "$TAG" && -n "$EVENT" ]] || usage
[[ "${TAG#v}" != "$TAG" && "${TAG#v}" =~ $SEMVER ]] || die "the tag '${TAG}' is not v<semver>"

[[ "$(git rev-parse --is-shallow-repository)" == "false" ]] ||
    die "this clone is shallow: whether ${TAG} is on main cannot be decided"
main_sha="$(git rev-parse --verify --quiet 'refs/remotes/origin/main^{commit}')" ||
    die "origin/main is not in this clone: whether ${TAG} is on main cannot be decided"
sha="$(git rev-parse --verify --quiet "refs/tags/${TAG}^{commit}")" ||
    die "there is no tag ${TAG} in this clone"

git merge-base --is-ancestor "$sha" "$main_sha" ||
    die "${TAG} (${sha}) is not on main: a release publishes only a commit main has"
log_info "${TAG} (${sha}) is on main"

if [[ "$EVENT" == push ]]; then
    log_info "a push to main (the release-tag.yml door): its CI is the ruleset's, not asked here"
else
    [[ -n "${GITHUB_REPOSITORY:-}" ]] || die "GITHUB_REPOSITORY is not set: CI on main cannot be asked about"
    runs="$(gh api "repos/${GITHUB_REPOSITORY}/actions/workflows/ci.yml/runs?head_sha=${sha}&event=push&branch=main&per_page=100")" ||
        die "could not ask GitHub for CI's runs on ${sha}"
    passed="$(jq --arg sha "$sha" '[.workflow_runs[]
        | select(.path == ".github/workflows/ci.yml" and .head_sha == $sha and .event == "push"
                 and .head_branch == "main" and .status == "completed" and .conclusion == "success")]
        | length' <<< "$runs")" ||
        die "GitHub's answer about CI's runs on ${sha} could not be read"
    [[ "$passed" =~ ^[0-9]+$ && "$passed" -gt 0 ]] ||
        die "${TAG} (${sha}) has no successful CI run on main: a release publishes only a commit that passed CI there"
    log_info "${TAG} (${sha}) passed CI on main"
fi
