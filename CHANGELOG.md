# Changelog

All notable changes. Versions follow [semver](https://semver.org); while in
0.x, output formats and the `~/.justrust` layout may change in any release.

## Unreleased

## 0.1.0-alpha.18

- **Account-linked diagnostic telemetry** uses service-authenticated account
  attribution and the credential scope captured when each event is queued.
  Account changes do not reassign old events, custom endpoints receive no hosted
  credentials, and notices now disclose account linkage when signed in.
- **Structured diagnostics (schema 2)** include run and event IDs, failure and
  routing categories, phase and test timing, cache costs, pinned toolchain IDs,
  and sampling quality. Missing observations remain unknown rather than zero.
- **Early failures** record fixed diagnostic categories for initialization,
  remote setup, execution, and result recording without collecting raw logs,
  source, paths, crate/test names, argument values, or credentials.
- **Best-effort delivery** deduplicates scoped retries and exposes queue and
  authentication outcomes through `justrust telemetry status`. Existing opt-out
  controls remain supported. Collection is not lossless, and old anonymous
  events are not retroactively linked to accounts.
- **Build lifecycle visibility** asynchronously reports authenticated queued,
  running, heartbeat, and terminal states correlated by run ID. Log availability
  describes client-local persistence, not centrally uploaded raw logs.
- **Recoverable local logs** use a bounded client-local JSONL journal with a
  nominal one-second persistence tick, without `fsync`. Interrupted logs can be
  retrieved using the exact run ID. Raw logs are never uploaded, and this is not
  a crash-proof or lossless durability guarantee.
- **Persistence diagnostics** expose fixed-label local recording failures while
  preserving Cargo's exit status.
- **Bounded lifecycle delivery** captures authentication scope when events are
  queued and retries in a detached sender, with at most four attempts and a
  one-second delay between retries. Telemetry status includes local HTTP/curl
  health diagnostics. Delivery remains best-effort.
- **Build provenance** distinguishes source and release builds with a compile-time
  `release_build` marker in structured run metrics.
- Windows sampling bounds process traversal against reused-PID cycles.
  No compile-speedup claim is made for this release.

## 0.1.0-alpha.16

- **macOS clients can install the Linux hosted agent.** Release clients
  download the host architecture's binary from the same GitHub release,
  verify its archive checksum and executable version, then replace the agent
  atomically. This avoids the unavailable crates.io source-install fallback.
- **Active hosted agents survive upgrades.** Each connection pins its running
  executable at a stable path, so another client replacing the installed
  binary cannot break later builds or their recording shims.
- **Remote reconnects** mirror only workspaces requested by the build, while
  invalidating all remembered source manifests before any reuse.
- **Remote diagnostics** include opt-in client phase tracing and slow-sync
  phase breakdowns. No compile speedup is claimed for this release.

## 0.1.0-alpha.15

- Fix the hosted prebuilt-policy regression test so it checks normal costs
  and the host's forced-fetch override without changing global environment.

## 0.1.0-alpha.14

- **Hosted builds on macOS with longer user names work.** The ssh control
  socket used Linux's 108-byte limit; macOS allows 104, so a home like
  `/Users/<17 characters>` made ssh fail and every hosted build ran locally.
- **Hosted fleet at capacity** gets its own message and builds locally, and
  connect polling follows the server's hint (10 s while a machine is being
  created, was 2 s).
- **Remote sync** seeds a new mirror from its public git origin on the
  machine when no mirrored relative exists, and installs dev builds on the
  machine as a zstd patch. The seed no longer needs GNU `timeout` (macOS).
- **Prebuilt store** fetch cost is measured per machine instead of a fixed
  700 ms.
- **`justrust upgrade`** switches plans for subscribers instead of failing.

## 0.1.0-alpha.13

