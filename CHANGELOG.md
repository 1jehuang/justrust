# Changelog

All notable changes. Versions follow [semver](https://semver.org); while in
0.x, output formats and the `~/.justrust` layout may change in any release.

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
