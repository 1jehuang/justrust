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

## Native CI and expanded acceptance hold, 11:55 UTC

Combined portability/package candidate 264d7bc, CI 38556244057, passed Linux,
Windows and MSRV, including actual tests and smoke checks. Both native Mac jobs
remain queued after more than 30 minutes. These passes do not authorize a later
source commit or waive native macOS execution.

The coordinator expanded the acceptance gate to installer token attribution,
asynchronous build lifecycle and bounded client-local interrupted logs. Installed
lifecycle source 41b787c produced trusted queued/running/summary/done events for
actual run 20261014-043509383-2672389, documented by its owner in
`docs/install-observability-acceptance.md`. This is accepted lifecycle evidence,
not proof of centralized log access or final public release acceptance.

Journal candidate c9961bf has passed native Linux and MSRV in CI 38558559636.
Intermediate commit 8c17751, CI 38559000957, fails warning-as-error checks because
its persistence diagnostics have no callers in that committed snapshot; the
caller owner still has pending changes. Final publication remains explicitly
held for the coordinator's complete source and installer audit. No alpha.18 tag
or release has been pushed, and the previous latest release remains alpha.17.

## Recovered release observation, 12:19 UTC

Source main `2dd8f347dfc38be0eb44e60dfac638b1e305c412`, CI 38561542585,
has passed native Linux (fmt, strict all-target clippy, tests, smoke and package),
Windows (strict all-target clippy, tests, smoke, integration and release binary)
and MSRV. Both `macos-latest` and `macos-15-intel` remain queued with no
executed steps. This is an external queue blocker, not a demonstrated global
outage or permission to substitute platforms.

This source includes typed persistence callers and strict fixture opt-outs
(`68b20fe`), bounded asynchronous sending (`02edd55`) and recoverable logs
(`c9961bf`). The coordinator reports final installed-source check, 437 passing
tests with 3 ignored, and strict all-target clippy passed, plus real local
success/failure, heartbeat, SIGKILL recovery, expired `unknown_lost` ledger and
fresh billed hosted delivery acceptance. These coordinator-reported checks do
not replace native CI for the final exact release candidate.

The coordinator's final docs/data acceptance commit is still incoming, so
`2dd8f34` is the observed source snapshot, not yet an approved final tag target.
Publication remains explicitly HOLD until the coordinator lifts it for the
exact candidate and all five native jobs are green. The release workflow has
no dependency on CI, so neither a tag nor a release dispatch is authorized.
GitHub's latest published release was rechecked at 12:19 UTC: alpha.17,
published at 08:25:19 UTC. No source, version, changelog, workflow, tag or
publication changes were made by the recovered observer.

## Publication and rollout

Pending exact-candidate native CI and release build, asset/checksum verification,
live update endpoint checks for all five targets, and public released executable
acceptance. A customer's inaccessible private machine is not considered verified
merely because the update endpoint offers a release.