- **Remote builds need a card on file.** The 25 free no-card builds are
  gone. The first month of the $20 plan is free (5,000 builds, card
  required, $20/month after unless canceled): start it at
  [jcode.sh/rust](https://jcode.sh/rust#get-started) or with
  `justrust upgrade`, which now opens the free-month checkout when the
  account is eligible. A build without a card prints one line saying so and
  runs locally. Trial builds used up (`end_trial`) get their own message.
- **`justrust status`** shows where a build runs (local, remote, hosted),
  including remote client builds in flight and their client wall time.
- **waste:** a depcache miss names keyed env vars cargo does not set (user
  env or cargo `[env]`, e.g. `LIBRARY_PATH`) that keep keys from matching
  the prebuilt store.

## 0.1.0-alpha.12

- **Automatic updates.** Release binaries check build.jcode.sh at most
  daily in the background, download the new release from GitHub, verify
  its sha256 and that it runs, and swap it in when no build is running.
  `justrust update [now|status|on|off]`; `JUSTRUST_NO_UPDATE=1` turns it
  off. Source builds only print a notice.
- **Anonymous usage telemetry** (timings, counts, waste kinds, flag names,
  update outcomes, panic locations; never code, paths, or names), queued
  locally and sent in background batches. `justrust telemetry off`,
  `JUSTRUST_NO_TELEMETRY=1`, or `DO_NOT_TRACK=1` turns it off.

## 0.1.0-alpha.11

- **Hosted builds hit the prebuilt dependency store.** The build machine
  downloads every store hit (fetches overlap at 32 jobs), and
  `CARGO_BUILD_JOBS` is no longer part of cache keys (the agent sets it,
  so hosted keys never matched the store). Cold `check -p jcode-tui` from
  a fresh worktree: 529 of 575 dependencies downloaded, 32.4 s on the
  machine (FINDINGS 65).
- The build machine installs justrust from this release on GitHub
  (sha256-checked against your binary) instead of an 11 MB upload over
  your connection: 0.4 s instead of 40-130 s after each update.
- Removing a mirrored worktree no longer sends the next remote build local.
- Windows builds and tests pass again.
- **Remote optimized builds.** `justrust build --release` (or any
  optimized profile, host target) routes like `check`: it goes remote once
  the local build takes over 5 s and stays there while it wins. The binary
  comes back into `target/<profile>/` as a zstd delta against the last one
  fetched (0.1-2 MB, under a second), checked with `ldd` before it replaces
  the local file. Debug `build` and `run` stay local. 1.75x on the justrust
  release edit loop (FINDINGS 45b).

## 0.1.0-alpha.10

- `justrust login` honours `JUSTRUST_NO_TELEMETRY` and `DO_NOT_TRACK` even
  when an earlier install saved an install id.
- The kept-local report includes the command when `JUSTRUST_REMOTE=off`.

## 0.1.0-alpha.9

- **Install funnel.** The installer reports installer_start and
  installer_finish (or the step that failed) with a random install id, OS,
  and CPU arch, and saves the id in `~/.justrust/install_id`.
  `justrust login` sends that id with its sign-in flow id, so an install
  can be followed to its account. No IP is stored, nothing else is sent
  before sign-in. `JUSTRUST_NO_TELEMETRY=1` or `DO_NOT_TRACK=1` turns it off.

## 0.1.0-alpha.8

- **Hosted builds are the remote path.** `justrust login` selects hosted
  builds and switches off the legacy AWS backend. The ssh backend is
  removed. Entitlement is confirmed with the build fleet, so a fleet outage
  is reported as an outage, not an account problem.
- **Operator visibility for hosted builds.** Each remote run reports its
  command, project, client version, machine, and agent session, then
  running/done/abandoned with exit code and wall time. A signed-in client
  that keeps a build local reports why (router rule, remote unavailable,
  billing). Both are detached and never delay or fail a build.
- **macOS support**: real recording, sampling, depcache, slots, and the
  sync daemon instead of plain cargo passthrough.

## 0.1.0-alpha.7

- **Prebuilt store on by default, cost-aware.** A dependency is downloaded
  only when that beats compiling it here, from the store's price list
  (compile time and size per unit) and this machine's measured download
  rate. Quiet 16-core laptop, cold check of 105 popular crates: 19-26 s
  vs 34-35 s compiling; projects the store does not cover are unaffected.
  `JUSTRUST_PREBUILT=off` disables it, `JUSTRUST_PREBUILT_ALWAYS=1`
  fetches every hit.

## 0.1.0-alpha.6

- **Prebuilt store is opt-in again** (`JUSTRUST_PREBUILT=default`, now
  served through CloudFront). Measured on a quiet 16-core laptop, a cold
  check of 105 popular crates took 80-138 s from the store against 36-39 s
  compiling locally, so on by default made such machines slower. It still
  helps on busy machines, small VMs, and CI.

## 0.1.0-alpha.5

- **Prebuilt store hits from release binaries.** The static musl release
  keyed proc macros (and every crate depending on one) by its own C
  library, musl, instead of the host's, so on glibc hosts those units
  never matched the store. The host's C library and glibc version are now
  detected at runtime. Cache keys of proc macros change once for release
  users.

## 0.1.0-alpha.4

- **Public prebuilt dependency store, on by default.** After a local miss,
  dependency units come from justrust's S3 store (popular crates.io crates
  compiled by our fleet with the pinned toolchain), verified before use.
  `JUSTRUST_PREBUILT=off` disables it, `JUSTRUST_PREBUILT=<url>` points at
  another store. Cold check of 43 popular crates: 60-70 s -> 39-43 s.
  Cache keys are portable across cargo homes and symlinked registries
  (scheme 8), and vendored-C build scripts (ring, aws-lc-sys,
  libsqlite3-sys, onig_sys) replay from the store.
