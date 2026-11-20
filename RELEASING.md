# Releasing duet

A release publishes every duet crate to crates.io at one shared version, tags the repository
`v<VERSION>`, and creates a GitHub Release with generated notes. The tooling is borrowed from
temper's, cut down to one lane.

## The shared version

Every published crate (duet-core, duet-midi, duet-abc, duet-drums, duet-dsp, duet-capture,
duet-play, duet-schemas, duet-bus, duet-actions, duet-cli) has the same version: `[workspace.package].version`, which each
inherits. The repo-root `VERSION` file holds it too, and each crate's entry in
`[workspace.dependencies]` names it. `tools/scripts/release/update-versions.sh` is the one writer
of all of them. A crate whose source did not change still gets the new version, so the one tag
describes the whole set.

Below 1.0, a minor bump (0.3.0 to 0.4.0) is the one that may break the Rust API, and a patch may
only add to it.

## Cutting a release

1. **Check.** On an up-to-date `main`, `cargo make release-check` (a patch) or
   `cargo make release-check-minor` reports the next version, runs the API check, and packages
   every crate. It changes nothing. The API check (`tools/scripts/release/api-check.sh`) runs
   cargo-semver-checks on each library crate against the last release on crates.io, at the bump
   requested. A crate whose API changed more than the bump allows is named; use the next level up.
   duet-cli is the application, so its contract is the command line (`crates/duet-e2e`), and it
   is not API-checked.
2. **Prepare.** `cargo make release-prepare`, `release-prepare-minor` or `release-prepare-major`
   runs the same checks, then cuts `release/v<NEXT>`, writes every version site, commits,
   verify-packages every crate from the commit, pushes, and opens the release PR.
3. **Merge.** Merging the release PR changes `VERSION` on `main`.
   [`release-tag.yml`](.github/workflows/release-tag.yml) pushes the `v<VERSION>` tag and calls
   [`release.yml`](.github/workflows/release.yml):
   - **build-crates** runs the API check again, on the tagged candidate, so what is published is
     what was judged: a `VERSION` change `release-prepare` did not make, or a merge to `main`
     after the check, is judged too. Then it verify-packages every crate crates.io does not yet
     have at the version, and records each package's SHA-256 as a job output. It holds no
     credential.
   - **publish-crates** downloads those packages, checks them against the hashes, exchanges its
     OIDC token for a short-lived crates.io token (trusted publishing), and publishes them in
     dependency order, compiling nothing. It does not run when there is nothing to publish.
   - **github-release** creates the Release, with notes generated from the merged PRs.
4. **Verify.** The run is green, the Release exists, and crates.io lists every crate at the new
   version.

A run that fails partway is re-run from the Actions tab with `workflow_dispatch` and the tag:
every version crates.io already lists is skipped, so it continues where it stopped.

## The API check

`tools/scripts/release/api-check.sh CANDIDATE` judges the tree at `HEAD` as `CANDIDATE`.

- **The last release** is origin's newest `v<semver>` tag in `HEAD`'s history (`last_release` in
  `tools/scripts/release/lib/common.sh`). `release-prepare` opens the window from it, and requires
  `VERSION` to name it. The check passes it to cargo-semver-checks as `--baseline-version`, so
  the baseline is the copy crates.io published, and there is one derivation of it. A shallow
  clone, a remote whose tags cannot be listed, a tag whose commit is not in the clone, or no one
  newest tag refuses.
- **The bump** from the last release to `CANDIDATE` is the class the check admits. Below 1.0, a
  minor may break and a patch may only add. duet declares only the class, so the check cannot tell
  one break from another.
- **A new crate** is one crates.io answers 404 for, or lists at `CANDIDATE` alone (the bootstrap
  below). Any other failed lookup refuses: it is never read as "not published".

Its bite test, `tools/scripts/release/test-api-check.sh`, runs in CI with fakes for crates.io
and cargo-semver-checks. Only a real release run shows the check against the real registry and
comparator.

## Authentication

**crates.io trusted publishing**, with no registry secret. Each crate's trusted publisher on
crates.io names this repository and **two** workflow files: `release-tag.yml`, because inside the
release-tag.yml → release.yml chain crates.io matches the calling workflow, and `release.yml`,
for the `workflow_dispatch` re-run. A new crate needs both entries.

crates.io attaches a trusted publisher only to a crate that already exists, so a crate's first
version is published once with an API token, from a maintainer's machine:

1. Cut the release PR as above, and check out its branch.
2. `cargo login`, with a token scoped to publishing.
3. `cargo make release-publish` verify-packages every crate, checks the first crate crates.io
   lacks against its verified package, asks, and publishes what crates.io lacks, in order.
4. Add the trusted publishers for every crate on crates.io, and revoke the token.
5. Merge the release PR, with nothing merged to `main` before it: the published packages are its
   branch's, and the tag should name the same sources. Its run finds every crate on crates.io
   already, publishes nothing, mints no credential, and tags the release and creates the Release.

`cargo make release-publish` is also a recovery door for a release CI cannot finish.

## Pinned triggers

The two release workflows are the only ones that can mint a crates.io credential, so their
triggers are fixed (SC-GHA-3): `release-tag.yml` runs only on a push to `main` that changes
`VERSION`; `release.yml` only when `release-tag.yml` calls it, and on `workflow_dispatch`.
Adding a trigger to either is a change to the release's security, and is reviewed as one.
