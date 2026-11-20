#!/usr/bin/env bash
# tools/scripts/release/release-prepare.sh
#
# Prepare a release: compute the next shared version, check the bump against each published
# crate's Rust API, cut release/v<NEXT>, write every version site, verify-package every crate, and
# open the release PR. Merging that PR changes VERSION on main, which release-tag.yml tags and
# releases. Borrowed from temper's release-prepare.sh, cut down to duet's one lane.
#
# Usage:
#   ./tools/scripts/release/release-prepare.sh [--dry-run] [--minor | --major]
#
#   (default)  a patch bump: 0.3.0 -> 0.3.1
#   --minor    0.3.0 -> 0.4.0. Below 1.0, a minor is the breaking bump.
#   --major    0.3.0 -> 1.0.0
#   --dry-run  report the next version, the API check and the packaging; change nothing
#
# The API check is api-check.sh, the same check release.yml's build-crates runs on the tagged
# candidate: cargo-semver-checks on every library crate, against the last release, at the bump
# requested. A crate whose public API changed more than the bump allows refuses the release. The
# last release comes from last_release (lib/common.sh), and VERSION must name it, so the window
# this opens and the baseline the check reads are one derivation (COMPAT-2).

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

DRY_RUN=false
LEVEL="patch"
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --minor) LEVEL=minor; shift ;;
        --major) LEVEL=major; shift ;;
        *) die "unknown argument: $1" ;;
    esac
done
cd "$REPO_ROOT"

refuse() { if [[ "$DRY_RUN" == "true" ]]; then log_warn "$* (a real run refuses here)"; else die "$*"; fi; }

log_header "pre-flight"
git fetch --quiet origin main
[[ "$(git branch --show-current)" == "main" ]] || refuse "not on main"
[[ -z "$(git status --porcelain)" ]] || refuse "the working tree has changes"
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || refuse "main is not origin/main"
held_check
cargo semver-checks --version >/dev/null 2>&1 || die "cargo-semver-checks is not installed (cargo install cargo-semver-checks --locked)"
command -v gh >/dev/null || [[ "$DRY_RUN" == "true" ]] || die "gh is not installed"

if [[ -f VERSION ]]; then
    CURRENT="$("$(dirname "$0")/read-version.sh")"
    CURRENT="${CURRENT#VERSION=}"
else
    CURRENT="$(workspace_version)"
    log_info "no VERSION yet: this is the first release cut by this tooling; from [workspace.package] ${CURRENT}"
fi
NEXT="$(bump "$CURRENT" "$LEVEL")"
log_info "version: ${CURRENT} -> ${NEXT} (${LEVEL})"
git ls-remote --exit-code --tags origin "v${NEXT}" >/dev/null 2>&1 && die "the tag v${NEXT} already exists"
LAST="$(last_release "$NEXT")" || exit 1
if [[ "$LAST" == "none" ]]; then
    if [[ -f VERSION ]]; then refuse "VERSION says ${CURRENT}, but origin has no release tag in main's history"; fi
elif [[ ! -f VERSION || "$LAST" != "$CURRENT" ]]; then
    refuse "the last release is ${LAST} (origin's newest release tag in main's history), but VERSION says ${CURRENT}"
fi

API_LOG="$(mktemp)"
if "$(dirname "$0")/api-check.sh" "$NEXT" | tee "$API_LOG"; then
    :
else
    refuse "the API check refused ${CURRENT} -> ${NEXT}: re-run with --minor (below 1.0, the breaking bump) or --major"
fi
NEW_CRATES="$(sed -n 's/^new crates: //p' "$API_LOG")"
rm -f "$API_LOG"

if [[ "$DRY_RUN" == "true" ]]; then
    log_header "packaging (at ${CURRENT}; a real run packages at ${NEXT})"
    PKG_ARGS=()
    for crate in "${PUBLISHED[@]}"; do PKG_ARGS+=(-p "$crate"); done
    # --allow-dirty: a dry run may be run on an unfinished tree; a real run refuses one above.
    # A fresh target dir, so a second check at this version verifies today's sources, not the
    # first check's build of them (fresh_verify_dir in lib/common.sh says why); the real run gets
    # the same from publish-crates.sh --out.
    VERIFY_DIR="$(fresh_verify_dir "${CARGO_TARGET_DIR:-${REPO_ROOT}/target}")"
    trap 'rm -rf "$VERIFY_DIR"' EXIT
    cargo package --quiet --allow-dirty --target-dir "$VERIFY_DIR" "${PKG_ARGS[@]}"
    log_info "all ${#PUBLISHED[@]} crates this release publishes package and verify"
    log_header "dry run: nothing changed. A real run cuts release/v${NEXT} and opens its PR."
    exit 0
fi

BRANCH="release/v${NEXT}"
log_header "cut ${BRANCH}"
git checkout -b "$BRANCH"
"$(dirname "$0")/update-versions.sh" "$NEXT"
git add VERSION Cargo.toml Cargo.lock
git commit --quiet -m "chore(release): prepare v${NEXT}"
# Packaged from the commit, as release.yml packages the tag: cargo refuses an uncommitted tree.
"$(dirname "$0")/publish-crates.sh" "$NEXT" --out "${CARGO_TARGET_DIR:-target}/release-dist"
git push --quiet -u origin "$BRANCH"

NEW_LINE="none"
[[ -n "$NEW_CRATES" && "$NEW_CRATES" != "none" ]] && NEW_LINE="${NEW_CRATES} (first published at ${NEXT})"
HELD_LINE="none"
if [[ ${#HELD[@]} -gt 0 ]]; then
    HELD_LINE="${HELD[*]}: their version moves to ${NEXT} with the rest, but this release does not publish them (HELD in lib/common.sh). A new crate's first version is published by token after the release, from a branch that empties HELD (RELEASING.md)"
fi
BODY="$(mktemp)"
cat > "$BODY" <<BODY_EOF
## What

Release v${NEXT}: every duet crate moves from ${CURRENT} to ${NEXT}, with VERSION, [workspace.package] and [workspace.dependencies] together.

## Why

Merging changes VERSION on main, which release-tag.yml tags as v${NEXT} and releases: the crates to crates.io and a GitHub Release.

## How

- **Bump:** a ${LEVEL}. The API check (api-check.sh) passed against the last release ${CURRENT} for every library crate crates.io already has; release.yml runs it again on the tagged candidate.
- **New on crates.io:** ${NEW_LINE}.
- **Held back:** ${HELD_LINE}.
- **Formats:** none.
- **Verified:** every crate verify-packaged at ${NEXT} (\`tools/scripts/release/publish-crates.sh ${NEXT} --out\`).

## Next

After the merge, check the Release run, and the crates on crates.io at ${NEXT}.
BODY_EOF
gh pr create --base main --head "$BRANCH" --title "chore(release): prepare v${NEXT}" --body-file "$BODY"
rm -f "$BODY"