- **Test pool.** Test binaries run in parallel, likely failures first,
  slow tests in their own processes, failures under overlap confirmed alone,
  cargo's fail-fast honored. `JUSTRUST_TEST_PARALLEL=0` turns it off.
- **Early diagnostics (opt-in).** `JUSTRUST_EARLY=1` or `auto` keeps a
  rust-analyzer per workspace and prints type errors in ~0.1 s while cargo
  runs. `justrust early start|stop|status`.
- **wild linker for local crates** (`justrust toolchain wild`), falling
  back to the project's linker on any failure. `JUSTRUST_LINKER=off`.
- **jr-cargo**: mimalloc and parallel manifest parsing (no-op `test`
  0.39 s -> 0.22 s with the pinned toolchain's cargo fork).
- Fixes: daemons no longer inherit the caller's file descriptors (held
  locks), remote sync keeps source directories named `target`, remote
  builds size jobs for the build machine.

## 0.1.0-alpha.1

First public release. Linux only.

- **Agent interface.** `justrust check|test|build|clippy|run` take cargo's
  arguments, print errors and failing tests in full plus the first few
  warnings (`JUSTRUST_MAX_WARNINGS`), drop progress noise, and end with a
  verdict and a timing report with detected waste. Exit code is cargo's.
- **Build recorder.** Per-unit rustc timings (frontend, codegen, link,
  incremental cache), rebuild reasons, lock waits, test time, CPU, memory,
  and pressure stalls. `justrust runs`, `show`, `log`, `status`.
- **`cargo` proxy.** `justrust install` records every cargo build without
  changing its output or invalidating caches. Fails open.
- **Per-agent build slots and a shared dependency cache** so parallel agents
  stop contending for the target dir lock (`justrust slots`, `justrust cache`).
- **`justrust split`.** Suggests which code to move into its own crate to stop
  rebuild cascades, from recorded edits and the module graph.
- **`justrust history`.** Mines Jcode session history for cargo calls.
- **Experimental remote builds.** `justrust remote` routes `check`, `clippy`,
  and `test` to an AWS machine or ssh host when measured faster. Off unless a
  machine is configured (`JUSTRUST_REMOTE=off` to force local).
- **Other platforms.** On macOS and Windows, `justrust check|test|build|clippy|run`
  run plain cargo without recording. Other commands are unavailable.
