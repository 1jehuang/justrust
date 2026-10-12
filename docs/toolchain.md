# Owning the toolchain

Decision (2026-10-08): justrust owns the entire Rust toolchain it builds with.
Maintenance cost and infrastructure are acceptable. The only goal is the
fastest possible edit -> check -> test loop for agents.

## Target

The reference loop is a one-line body edit in `jcode-desktop-ui` followed by
`justrust test -p jcode-desktop-ui --lib -- <filter>`. It is measured by
`bench/suite.py run desktop-test-edit` on pinned sources; the latest is
7.59s median (`bench/results/20261010-010147-c219b496046c.json`: startup
0.48, frontend 3.83, codegen 1.11, incr persist 0.77, link 0.47). The older
breakdown below (run 20261007-235214927-3498261, unpinned):

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
| rustc | forked: `jr-rustc` (github.com/1jehuang/jr-rustc, branch `justrust`, based on the pinned nightly's commit), submodule pointer at `toolchain/rustc` (opt-in checkout: `git submodule update --init --checkout toolchain/rustc`), built with `./build-jr.sh` (stage 1, CI LLVM, ~9.5 min cold) | unstable flags, resident compiler, body-only invalidation (`docs/integrations.md`) |
| LLVM | prebuilt from rust CI first, built ourselves with PGO+BOLT later | faster codegen of dependencies |
| std / sysroot | rebuilt with `-Zbuild-std` flags we choose (share-generics, dylib) | dylib test binaries, fewer monomorphizations |
| cargo | forked: `jr-cargo` (github.com/1jehuang/jr-cargo, branch `justrust`), pinned as submodule `toolchain/cargo` (opt-in checkout: `git submodule update --init --checkout toolchain/cargo`), installed into the pinned toolchain as `bin/jr-cargo` by `justrust toolchain cargo-fork` | resolve stamp, mimalloc and parallel manifest prefetch shipped (agent-level no-op `test`: 0.39 s stock -> 0.22 s, FINDINGS 38, 39). A resident cargo without kept caches gives nothing; next: cache the targeted resolve per workspace (~200 ms), then fingerprints (~120 ms) |
| codegen backends | cranelift for local opt-level 0 crates | faster full-crate regeneration (-25%) |
| linker | wild for local crates, shipped (`justrust toolchain wild`, fail open to the project's linker; FINDINGS 36: link pass 0.44 -> 0.23 s). Incremental wild when it exists. Deps-as-dylib prototyped, not worth it yet | relinking a 283 MB test binary on every edit |
| C toolchain | clang, lld, ar, a sysroot of pinned libs/headers | build scripts become hermetic, so their cache keys need no system stamp |
| pkg-config + -sys libraries | vendored sysroot (wayland, xkbcommon, alsa, fontconfig, openssl...) | same output on every machine |
| registry sources | local mirror / `cargo vendor` | offline, no index updates, stable paths |
| rust-analyzer | pinned to the same rustc (`rust-analyzer-preview`), resident per workspace (`src/early.rs`, opt-in `JUSTRUST_EARLY=1`) | first type error in 0.08-0.09 s (FINDINGS 28), callers and syntax errors, 14/19 first errors found with 0 false positives, `JUSTRUST_EARLY=auto` (FINDINGS 29). Borrowck and exhaustiveness need the resident rustc |
| test runner | justrust-owned test pool, shipped: binaries in parallel, likely failures first, slow tests split out, overlapped failures confirmed alone (FINDINGS 34: justrust test -31%, Desktop first failure 14.4 -> 7.0 s) | test selection, hot patching hook |
| hot patcher | subsecond-style patching of the running test binary | skip relinking entirely |
| artifact cache | depcache, peer and remote cache, and a static prebuilt store (`JUSTRUST_PREBUILT`, `justrust cache --publish`) with keys portable across cargo homes and hermetic keys for vendored-C build scripts (FINDINGS 30, 37: cold Desktop check from an empty cache 53.3 -> 14.8 s, floor 12.7 s). Public store deployed with a daily AWS prebuild fleet (105 seed crates) behind CloudFront, on by default and cost-aware (only units whose download beats compiling, from the store's `v1/costs` and the measured rate): quiet laptop cold check 19-26 s vs 34-35 s compiling (FINDINGS 44; fetch-all was 2-3x slower, FINDINGS 43). Next: one HTTP/2 connection, rmeta-only for check | cold builds from cache across machines and agents |

## Phases

The optimizations that owning both sides of a layer boundary enables, ranked,
are in `docs/integrations.md`.

Each phase ships only after a measured win on the reference loop
(`bench/edit-loop.sh` and the Desktop benches), recorded in FINDINGS.md.

1. **Pinned toolchain, managed by justrust.** *Shipped 2026-10-10*
   (`justrust toolchain`, README "Pinned toolchain"; FINDINGS 2026-10-10:
   neutral on the loop, -12% within noise). `~/.justrust/toolchains/<id>`
   holds a nightly with rust-src, cranelift, and rustc-dev. Projects opt in
   (`justrust.toml` or `[toolchain]`). One toolchain everywhere, so depcache
   keys never churn from distro updates.
2. **Flag experiments on that toolchain.** *Done 2026-10-11, none ship*
   (FINDINGS 32: every flag within noise on the reference loop).
   `-Zcache-proc-macros` caches derives only, and most of the macro time is
   attribute macros (`#[gpui::test]`) and `div()` builders, so an
   attribute proc-macro cache in jr-rustc is the real target. Each flag was
   measured alone and combined:
   `-Zcache-proc-macros` (targets the 2.7 s), `-Zshare-generics`,
   `-Zhint-mostly-unused` on large deps, dependencies as one dylib for test
   builds (relink only the small crate), cranelift for local crates, wild.
3. **Hermetic sysroot.** Vendored C toolchain and libraries. The
   build-script cache loses its heuristics. Remote artifact cache.
   *In progress*: the four vendored-C build scripts on Desktop are keyed
   by the C toolchain instead of the host package db and replay from the
   prebuilt store (FINDINGS 37). Next layers in `docs/c-sysroot.md`.
4. **jr-rustc fork.** A resident compiler process per crate that keeps the
   query cache in memory between edits (no dep-graph load/persist, no
   re-expansion of untouched items), early diagnostics streamed to the
   agent, body-only changes in an upstream crate not invalidating
   downstream codegen (finding 5: 9.5 s became 46 s).
5. **Hot patching** of test binaries, so an edit that only changes function
   bodies reruns tests without relinking. *Prototyped 2026-10-11*
   (`bench/hotpatch/`, FINDINGS 35), not shipped. Design: justrust's linker
   shim links a base once (`--no-gc-sections`) and records each codegen
   unit's inode. After an edit, the units with new inodes (rustc hardlinks
   reused ones) are linked alone by lld into a small image at a fixed
   address next to the no-ASLR base, with every other symbol bound to its
   base address through a generated linker script. The test runner
   preloads a loader that maps the image and writes `jmp rel32` over each
   replaced function's entry before `main`. Unlike subsecond it needs no
   jump table in the code, because test processes start fresh. Fallback to
   a full link on TLS accesses, symbols missing from the base, or large
   changes. Measured on the reference loop: link 0.54 s -> 0.20 s, wall
   within noise (~5% of today's loop). Ship it after phase 4, when
   frontend + persist are under a second and the link is the largest fixed
   cost left (then 30-50% of the loop). Remaining work: cache the base
   symbol index (patch step 150 ms -> ~30 ms), integrate it into the shim
   and runner behind `JUSTRUST_HOTPATCH=1`, rebase on a schedule.

## Architecture tracks (started 2026-10-12)

Changes to how justrust itself is structured, independent of the rustc
fork. Each is opt-in until a factor in `bench/factors.py` shows a win and
the correctness check passes. Status is updated as results land.

| track | what | where | status |
|---|---|---|---|
| resident daemon | one long-lived justrust process per machine: identical concurrent agent builds share one execution (late arrivals get the full output and exit code), warm per-run state, seam for pre-building on edit | `src/buildd.rs`, `JUSTRUST_BUILDD=1`, `justrust daemon prebuild -- ARGS` | coalescing committed (a2c6ad1), not measured yet |
| shadow split | build a private copy of the workspace with often-edited modules of giant crates moved into leaf crates (leaf is a dependency of the old crate, re-exported under the old path, so test names stay the same); route a command to the leaf when only leaf files changed; map diagnostics back to the user's files | `JUSTRUST_SHADOW=1` (planned) | manual A/B on Desktop (`~/.jcode/scratch/shadow-proto`): edit `fps_counter` + its tests 6.0 s -> 0.49 s median (n=6); edits staying in the big crate +0.2-0.5 s. Productizing |
| hot patching / test host | link only regenerated codegen units against the base test binary, loader patches before `main`; then a warm zygote per test binary | `src/hotpatch.rs`, `JUSTRUST_HOTPATCH=1` | measured (FINDINGS 48, n=8): patches on every edit through `justrust test`, full UI suite identical (1303 passed), link pass 0.33 -> 0.12 s, patch step 40-105 ms, wall within noise (8.23 vs 8.39 s). Stays opt-in until the frontend shrinks. Zygote not built: process startup is 0.2-0.4% of a full suite (jcode-tui ~50 ms of 16.6 s), big suites are test bodies |
| workspace crate cache | content-addressed cache for workspace/path crates (depcache keys plus link args), so reverts, branch switches, fresh worktrees and slots reuse compiled local crates | `src/localcache.rs`, `JUSTRUST_LOCALCACHE=1` | committed (239bef0) with factors for revert, loop overhead, fresh target dir (bb59f9f); not measured yet |

Why these four: the reference loop is frontend-bound, and the fork owns
that. These remove the work around the compiler instead: rebuilding code
that did not change (shadow split, workspace cache), relinking and
restarting test processes (hot patching), and duplicated or serialized
builds between agents (daemon).

## Infrastructure

justrust is one all-in-one solution: client, toolchain, and servers.

- **Prebuilt artifact registry.** A cargo-like service that serves prebuilt
  dependency artifacts (rlibs, rmeta, proc-macro dylibs, build-script
  outputs) for popular crates and for our own projects, keyed exactly like
  depcache (toolchain id, target, flags, features, source hash). Because we
  ship the toolchain, keys match across every machine, so cold builds
  download instead of compiling. Content-addressed, served over HTTP, local
  depcache as the first tier.
- **Remote compile service.** Server-grade CPUs (many cores, lots of RAM,
  fast NVMe) running the same vendored toolchain, with warm incremental
  state and the shared artifact cache. justrust decides per build whether
  remote or local is faster, from measured cost: cold builds, full-crate
  regenerations, and large test suites go remote, small incremental edits
  stay local. Source sync is incremental (only changed files). Results are
  pushed back to the cache so the next local build hits.
- **Toolchain build farm.** Builds rustc/LLVM (one full build is ~1 h on 16
  threads; PGO+BOLT needs more), the std variants, and the C sysroot.
- Rebase cadence: follow nightly weekly, cut a justrust toolchain every 6
  weeks. CI runs the Jcode and Desktop test suites on every toolchain cut.

## Rules

- Correctness first: a toolchain cut must pass the full Jcode and Desktop
  test suites and produce the same test results as stock stable.
- Projects keep compiling on stock stable in CI. Nightly-only flags live in
  justrust, not in the projects' manifests.
- Fail open: if the managed toolchain is missing or broken, fall back to the
  system toolchain and say so in the verdict.
