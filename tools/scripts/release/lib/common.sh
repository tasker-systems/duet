#!/usr/bin/env bash
# tools/scripts/release/lib/common.sh
#
# Shared by duet's release scripts: the published crates in publish order and those held back, the shared version, the
# crates.io probe, the last release's one derivation, and portable helpers. Borrowed from temper's tools/scripts/release/lib/common.sh
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

# Crates held back from this release: they stay in CRATES, so update-versions.sh still moves their
# version with the rest, but the API check, the packaging and the publish leave them out.
#
# Why a crate is held: crates.io takes a crate's first version only from an API token, and attaches
# a trusted publisher only to a crate that already exists, while every crate already published is
# trusted-publishing-only, so no token can publish it. A new crate whose dependencies are not yet
# on crates.io at the shared version can therefore go out by neither door in one release. It is
# held, the release publishes the rest through CI, and its first version is then published by
# token from a branch that empties this list (RELEASING.md, "A crate's first version").
#
# A crate that depends on a held crate is held too: it cannot be packaged until the held one is
# on crates.io. held_check refuses a release that publishes such a crate.
#
# Empty: duet-bus and duet-actions were held from v0.4.0 and claimed by token after it, with
# duet-cli, which depends on both, held with them; the next release publishes every crate.
HELD=()

# CRATES without HELD, in publish order: what the API check, the packaging and the publish act on.
PUBLISHED=()
for _crate in "${CRATES[@]}"; do
    _held=false
    for _h in ${HELD[@]+"${HELD[@]}"}; do [[ "$_h" == "$_crate" ]] && _held=true; done
    [[ "$_held" == "true" ]] || PUBLISHED+=("$_crate")
done
unset _crate _held _h

# Refuse when HELD names a crate CRATES does not, or when a crate this release publishes depends on
# a held crate through a dependency that carries a version (a `.workspace = true` entry, or one with
# `version =`): cargo keeps those in the packed manifest, so its package resolves the held crate
# from crates.io and cannot be built. A path-only dev-dependency is stripped, and constrains nothing.
held_check() {
    local h c found manifest
    for h in ${HELD[@]+"${HELD[@]}"}; do
        found=false
        for c in "${CRATES[@]}"; do [[ "$c" == "$h" ]] && found=true; done
        [[ "$found" == "true" ]] || die "HELD names ${h}, which is not one of CRATES"
    done
    for c in "${PUBLISHED[@]}"; do
        manifest="${REPO_ROOT}/crates/${c}/Cargo.toml"
        [[ -f "$manifest" ]] || continue
        for h in ${HELD[@]+"${HELD[@]}"}; do
            if grep -E "^${h}(\.workspace[[:space:]]*=|[[:space:]]*=)" "$manifest" |
                grep -qE 'workspace|version'; then
                die "${c} is published by this release, but depends on ${h}, which is held; hold ${c} too"
            fi
        done
    done
}

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

# Ask crates.io for crate $1's versions. Returns 0 when crates.io has the crate, with every version
# it lists in CRATE_VERSIONS (one per line), and 1 only on a definite 404, the one answer that
# means "not published". Any other answer (a network error, a 5xx, a rate limit after curl's own
# retries, a body that is not a versions list) ends the script: "I could not look" is not
# "nothing shipped" (COMPAT-2). Its API answers unauthenticated.
#
# Call it directly (`if crate_lookup ...`), never inside `$(...)`: there `die` would end only the
# subshell, and the refusal would read as an absence.
CRATE_VERSIONS=""
crate_lookup() {
    local crate="$1" body code status=0
    CRATE_VERSIONS=""
    body="$(mktemp)"
    code="$(curl -s --retry 3 --retry-delay 2 -o "$body" -w '%{http_code}' -A "$USER_AGENT" \
        "https://crates.io/api/v1/crates/${crate}/versions")" || status=$?
    if [[ "$status" -ne 0 ]]; then
        rm -f "$body"
        die "could not ask crates.io about ${crate} (curl exit ${status}); refusing to read that as not published"
    fi
    case "$code" in
        200)
            if ! CRATE_VERSIONS="$(jq -r '.versions | if type == "array" and length > 0
                    then .[].num else error("no versions") end' "$body" 2>/dev/null)"; then
                rm -f "$body"
                die "crates.io answered 200 for ${crate} without a versions list; refusing to guess"
            fi
            rm -f "$body"
            return 0 ;;
        404)
            rm -f "$body"
            return 1 ;;
        *)
            rm -f "$body"
            die "crates.io answered HTTP ${code} for ${crate}; only a 404 means not published" ;;
    esac
}

