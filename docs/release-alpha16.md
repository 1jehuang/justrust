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

## Active hosted sessions survive binary replacement

Release preparation exposed a second live issue: another client atomically
replacing the installed agent made an existing Linux agent's `current_exe`
point at a deleted pathname. Its next build failed with ENOENT before any
compilation. Commit `3687ab6` snapshots the running executable from its open
inode, publishes a stable non-replacing path, and starts all builds through
that path. Old snapshots are retained rather than unlinking active programs.

All 14 agent protocol tests passed on hosted run
`20261013-075429794-820147`, including a real subprocess whose installed
executable is atomically replaced. It then executes recorded, unrecorded,
and recorded offline checks successfully. The final recorded check compiles
zero units and the rustc shim resolves to the stable snapshot, checking that
recording still shares Cargo artifacts. Hosted check
`20261013-075149665-800573` and warning-free all-target clippy
`20261013-075431281-820282` also passed. These records are exported by the
fix owner.

After review, the cross-platform installer also uses a unique temporary
staging file for each connection, and the Mac test fixture supplies the
Linux checksum command through the equivalent native `shasum` tool.
Hosted installer tests `20261013-074833055-771527` and all-target clippy
`20261013-074848033-773179` passed after this hardening.

## Final prepublication validation

After installing the complete alpha.16 source with `cargo install --path .
--locked`, the live hosted path passed again with both fixes:

- Check: `20261013-075650793-836118`.
- Full tests: `20261013-075656254-836888`, 330 passed, 0 failed, 2 ignored.
- Warning-free all-target clippy: `20261013-075703750-837765`.

The new agent pins its executable before accepting the connection. The
checks use normal hosted admission and billing, not a local substitute.
Run summaries are exported alongside the earlier installer regressions.

## Live service

Cloud readiness and installed `/Users` permissions were verified and recorded
in private `justrust-cloud/docs/macos-hosted-readiness-2026-10-13.md`.
The hardened directory wrapper matches source, and curl, tar, sha256sum and
mktemp are installed. The service follows GitHub latest with a five-minute
cache, automatic updates are enabled, and no release tag is pinned.

## Published release and live acceptance

GitHub published `v0.1.0-alpha.16` at 2026-10-13 15:06:34 UTC, pointing at
`6766866`. It is public and the latest release. Release workflow
`38418175136` succeeded for all five targets: Intel and Apple Silicon macOS,
x86_64 and aarch64 Linux musl, and x86_64 Windows MSVC. Every published
archive was downloaded and verified against its published SHA-256 file.
Each archive contains the expected target directory, executable, README
and license. The published Linux executable reports `justrust 0.1.0-alpha.16`.

Exact-tag CI `38418175085` passed all jobs: Intel macOS, Apple Silicon macOS,
Linux, Windows, and MSRV. Both native Mac jobs passed clippy, tests, binary
smoke tests, cross-checkout cache tests, macOS integration paths and release
binary execution. These are native checks, not Linux substitutes.

At 15:09 UTC, live `/health` returned HTTP 200 and both macOS
`/v1/release/latest` targets returned alpha.16 with `enabled: true`,
`update: true`, and their correct architecture-specific archive and checksum
URLs for an alpha.13 client. Rollout followed the natural cache refresh,
without a release pin or security/configuration changes.

At 15:10 UTC, the actual public `jcode.sh/rust.sh` installer installed alpha.16
into an isolated scratch directory. Login, agents-file changes, telemetry
and background updates were disabled for this acceptance check. The exact
cross-platform shell template was also rendered from the release source
and executed on the live account host with a newly created isolated HOME.
It downloaded the published Linux archive, passed SHA-256 verification and
installed the exact alpha.16 executable. No shared host installation was
overwritten by that isolated installer check.

Finally, the publicly installed release binary checked an untouched archive
of the exact tag through the live hosted service:

| Command | Run | Observed result |
| --- | --- | --- |
| `check --remote --locked` | `20261013-081121855-923025` | Passed, hosted client wall 12.8s |
| `test --remote --locked` | `20261013-081134607-924206` | 330 passed, 0 failed, 2 ignored, hosted client wall 19.0s |
| `clippy --remote --locked --all-targets -- -D warnings` | `20261013-081153578-926111` | Passed without warnings, hosted client wall 7.7s |

`show` confirms remote on hosted. Summaries are exported, with scratch
source paths redacted. This clean-checkout acceptance used the normal signed-in
account, billing, admission and cache controls. The account-local native
Cargo fork had already been enabled by its owner; no fleet-wide toolchain
change was made for this release. These timings are not a speedup comparison.

The customer's Mac and private workspace are not accessible, so their actual
retry cannot be claimed as verified by these checks. Hosted machines remain
Linux and explicitly hosted projects must be Linux-compatible.
