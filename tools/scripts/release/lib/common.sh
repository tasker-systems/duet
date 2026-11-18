#!/usr/bin/env bash
# tools/scripts/release/lib/common.sh
#
# Shared by duet's release scripts: the published crates in publish order, the shared version, the
# crates.io probe and portable helpers. Borrowed from temper's tools/scripts/release/lib/common.sh
# and cut down to one lane: duet publishes Rust crates only.
#
# Source it: source "$(dirname "$0")/lib/common.sh"

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

# Every published crate, in publish order. Not style: a crate's packed manifest resolves its
# dependencies from the registry (cargo keeps the version and drops the path), so a crate's
# workspace dependencies, and its dev-dependencies that carry a version, must be published first.
# duet-e2e is `publish = false`. The two dev-dependency cycles back to duet-schemas (from duet-midi
# and duet-drums) are path-only, so cargo strips them and they constrain nothing here.
CRATES=(duet-core duet-midi duet-abc duet-drums duet-dsp duet-capture duet-play duet-schemas duet-bus duet-actions duet-cli)

SEMVER='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'

# crates.io asks automated clients to identify themselves; no personal detail goes in it.
USER_AGENT="duet-release (github.com/tasker-systems/duet)"

log_info()   { echo "  [info] $*"; }
log_warn()   { echo "  [warn] $*" >&2; }
log_header() { echo ""; echo "== $* =="; echo ""; }
die()        { echo "  [error] $*" >&2; exit 1; }

sed_i() {
    if sed --version 2>/dev/null | grep -q 'GNU'; then sed -i "$@"; else sed -i '' "$@"; fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# [workspace.package].version in the root Cargo.toml: what every published crate inherits.
workspace_version() {
    awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' \
        "${REPO_ROOT}/Cargo.toml"
}

# A [workspace.dependencies] entry's version: `duet-core = { path = ..., version = "X" }`.
workspace_dep_version() {
    sed -n -E "s/^$1 = \\{.*version = \"([^\"]+)\".*/\\1/p" "${REPO_ROOT}/Cargo.toml"
}

# True when crates.io lists $1 at version $2. Its API answers unauthenticated.
crate_published() {
    curl -sf -A "$USER_AGENT" "https://crates.io/api/v1/crates/$1/versions" 2>/dev/null |
        grep -q "\"num\":\"$2\""
}

# True when crates.io has the crate at all (any version).
crate_exists() {
    curl -sf -o /dev/null -A "$USER_AGENT" "https://crates.io/api/v1/crates/$1"
}

bump() {
    local version="$1" level="$2" major minor patch
    IFS='.' read -r major minor patch <<< "${version%%[-+]*}"
    case "$level" in
        patch) echo "${major}.${minor}.$((patch + 1))" ;;
        minor) echo "${major}.$((minor + 1)).0" ;;
        major) echo "$((major + 1)).0.0" ;;
        *) die "unknown bump level: ${level}" ;;
    esac
}
