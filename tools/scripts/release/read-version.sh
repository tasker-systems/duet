#!/usr/bin/env bash
# tools/scripts/release/read-version.sh
#
# Print the released version as `VERSION=<semver>`, for eval and for $GITHUB_OUTPUT: the repo-root
# VERSION file, which the first release PR creates and every later one bumps.
#
# The value becomes a step output, a tag name and an argument in a job that pushes a tag, so
# anything that is not a semver version is refused before it is written anywhere: whitespace
# stripping alone would pass `0.3.0$(id)` straight through.

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

VERSION_FILE="${REPO_ROOT}/VERSION"
[[ -f "$VERSION_FILE" ]] || die "VERSION not found at ${VERSION_FILE}"
VERSION="$(tr -d '[:space:]' < "$VERSION_FILE")"
[[ "$VERSION" =~ $SEMVER ]] || die "VERSION '${VERSION}' is not a semver version (MAJOR.MINOR.PATCH[-pre][+build])"
echo "VERSION=${VERSION}"
