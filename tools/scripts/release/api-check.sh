#!/usr/bin/env bash
# tools/scripts/release/api-check.sh
#
# The pre-publish API check (COMPAT-1, COMPAT-2): judge the tree at HEAD, as version CANDIDATE,
# against the last release, and refuse a break the bump from that release to CANDIDATE does not
# admit. release.yml's build-crates runs it on the tagged candidate before anything is packaged,
# so it judges the very sources it publishes; release-prepare.sh runs it before cutting the
# release branch.
#
# Usage: ./tools/scripts/release/api-check.sh CANDIDATE
#
# - The last release comes from last_release (lib/common.sh), the derivation release-prepare.sh
#   opens the window with. It is passed to cargo-semver-checks as --baseline-version, so the
#   comparator never looks for a baseline of its own, and what it reads is the published copy on
#   crates.io, not the tree at the tag.
# - The bump is the version class duet declares: release_type (lib/common.sh) turns the last
#   release and CANDIDATE into cargo-semver-checks' --release-type. duet declares only a class,
#   so the check cannot tell one break from another.
# - Each library crate is judged; duet-cli is the application, whose contract is the command line
#   (duet-e2e tests it), so it is not.
# - A crate is new, and has nothing to be judged against, only on a definite answer: crates.io
#   answers 404 for it, or lists it at CANDIDATE alone (its first version was published ahead of
#   this run, RELEASING.md's bootstrap). A crates.io answer that is neither a 404 nor a versions
#   list ends the check (crate_lookup). A crate crates.io lists, but not at the last release, or
#   with no release tag to judge it against, is refused.
#
# NOT CHECKED: breaks a crate's Rust API cannot show (behaviour, the formats' text, the command
# line), and anything cargo-semver-checks has no lint for. A crate already published at CANDIDATE
# is judged too, but the check cannot unpublish it.

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

CANDIDATE="${1:-}"
[[ $# -eq 1 && "$CANDIDATE" =~ $SEMVER ]] || die "usage: $0 CANDIDATE (a semver version)"
cd "$REPO_ROOT"
cargo semver-checks --version >/dev/null 2>&1 ||
    die "cargo-semver-checks is not installed (cargo install cargo-semver-checks --locked)"

LAST="$(last_release "$CANDIDATE")" || exit 1
if [[ "$LAST" == "none" ]]; then
    log_header "API check for ${CANDIDATE}: origin has no release tag in HEAD's history"
else
    TYPE="$(release_type "$LAST" "$CANDIDATE")" || exit 1
    log_header "API check for ${CANDIDATE} against the last release ${LAST} (cargo-semver-checks --release-type ${TYPE})"
fi

NEW=()
FAILED=()
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT
for crate in "${CRATES[@]}"; do
    [[ "$crate" == "duet-cli" ]] && continue
    if ! crate_lookup "$crate"; then
        log_info "${crate}: not on crates.io (404); new, first published at ${CANDIDATE}"
        NEW+=("$crate")
        continue
    fi
    if [[ "$CRATE_VERSIONS" == "$CANDIDATE" ]]; then
        log_info "${crate}: crates.io lists it at ${CANDIDATE} only; new, first published at ${CANDIDATE}"
        NEW+=("$crate")
        continue
    fi
    if [[ "$LAST" == "none" ]]; then
        log_warn "${crate}: crates.io lists it ($(echo "$CRATE_VERSIONS" | tr '\n' ' ')), but no release tag precedes ${CANDIDATE} to judge it against"
        FAILED+=("$crate")
        continue
    fi
    if ! grep -qxF "$LAST" <<< "$CRATE_VERSIONS"; then
        log_warn "${crate}: crates.io lists it ($(echo "$CRATE_VERSIONS" | tr '\n' ' ')), but not at the last release ${LAST}"
        FAILED+=("$crate")
        continue
    fi
    if cargo semver-checks check-release -p "$crate" --baseline-version "$LAST" --release-type "$TYPE" \
        >"$OUT" 2>&1; then
        log_info "${crate}: the bump ${LAST} -> ${CANDIDATE} admits its API change"
    else
        cat "$OUT" >&2
        log_warn "${crate}: its API changed more than ${LAST} -> ${CANDIDATE} admits, or could not be judged"
        FAILED+=("$crate")
    fi
done

if [[ ${#NEW[@]} -gt 0 ]]; then echo "new crates: ${NEW[*]}"; else echo "new crates: none"; fi
[[ ${#FAILED[@]} -eq 0 ]] ||
    die "refused: ${FAILED[*]} (below 1.0 a minor is the breaking bump; re-run release-prepare with --minor or --major)"
if [[ "$LAST" == "none" ]]; then log_info "every library crate is new"; else log_info "every library crate passes against ${LAST}"; fi
