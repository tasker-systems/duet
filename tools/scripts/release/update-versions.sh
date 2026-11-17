#!/usr/bin/env bash
# tools/scripts/release/update-versions.sh
#
# The one writer of duet's version sites, so they cannot drift apart:
#   - the repo-root VERSION file (what release-tag.yml tags);
#   - [workspace.package].version (what every published crate inherits);
#   - each duet-* entry in [workspace.dependencies] (a dependency spec cannot inherit
#     [workspace.package], so each is spelled out);
#   - Cargo.lock's record of the workspace's own packages.
#
# Usage: ./tools/scripts/release/update-versions.sh NEW_VERSION

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

NEW="${1:-}"
[[ "$NEW" =~ $SEMVER ]] || die "usage: $0 NEW_VERSION (a semver version), got '${NEW}'"
cd "$REPO_ROOT"

OLD="$(workspace_version)"
[[ -n "$OLD" ]] || die "no [workspace.package] version in Cargo.toml"

printf '%s\n' "$NEW" > VERSION
# Only the version line inside [workspace.package].
awk -v new="$NEW" '
    /^\[workspace\.package\]/ { p = 1; print; next }
    /^\[/ { p = 0 }
    p && /^version = / { print "version = \"" new "\""; p = 0; next }
    { print }
' Cargo.toml > Cargo.toml.next && mv Cargo.toml.next Cargo.toml
for crate in "${CRATES[@]}"; do
    [[ "$crate" == "duet-cli" ]] && continue   # an application: nothing depends on it
    [[ -n "$(workspace_dep_version "$crate")" ]] || die "${crate} has no [workspace.dependencies] entry"
    sed_i -E "s/^(${crate} = \\{ path = \"crates\\/${crate}\", version = )\"[^\"]+\"/\\1\"${NEW}\"/" Cargo.toml
done
cargo update --workspace --quiet

# Every site must now read NEW, or the release is refused here rather than at publish.
[[ "$(workspace_version)" == "$NEW" ]] || die "[workspace.package] version did not move to ${NEW}"
for crate in "${CRATES[@]}"; do
    [[ "$crate" == "duet-cli" ]] && continue
    [[ "$(workspace_dep_version "$crate")" == "$NEW" ]] || die "${crate}'s [workspace.dependencies] version did not move to ${NEW}"
done
log_info "versions: ${OLD} -> ${NEW} (VERSION, [workspace.package], ${#CRATES[@]} crates, Cargo.lock)"
