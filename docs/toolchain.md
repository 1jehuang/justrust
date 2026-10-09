# Owning the toolchain

Decision (2026-10-08): justrust owns the entire Rust toolchain it builds with.
Maintenance cost and infrastructure are acceptable. The only goal is the
fastest possible edit -> check -> test loop for agents.

## Target

The reference loop is a one-line body edit in `jcode-desktop-ui` followed by
`justrust test -p jcode-desktop-ui --lib -- <filter>`. Today (run
20261007-235214927-3498261):

| phase | seconds | notes |
|---|---:|---|
| cargo startup | 0.33 | resolve, fingerprints |
| macro expansion | 2.70 | gpui `div()` builders, derives, `actions!` |
| other frontend (resolve, typeck, borrowck) | 1.96 | |
| codegen | 1.45 | incremental, most CGUs reused |
| incr persist (dep graph + result cache) | 0.85 | written on every run |
| link | 0.63 | mold, 670 MB test binary |
| other | 0.96 | metadata, misc |
| **total** | **9.0** | |

Goals: **< 2 s** with a pinned toolchain plus flags (phase 1-2), **< 0.5 s**
with a resident compiler (phase 4), and **< 0.2 s** to the first type
error. Cold fresh-dir builds: seconds, not minutes, by pulling artifacts
from a cache.

## Components

| component | how we own it | why |
|---|---|---|
| rustc | pinned nightly first, then fork `jr-rustc` | unstable flags, resident compiler, body-only invalidation |
| LLVM | prebuilt from rust CI first, built ourselves with PGO+BOLT later | faster codegen of dependencies |
| std / sysroot | rebuilt with `-Zbuild-std` flags we choose (share-generics, dylib) | dylib test binaries, fewer monomorphizations |
| cargo | pinned, then forked | in-process fingerprints, no rescan, daemon mode |
| codegen backends | cranelift for local opt-level 0 crates | faster full-crate regeneration (-25%) |
| linker | wild (incremental linking), mold as fallback | relinking a 670 MB binary on every edit |
| C toolchain | clang, lld, ar, a sysroot of pinned libs/headers | build scripts become hermetic, so their cache keys need no system stamp |
| pkg-config + -sys libraries | vendored sysroot (wayland, xkbcommon, alsa, fontconfig, openssl...) | same output on every machine |
| registry sources | local mirror / `cargo vendor` | offline, no index updates, stable paths |
| rust-analyzer | pinned to the same rustc, kept warm | sub-second type errors |
| test runner | justrust-owned (nextest-style process per test binary) | test selection, hot patching hook |
| hot patcher | subsecond-style patching of the running test binary | skip relinking entirely |
| artifact cache | depcache today, then a remote content-addressed store | cold builds from cache across machines and agents |

## Phases

Each phase ships only after a measured win on the reference loop
(`bench/edit-loop.sh` and the Desktop benches), recorded in FINDINGS.md.

1. **Pinned toolchain, managed by justrust.** `~/.justrust/toolchains/<id>`
   holds a nightly with rust-src, cranelift, and rustc-dev. Projects opt in
   (`justrust.toml` or `[toolchain]`). One toolchain everywhere, so depcache
   keys never churn from distro updates.
2. **Flag experiments on that toolchain**, each measured alone and combined:
   `-Zcache-proc-macros` (targets the 2.7 s), `-Zshare-generics`,
   `-Zhint-mostly-unused` on large deps, dependencies as one dylib for test
   builds (relink only the small crate), cranelift for local crates, wild.
3. **Hermetic sysroot.** Vendored C toolchain and libraries. The
   build-script cache loses its heuristics. Remote artifact cache.
4. **jr-rustc fork.** A resident compiler process per crate that keeps the
   query cache in memory between edits (no dep-graph load/persist, no
   re-expansion of untouched items), early diagnostics streamed to the
   agent, body-only changes in an upstream crate not invalidating
   downstream codegen (finding 5: 9.5 s became 46 s).
5. **Hot patching** of test binaries, so an edit that only changes function
   bodies reruns tests without relinking.

## Infrastructure

- Build host for rustc/LLVM (one full build is ~1 h on 16 threads; use a
  bigger box for PGO/BOLT).
- Rebase cadence: follow nightly weekly, cut a justrust toolchain every 6
  weeks. CI runs the Jcode and Desktop test suites on every toolchain cut.
- Artifact store: content-addressed, keyed like depcache, served over HTTP.

## Rules

- Correctness first: a toolchain cut must pass the full Jcode and Desktop
  test suites and produce the same test results as stock stable.
- Projects keep compiling on stock stable in CI. Nightly-only flags live in
  justrust, not in the projects' manifests.
- Fail open: if the managed toolchain is missing or broken, fall back to the
  system toolchain and say so in the verdict.