# True when crates.io lists $1 at version $2. Fails closed through crate_lookup.
crate_published() {
    crate_lookup "$1" || return 1
    grep -qxF "$2" <<< "$CRATE_VERSIONS"
}

# True when crates.io has the crate at all (any version); false only on a definite 404.
crate_exists() {
    crate_lookup "$1"
}

# The last release before version $1: the one derivation of it (COMPAT-2), used by
# release-prepare.sh to open the window and by api-check.sh as cargo-semver-checks' baseline.
# Prints the version, or `none` when origin holds no release tag in HEAD's history.
#
# A release is a tag `v<semver>` on origin, as release-tag.yml pushes it. The one taken is the
# tag whose commit is an ancestor of HEAD and a descendant of every other such tag; $1's own tag
# (release.yml runs at it) is left out. It fails closed, ending the script, when the clone is
# shallow, origin's tags cannot be listed, a listed tag's commit is not in this clone, or no one
# tag is newest. Run it as `last="$(last_release "$v")" || exit 1`: `die` ends the subshell.
last_release() {
    local candidate="$1" listing name sha version t o newest="" winners=0
    local -a names=() shas=()
    [[ "$(git -C "$REPO_ROOT" rev-parse --is-shallow-repository)" == "false" ]] ||
        die "this clone is shallow, so the last release cannot be placed in its history; fetch the full history"
    listing="$(git -C "$REPO_ROOT" ls-remote --tags origin)" ||
        die "cannot list origin's tags, so the last release cannot be shown to be the newest"
    # One line per tag, `name commit`: an annotated tag's peeled `^{}` line names its commit.
    while read -r name sha; do
        version="${name#v}"
        [[ "$name" == v* && "$version" =~ $SEMVER ]] || continue
        [[ "$version" == "$candidate" ]] && continue
        git -C "$REPO_ROOT" cat-file -e "${sha}^{commit}" 2>/dev/null ||
            die "origin's tag ${name} names ${sha}, which is not in this clone; fetch tags"
        if git -C "$REPO_ROOT" merge-base --is-ancestor "$sha" HEAD; then
            names+=("$name")
            shas+=("$sha")
        fi
    done < <(awk '{
            ref = $2; sub("^refs/tags/", "", ref)
            if (ref ~ /\^\{\}$/) { sub(/\^\{\}$/, "", ref); peeled[ref] = $1 } else { direct[ref] = $1 }
        } END { for (r in direct) print r, ((r in peeled) ? peeled[r] : direct[r]) }' <<< "$listing")
    if [[ ${#names[@]} -eq 0 ]]; then
        echo none
        return 0
    fi
    for t in "${!names[@]}"; do
        for o in "${!names[@]}"; do
            git -C "$REPO_ROOT" merge-base --is-ancestor "${shas[$o]}" "${shas[$t]}" || continue 2
        done
        newest="${names[$t]}"
        winners=$((winners + 1))
    done
    [[ "$winners" -eq 1 ]] ||
        die "no one release tag is the newest in HEAD's history (${names[*]}): cannot tell which is the last release"
    echo "${newest#v}"
}

# The cargo-semver-checks release type that moving from version $1 to $2 admits. Below 1.0,
# cargo's rule is that 0.y.z -> 0.(y+1).0 may break and 0.y.z -> 0.y.(z+1) may only add, and
# cargo-semver-checks names those levels "major" and "minor". Only MAJOR.MINOR.PATCH versions are
# classified, and $2 must be newer than $1. Run it in `$(...) || exit 1`, as last_release.
release_type() {
    local last="$1" next="$2" lmaj lmin lpat nmaj nmin npat
    [[ "$last" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$next" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        die "the API check classifies only MAJOR.MINOR.PATCH versions, not ${last} -> ${next}"
    IFS='.' read -r lmaj lmin lpat <<< "$last"
    IFS='.' read -r nmaj nmin npat <<< "$next"
    if (( nmaj != lmaj )); then
        (( nmaj > lmaj )) || die "${next} is not newer than the last release ${last}"
        echo major
    elif (( nmin != lmin )); then
        (( nmin > lmin )) || die "${next} is not newer than the last release ${last}"
        if (( nmaj == 0 )); then echo major; else echo minor; fi
    else
        (( npat > lpat )) || die "${next} is not newer than the last release ${last}"
        if (( nmaj == 0 )); then echo minor; else echo patch; fi
    fi
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
