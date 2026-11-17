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
# The API check is cargo-semver-checks, run on every library crate crates.io already has, against
# its latest published version, at the bump requested. A crate whose public API changed more than
# the bump allows refuses the release. A crate crates.io does not have yet has no baseline and is
# reported as new. duet-cli is the application: its contract is the command line, which duet-e2e
# tests, not its Rust API, so it is not checked.
#
# Below 1.0, cargo's rule is that 0.y.z -> 0.(y+1).0 may break and 0.y.z -> 0.y.(z+1) may only
# add, and cargo-semver-checks names those levels "major" and "minor". So a duet minor is checked
# as its "major", and a duet patch as its "minor".

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
command -v cargo-semver-checks >/dev/null || die "cargo-semver-checks is not installed (cargo install cargo-semver-checks --locked)"
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

CHECK_LEVEL="$LEVEL"
if [[ "${CURRENT%%.*}" == "0" ]]; then
    case "$LEVEL" in patch) CHECK_LEVEL="minor" ;; *) CHECK_LEVEL="major" ;; esac
fi
log_header "API check (cargo-semver-checks --release-type ${CHECK_LEVEL}, for a duet ${LEVEL} from ${CURRENT})"
NEW_CRATES=()
FAILED=()
for crate in "${CRATES[@]}"; do
    [[ "$crate" == "duet-cli" ]] && continue
    if ! crate_exists "$crate"; then
        log_info "${crate}: not on crates.io yet; its first version is ${NEXT}"
        NEW_CRATES+=("$crate")
        continue
    fi
    if cargo semver-checks check-release -p "$crate" --release-type "$CHECK_LEVEL" >/dev/null 2>&1; then
        log_info "${crate}: a ${LEVEL} is enough"
    else
        log_warn "${crate}: its API changed more than a ${LEVEL} allows (cargo semver-checks check-release -p ${crate})"
        FAILED+=("$crate")
    fi
done
if [[ ${#FAILED[@]} -gt 0 ]]; then
    refuse "${FAILED[*]} need a larger bump than ${LEVEL}: re-run with --minor (below 1.0, the breaking bump) or --major"
fi

if [[ "$DRY_RUN" == "true" ]]; then
    log_header "packaging (at ${CURRENT}; a real run packages at ${NEXT})"
    PKG_ARGS=()
    for crate in "${CRATES[@]}"; do PKG_ARGS+=(-p "$crate"); done
    # --allow-dirty: a dry run may be run on an unfinished tree; a real run refuses one above.
    cargo package --quiet --allow-dirty "${PKG_ARGS[@]}"
    log_info "all ${#CRATES[@]} crates package and verify"
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
[[ ${#NEW_CRATES[@]} -gt 0 ]] && NEW_LINE="${NEW_CRATES[*]} (first published at ${NEXT})"
BODY="$(mktemp)"
cat > "$BODY" <<BODY_EOF
## What

Release v${NEXT}: every published duet crate moves from ${CURRENT} to ${NEXT}, with VERSION, [workspace.package] and [workspace.dependencies] together.

## Why

Merging changes VERSION on main, which release-tag.yml tags as v${NEXT} and releases: the crates to crates.io and a GitHub Release.

## How

- **Bump:** a ${LEVEL}. cargo-semver-checks passed at that level for every crate crates.io already has.
- **New on crates.io:** ${NEW_LINE}.
- **Formats:** none.
- **Verified:** every crate verify-packaged at ${NEXT} (\`tools/scripts/release/publish-crates.sh ${NEXT} --out\`).

## Next

After the merge, check the Release run, and the crates on crates.io at ${NEXT}.
BODY_EOF
gh pr create --base main --head "$BRANCH" --title "chore(release): prepare v${NEXT}" --body-file "$BODY"
rm -f "$BODY"
