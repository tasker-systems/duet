#!/usr/bin/env bash
# tools/scripts/release/publish-crates.sh
#
# Verify, then publish, duet's crates to crates.io in dependency order. The two stages run in
# separate jobs of release.yml; `cargo make release-publish` runs both locally. Borrowed from
# temper's publish-crates.sh.
#
# Usage:
#   ./tools/scripts/release/publish-crates.sh VERSION --out DIR
#   ./tools/scripts/release/publish-crates.sh VERSION --from DIR [--dry-run]
#
#   --out DIR   the BUILD stage: `cargo package` every crate, which verify-compiles each against the
#               others' packaged versions, and copy the `.crate` files into DIR. A crate already on
#               crates.io at VERSION is left out, and when every crate is, nothing is compiled and
#               DIR is left empty. Holds no registry credential.
#   --from DIR  the PUBLISH stage: for each crate, in order, skip it if crates.io has it at
#               VERSION; else re-package it without compiling, require its bytes to equal the
#               verified `.crate` in DIR, and `cargo publish --no-verify` it. Compiles nothing.
#               --dry-run stops before the first upload.
#
# WHY TWO STAGES. The publish job holds the crates.io credential, and `cargo publish` verify-
# compiles by default, running every dependency's build script beside it. So the compile runs in
# the build job, which cannot mint a credential, and the publish job compiles nothing. Packaging is
# deterministic, so the publish job checks it pushes exactly the bytes the build job verified.
#
# Re-runs are idempotent: a version crates.io already lists is a loud skip, so a release that
# failed at crate 5 of 9 continues from crate 5, and a release whose crates were all published
# beforehand (the first release's local bootstrap, RELEASING.md) publishes nothing.
#
# Version agreement: VERSION must equal [workspace.package].version and every duet-* version in
# [workspace.dependencies]; update-versions.sh writes them together.
#
# Auth: in CI, crates.io trusted publishing (rust-lang/crates-io-auth-action exchanges the job's
# OIDC token for a short-lived CARGO_REGISTRY_TOKEN). Locally, cargo's stored credential.

set -euo pipefail
source "$(dirname "$0")/lib/common.sh"

VERSION="${1:-}"
DRY_RUN=false
OUT_DIR=""
FROM_DIR=""
shift || true
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        --from) FROM_DIR="$2"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done
if [[ -z "$VERSION" ]] || [[ -n "$OUT_DIR" && -n "$FROM_DIR" ]] || [[ -z "$OUT_DIR" && -z "$FROM_DIR" ]]; then
    die "usage: $0 VERSION (--out DIR | --from DIR [--dry-run])"
fi
[[ "$VERSION" =~ $SEMVER ]] || die "'${VERSION}' is not a semver version"
[[ -z "$OUT_DIR" || "$DRY_RUN" == "false" ]] || die "--dry-run belongs to the publish stage (--from)"

cd "$REPO_ROOT"
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"

[[ "$(workspace_version)" == "$VERSION" ]] ||
    die "[workspace.package] version is $(workspace_version), but ${VERSION} was requested; run update-versions.sh"
for crate in "${CRATES[@]}"; do
    [[ "$crate" == "duet-cli" ]] && continue
    [[ "$(workspace_dep_version "$crate")" == "$VERSION" ]] ||
        die "${crate}'s [workspace.dependencies] version is $(workspace_dep_version "$crate"), but ${VERSION} was requested"
done

if [[ -n "$OUT_DIR" ]]; then
    log_header "verify-package duet ${VERSION} into ${OUT_DIR}"
    mkdir -p "$OUT_DIR"
    MISSING=0
    for crate in "${CRATES[@]}"; do crate_published "$crate" "$VERSION" || MISSING=$((MISSING + 1)); done
    if [[ "$MISSING" -eq 0 ]]; then
        # Nothing to publish, so nothing is compiled: the release's publish job will not run.
        log_info "every duet crate is on crates.io at ${VERSION}; nothing to package"
        exit 0
    fi
    PKG_ARGS=()
    for crate in "${CRATES[@]}"; do PKG_ARGS+=(-p "$crate"); done
    # All at once, so the unpublished crates resolve each other from cargo's local overlay and the
    # whole set is verified before anything is published.
    cargo package "${PKG_ARGS[@]}"
    for crate in "${CRATES[@]}"; do
        if crate_published "$crate" "$VERSION"; then
            log_info "${crate} ${VERSION} is already on crates.io; left out"
        else
            cp "${TARGET_DIR}/package/${crate}-${VERSION}.crate" "$OUT_DIR/"
        fi
    done
    exit 0
fi

log_header "publish duet ${VERSION} from ${FROM_DIR} (dry run: ${DRY_RUN})"
PRESENT=0
for crate in "${CRATES[@]}"; do
    if crate_published "$crate" "$VERSION"; then
        log_info "${crate} ${VERSION} is already on crates.io; skipped"
        PRESENT=$((PRESENT + 1))
        continue
    fi
    VERIFIED="${FROM_DIR}/${crate}-${VERSION}.crate"
    [[ -s "$VERIFIED" ]] || die "${crate}-${VERSION}.crate is not in ${FROM_DIR}"

    # Re-package without compiling, and require the bytes the build stage verified. A dry run
    # stops here: the next crate cannot be packaged on its own until this one is on the registry.
    cargo package --no-verify -p "$crate"
    [[ "$(sha256 "${TARGET_DIR}/package/${crate}-${VERSION}.crate")" == "$(sha256 "$VERIFIED")" ]] ||
        die "${crate}-${VERSION}.crate packaged here differs from the one the build stage verified"
    if [[ "$DRY_RUN" == "true" ]]; then
        log_info "[dry run] ${crate} ${VERSION} matches the verified package; would publish it, and stops here"
        exit 0
    fi
    log_info "publishing ${crate} ${VERSION}"
    cargo publish --no-verify -p "$crate"
    PRESENT=$((PRESENT + 1))
done
[[ "$PRESENT" -eq "${#CRATES[@]}" ]] && log_info "all ${#CRATES[@]} duet crates ${VERSION} are on crates.io"
