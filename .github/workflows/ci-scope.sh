#!/usr/bin/env bash
set -euo pipefail

# Change-scope detection for ci.yml (CI-2). Prints one word on stdout, `docs` or `all`, and why on
# stderr. `docs` means every changed path is on the allowlist below, so ci.yml may skip the jobs
# that compile. Anything else is `all`.
#
# It fails closed. ci.yml reads a docs-only decision only from a `docs` line on stdout and a zero
# exit; an error, a missing commit or an unexpected event is `all`. So is an empty diff, a rename,
# a copy, a type change, and any path not on the allowlist, which includes this script and every
# workflow: a change to the detector runs everything.
#
#   ci-scope.sh <event> <base-sha> <head-sha>
#
# pull_request diffs the PR's base and head from their merge base; merge_group diffs the queue's
# base_sha (CI-5). Any other event, push to main included, runs everything.

all() {
  echo "scope: all ($1)" >&2
  echo all
  exit 0
}

event="${1:-}"
base="${2:-}"
head="${3:-}"

case "$event" in
  pull_request | merge_group) ;;
  *) all "event '$event' has no change-scope base" ;;
esac

sha='^[0-9a-f]{40}$'
[[ "$base" =~ $sha ]] || all "base '$base' is not a commit SHA"
[[ "$head" =~ $sha ]] || all "head '$head' is not a commit SHA"
git cat-file -e "${base}^{commit}" 2> /dev/null || all "base $base is not in the checkout"
git cat-file -e "${head}^{commit}" 2> /dev/null || all "head $head is not in the checkout"
git merge-base "$base" "$head" > /dev/null || all "base and head share no history"

# Docs-only paths. Narrow on purpose: a path is docs only if nothing that compiles, tests, packages
# or checks conventions reads it. CLAUDE.md, .claude/ and .conventions/ are read by the conventions
# check, so they are code. README.md is each crate's `readme`, packaged but never compiled or
# tested. A new top-level file is code until it is added here.
is_docs() {
  case "$1" in
    docs/*) return 0 ;;
    README.md | RELEASING.md | SECURITY.md | LICENSE) return 0 ;;
    *) return 1 ;;
  esac
}

diff_out="$(mktemp)"
trap 'rm -f "$diff_out"' EXIT
# --find-renames so that a rename shows as one, and is refused below. -z keeps odd paths whole.
git diff --name-status -z --find-renames "${base}...${head}" > "$diff_out" \
  || all "git diff failed"

count=0
while IFS= read -r -d '' status; do
  IFS= read -r -d '' path || all "git diff output ended mid-record"
  case "$status" in
    A | M | D) ;;
    *) all "status '$status' for '$path' (a rename, copy or type change)" ;;
  esac
  is_docs "$path" || all "'$path' is not docs"
  count=$((count + 1))
done < "$diff_out"

[ "$count" -gt 0 ] || all "the diff is empty"
echo "scope: docs ($count paths, all docs)" >&2
echo docs
