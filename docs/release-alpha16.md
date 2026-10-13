# Alpha.16 release acceptance (2026-10-13)

## Purpose

Publish the pending justrust improvements and make released macOS clients
able to connect to Linux hosted builds. The old cross-platform installer
invoked `cargo install justrust --version <client version>`, but the crates.io
sparse index has no `justrust` package (HTTP 404 at release preparation).

Release builds now fetch the hosted machine's architecture from the same
GitHub release, verify the archive SHA-256 and exact executable version,
and replace the installed agent atomically. Source builds and unsupported
remote architectures retain their existing fallback. The opt-out
`JUSTRUST_REMOTE_FETCH_RELEASE=0` is honored.

This does not turn a Linux host into a native macOS compiler. macOS automatic
routing remains local, and explicitly hosted workloads must be Linux-compatible.

## Validation before tagging

- `cargo fmt --check`: passed.
- Hosted `justrust check --remote --locked`: passed,
  `20261013-074043475-677824`.
- Hosted `justrust test --remote --locked`: 329 passed, 0 failed, 2 ignored,
  `20261013-074113970-684828`.
- Hosted all-target clippy with `-D warnings`: passed,
  `20261013-074132836-689618`.
- `justrust show` confirms `where remote on hosted`. All cited summaries
  are exported under `data/runs/xps14/`.
- Installer regression executes the actual generated shell script against
  synthetic download fixtures. A corrupt checksum and a wrong executable
  version leave the old agent untouched. A verified matching archive replaces
  it successfully. Tests also cover x86_64/arm64 Linux target selection and
  source-build/unsupported-platform guards.

The initial shared session failed with an infrastructure ENOENT before
compilation. An isolated session with the existing account credentials
passed without changing billing, tenant isolation, or other agents' connections.
This is correctness validation, not a benchmark or a compile-speedup claim.

## Live service

Cloud readiness and installed `/Users` permissions were verified and recorded
in private `justrust-cloud/docs/macos-hosted-readiness-2026-10-13.md`.
The hardened directory wrapper matches source, and curl, tar, sha256sum and
mktemp are installed. The service follows GitHub latest with a five-minute
cache, automatic updates are enabled, and no release tag is pinned.

Publication, native platform CI, release endpoint delivery and live release
installation are checked after the tag is published. The customer's Mac
and private workspace are not accessible, so their actual retry cannot be
claimed as verified by these checks.
