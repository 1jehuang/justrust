# Alpha.18 telemetry release acceptance (2026-10-14 UTC)

## Ownership and release gate

Release preparation owns only Cargo.toml, Cargo.lock, CHANGELOG.md and this
record. Telemetry, recording, summary, remote implementation, billing, secrets
and tenant policy are not changed by the release owner. Unrelated dirty
benchmark files and run exports are preserved.

At 11:10 UTC, GitHub's latest published release was v0.1.0-alpha.17,
published at 08:25:19 UTC. Remote tags had no alpha.18. The live Apple Silicon
update endpoint returned alpha.17 with enabled=true and update=false for an
alpha.17 client. The coordinator reserved alpha.18 at 11:11 UTC and authorized
publication only after exact source plus bump native CI is fully green.

The release workflow builds five targets and publishes independently from CI.
Therefore no tag is pushed until the exact candidate commit has passed Linux,
Intel macOS, Apple Silicon macOS, Windows and MSRV CI. The tag must point to
that same commit, not a later untested main revision. A successful release
build alone is not a native test result.

## Changes and privacy boundary

Authenticated, enqueue-scoped telemetry attribution, structured schema 2
metrics, sampling observation quality and early failure events are documented
in CHANGELOG.md and docs/telemetry-coverage.md. The client notice explicitly
says usage statistics are linked to the account when signed in. Anonymous old
events are not retroactively attributed. Fixed categories and numerical metrics
are not raw diagnostic logs. Collection remains best-effort and opt-out remains
supported. No speedup claim is made.

The coordinator reported the installed source passed 427 tests, 3 ignored,
check and warning-free clippy. The cloud owner reported actual installed-client
authenticated delivery verified trusted account attribution in production.
These reports do not replace exact release-candidate native CI or released
binary acceptance. Hosted outage recovery check run
20261014-040706043-2400213 was reported successful by the coordinator.

## Initial blocking CI evidence

Run 38553140961 failed Windows clippy on Unix-only unused bundle fixture
helpers and Linux integration tests on an obsolete exact TOML error phrase.
Both native macOS jobs and MSRV passed that run. The coordinator assigned
minimal test-only fixes to a separate owner. Publication remains blocked until
fresh candidate CI succeeds; no checks are waived.

## Packaging follow-through, 11:23 UTC

Exact source-marker candidate 3d1c20d, CI 38555286828, passed Linux full tests
and smoke checks, but failed source-package verification because the unanchored
`toolchain/` exclusion also removed `src/toolchain/bundle.rs`. The coordinator
authorized a minimal manifest fix anchoring `/server/` and `/toolchain/`.
A package-list audit also found the shared dirty checkout's untracked Wrangler
runtime cache would be selected by `--allow-dirty`. `/.wrangler/` is now excluded,
and no dirty-checkout package was uploaded or published.

A clean archive of committed source at 522e6b2 plus only the manifest fix passed
`cargo package --list --locked` and complete `cargo package --locked` verification.
The package contains both Rust bundle modules and excludes root server, fork
submodules, Wrangler runtime cache, private justrust state, target outputs and
unrelated `a`/`b` scratch files. Packaging compiled and verified locally because
Cargo packaging is not a supported hosted command. No artifact was published.

Windows clippy now passes, but a Unix `/bin/echo` launcher test fails native
Windows path validation. The separate test owner is fixing the native fixture.
A Linux timeout fixture intermittently hit `Text file busy` under concurrent
forking; the owner is validating causal isolation rather than suppressing errors.
Both native Mac jobs remain queued. No publication gate has been waived.

The source-marker follow-up adds the compile-time `release_build` boolean to
structured run telemetry, distinguishing source builds from release binaries.
The coordinator is also auditing installer token attribution and lifecycle
visibility and explicitly holds publication pending that audit.

## Publication and rollout

Pending exact-candidate native CI and release build, asset/checksum verification,
live update endpoint checks for all five targets, and public released executable
acceptance. A customer's inaccessible private machine is not considered verified
merely because the update endpoint offers a release.
