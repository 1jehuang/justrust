# Owning the toolchain

Decision (2026-10-08): justrust owns the entire Rust toolchain it builds with.
Maintenance cost and infrastructure are acceptable. The only goal is the
fastest possible edit -> check -> test loop for agents.

## Architecture mandate: independent, vertically integrated toolchain

We own the whole path from source and manifests through compilation, native
dependencies, artifacts, linking, and test results. The end state is one
integrated system, not a set of interchangeable stock-tool replacements.
Our forks are independent development lines. They may end up with completely
different internals, or be replaced or merged into new components entirely.
The phases and component names below describe current starting points, not
permanent boundaries or a requirement to keep upstream's design.

External correctness is the compatibility contract: supported Rust semantics,
existing manifests, build-script and proc-macro behavior, native-library and
platform ABIs, correct builds and test results, and useful diagnostics.
Within that contract, redesign protocols, query graphs, IRs, artifact formats,
cache interfaces, scheduling, codegen, linking, and execution together.
Choose designs by measured end-to-end agent wait and correctness, not by
upstream compatibility, upstreamability, patch size, or ease of rebasing.

Version our integrated toolchain and cache identities whenever private
contracts change. Stock fallback uses its own compatible execution path and
artifacts, never private formats. Evaluate upstream correctness and security
fixes as inputs to our design, adapting or reimplementing them where useful.
There is no goal of minimal divergence or mandatory upstream rebase cadence.

## Source ownership (2026-10-14)

Our compiler and Cargo implementations are vendored as ordinary tracked
source in `toolchain/rustc/` and `toolchain/cargo/`. They are not submodules
and no longer require editing or publishing separate fork repositories.
Compiler, Cargo, standard-library, and client changes can land together in
one justrust commit. Root `AGENTS.md` governs these trees. Historical upstream
technical documentation is implementation reference, not project policy.

The source import preserves licenses and build/test inputs while removing
inherited agent and separate-repository contribution instructions. Import
identities are provenance, not an obligation to rebase. This does not yet
make all external dependencies vendored, all builds offline, or the compiler
and Cargo one resident process. See `docs/toolchain-vendoring.md` for build
commands, dependency boundaries, and migration validation.

## Target

The primary target is expected agent wait across the session-weighted
synthetic workload, not one selected compile. Follow the measurement policy
in `bench/README.md`: matched `bench/suite.py run --synth 1 -n 1` before/after
comparisons, with command/change breakdowns and relevant regression checks.
Use `bench/versus.py run --synth 1 -n 1` for comparisons against stock.

The historical reference loop is a diagnostic: a one-line body edit in `jcode-desktop-ui` followed by
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
| rustc | owned source: `toolchain/rustc/`, built with `toolchain/rustc/build-jr.sh` (stage 1, CI LLVM; historical separate-fork cold build ~9.5 min) | unstable flags, resident compiler, body-only invalidation (`docs/integrations.md`) |
| LLVM | prebuilt from rust CI first, built ourselves with PGO+BOLT later | faster codegen of dependencies |
| std / sysroot | owned Rust library source in `toolchain/rustc/library/`; bootstrap builds compiler-matched std, while `-Zbuild-std` enables project-specific flag experiments (share-generics, dylib) | coordinated compiler/library changes, dylib test binaries, fewer monomorphizations |
| cargo | owned source: `toolchain/cargo/`, built with `toolchain/cargo/build-jr.sh`, installed into the pinned toolchain as `bin/jr-cargo` by `justrust toolchain cargo-fork` | resolve stamp, mimalloc and parallel manifest prefetch shipped (agent-level no-op `test`: 0.39 s stock -> 0.22 s, FINDINGS 38, 39). A resident cargo without kept caches gives nothing; next: cache the targeted resolve per workspace (~200 ms), then fingerprints (~120 ms) |
| codegen backends | Rust-side backend sources included under `toolchain/rustc/compiler/`, including Cranelift integration; installed backends remain toolchain-matched artifacts | faster full-crate regeneration (-25% historical Cranelift result) |
| linker | wild for local crates, shipped (`justrust toolchain wild`, fail open to the project's linker; FINDINGS 36: link pass 0.44 -> 0.23 s). Incremental wild when it exists. Deps-as-dylib prototyped, not worth it yet | relinking a 283 MB test binary on every edit |
| C toolchain | clang, lld, ar, a sysroot of pinned libs/headers | build scripts become hermetic, so their cache keys need no system stamp |
| pkg-config + -sys libraries | vendored sysroot (wayland, xkbcommon, alsa, fontconfig, openssl...) | same output on every machine |
| registry sources | local mirror / `cargo vendor` | offline, no index updates, stable paths |
| rust-analyzer | source included at `toolchain/rustc/src/tools/rust-analyzer/`; runtime integration currently uses the pinned `rust-analyzer-preview`, resident per workspace (`src/early.rs`, opt-in `JUSTRUST_EARLY=1`) | first type error in 0.08-0.09 s (FINDINGS 28), callers and syntax errors, 14/19 first errors found with 0 false positives, `JUSTRUST_EARLY=auto` (FINDINGS 29). Borrowck and exhaustiveness need the resident rustc |
| test runner | justrust-owned test pool, shipped: binaries in parallel, likely failures first, slow tests split out, overlapped failures confirmed alone (FINDINGS 34: justrust test -31%, Desktop first failure 14.4 -> 7.0 s) | test selection, hot patching hook |
| hot patcher | subsecond-style patching of the running test binary | skip relinking entirely |
| artifact cache | depcache, peer and remote cache, and a static prebuilt store (`JUSTRUST_PREBUILT`, `justrust cache --publish`) with keys portable across cargo homes and hermetic keys for vendored-C build scripts (FINDINGS 30, 37: cold Desktop check from an empty cache 53.3 -> 14.8 s, floor 12.7 s). Public store deployed with a daily AWS prebuild fleet (105 seed crates) behind CloudFront, on by default and cost-aware (only units whose download beats compiling, from the store's `v1/costs` and the measured rate): quiet laptop cold check 19-26 s vs 34-35 s compiling (FINDINGS 44; fetch-all was 2-3x slower, FINDINGS 43). Next: one HTTP/2 connection, rmeta-only for check | cold builds from cache across machines and agents |

## Phases

The optimizations that owning both sides of a layer boundary enables, ranked,
are in `docs/integrations.md`.

Each phase needs before/after measurements recorded in FINDINGS.md.
Focused scenarios and factors establish component-specific wins. General
speed claims and default promotion require the comprehensive synthetic
comparison plus relevant regression and correctness checks, not merely a
win on the historical reference loop (`bench/edit-loop.sh` and Desktop).

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
   and runner behind `JUSTRUST_HOTPATCH=1`. Adapt useful upstream fixes as
   needed, without a mandatory rebase schedule.

## Architecture tracks (started 2026-10-12)

Changes to how justrust itself is structured, independent of the rustc
fork. Each is opt-in until a factor in `bench/factors.py` shows a win and
the correctness check passes. Status is updated as results land.

| track | what | where | status |
|---|---|---|---|
| resident daemon | one long-lived justrust process per user (`justrust __buildd`): prebuild on edit (each agent session's last request, 0.4 s after an edit burst, niced; `test` compiles only with `--no-run`, tests run when asked), coalescing of identical concurrent builds | `src/buildd.rs`, **on by default** for non-terminal agent builds on Linux, off under CI (fa79800, c998b6a; `JUSTRUST_BUILDD=0`, `JUSTRUST_BUILDD_PREBUILD=0`) | shipped (FINDINGS 51): reference loop with the mined 4.6 s think time **8.87 -> 5.14 s** (n=6, disjoint); predictor right 53%, 0.47 wasted builds per edit; another agent's wrong-guess prebuild within noise; replayed result = direct build (pass and fail), no Dirty units. Coalescing ties slots, warm per-run state has nothing to win (5 ms). Next: predict the next command from history (68% ceiling) |
| shadow split | when an agent tests a leaf module of a giant crate and only that file changed, run its tests in a generated leaf crate (`#[path]` to the real file) in `~/.justrust/shadow/`; same test names, the user's tree is never written | `src/shadow.rs`, `JUSTRUST_SHADOW=1` (opt-in), `justrust shadow` lists candidates | measured (FINDINGS 57, `factors-20261012-060141-2501cadf6e90-dirty.json`): reference loop **7.36 -> 0.79 s** (n=6) with a warm shadow; test lines identical. Not default: only ~2% of recorded edits land in leaf modules (the hot files, `panel*`/`workspace*`, are not leaves), the first use rebuilds dependencies (~100 s, different feature unification), and the big crate's uses of the leaf are only rechecked by the next normal build. Next: exact feature set (shared dependency units), background `check -p <big>` after a routed run; the real fix for the hot files is upstream splits (FINDINGS 57 list) |
| hot patching / test host | link only regenerated codegen units against the base test binary, loader patches before `main`; then a warm zygote per test binary | `src/hotpatch.rs`, `JUSTRUST_HOTPATCH=1` | measured (FINDINGS 48, n=8): patches on every edit through `justrust test`, full UI suite identical (1303 passed), link pass 0.33 -> 0.12 s, patch step 40-105 ms, wall within noise (8.23 vs 8.39 s). Stays opt-in until the frontend shrinks. Zygote not built: process startup is 0.2-0.4% of a full suite (jcode-tui ~50 ms of 16.6 s), big suites are test bodies |
| workspace crate cache | content-addressed cache for workspace/path crates (depcache keys plus link args), so reverts, branch switches, fresh target dirs and slots reuse compiled local crates | `src/localcache.rs`, **on by default** (d783f29, `JUSTRUST_LOCALCACHE=0` turns it off) | shipped (FINDINGS 50): revert 10.6 -> 0.70 s (with `SOURCE_DATE_EPOCH`; Desktop's build.rs stamps build time, such units are detected and not stored), justrust revert 2.41 -> 0.88 s, fresh target dir 66 -> 6.3 s, loop within noise (miss path ~35 ms). Byte-identical outputs, no Dirty units, key-poisoning checks miss correctly. Next: worktrees at another path need `--remap-path-prefix` |
| early exit | return rust-analyzer's coded errors in the changed files at once and let cargo finish in the build daemon (next identical request and `justrust log` get rustc's verdict; an edit stops it) | `src/early.rs`, `src/buildd.rs`, `JUSTRUST_EARLY_EXIT=1` (opt-in), factor `early-exit`, `bench/early_exit/` | measured (FINDINGS 59): type-error loop **3.20 -> 0.08 s** (n=8); 120 agent-weighted mutations on Jcode and Desktop: 79 exits, 78 right file and error, 1 wrong, 0 on passing builds, recall 67%; 0 false exits on 24 clean edits and 1,180-file sweeps; replay of 563 agent error runs saves 11.7-17.3% of their wall. Opt-in locally (5 GB per workspace); hosted default plan in `docs/early-exit-hosted.md`, not deployed |
| resident rustc | take the incremental persist/load off the hot rustc's critical path: (a) persist dep graph and query cache in parallel with join codegen + link; (b) reply to the shim after the link and persist in the background | jr-rustc 913df80fb7a1 (a, **on** in the fork, `JUSTRUST_RUSTC_OVERLAP_PERSIST=0`), 9c3c3ab5531a (b, opt-in `JUSTRUST_RUSTC_EARLY_REPLY=1`), factors `jr-overlap-persist`, `jr-early-reply` | (a) shipped (FINDINGS 58): warm Desktop rustc 4.45 -> 4.20 s, loop 4.93 -> 4.61 s (n=14), replay 4.52 -> 4.18 s (n=20); same diagnostics, verify-ich clean, UI suite passes. (b) opt-in (FINDINGS 60): loop 4.32 -> 4.10 s (n=14, within noise); back-to-back edits wait 0.3-0.5 s on a `jr-persisting` marker and stay incremental. Stopped: a preloading resident process would win ~0.26 s (dep-graph load 0.25 s + exec 0.01 s), under the 0.3 s bar; the rest needs a GlobalCtxt kept across sessions (multi-month redesign, FINDINGS 60) |
| cross-crate cutoff | jr-rustc writes an interface hash of each crate's metadata (`<rmeta>.jriface`, spans excluded, positions logical) and accepts a dependency rebuilt with the same interface; jr-cargo skips dependents whose recorded interface hashes still match | jr-rustc `justrust-10x` (b35416a3743d), jr-cargo `jr_cutoff.rs`, shim `JUSTRUST_RUSTC_IFACE_HASH` on jr toolchains (`JUSTRUST_CUTOFF=0`, `JUSTRUST_CARGO_CUTOFF=0`); factors `jr-cutoff*`; checks `jr/iface-hash-test.sh`, `jr/iface-stress.py`, `jr/cutoff-real-e2e.sh`, `bench/cutoff_semantic.sh`, `bench/cutoff_suites.sh` | measured (FINDINGS 63): jcode-core body edit **9.67 -> 5.16 s** (n=8, four runs agree), cost +0.03 s on the check loop (rmeta +0.1%), interface-change control equal; Desktop and Jcode suites match. Not in the default jr toolchain yet (`jr-b35416a3743d`, non-default). Next: PGO+BOLT build and ship as default |
| test-filter compile | the libtest filter reaches jr-rustc's harness: unselected tests become `#[allow(dead_code)] const`s (checked, not codegened) | jr-rustc `JUSTRUST_RUSTC_TEST_FILTER`, justrust `JUSTRUST_TEST_FILTER_COMPILE=1` (opt-in), factor `jr-test-filter`, `jr/test-filter-test.sh` | measured (FINDINGS 64): binary 242 -> 49 MB, loop 4.49 vs 4.51 s (noise): incremental codegen already reuses unchanged tests. Next: filter-aware frontend |
| cargo fork server | park a jr-cargo per exact invocation right after `create_bcx` (resolve, manifests, unit graph), fork it per request; the client passes its stdio fds and environment; a stamp of manifests, `Cargo.lock`, configs, rustc and target-discovery dirs falls back to a plain run on change | jr-cargo `src/jr_daemon.rs` (e238aec29..), `JUSTRUST_CARGO_DAEMON=1` (opt-in), justrust da4f05e (test pool socket via env), bf4f90e (samples the forked build); factors `cargo-daemon`, `cargo-daemon-check`, `cargo-daemon-loop`; check `jr/daemon-correct.py` | measured (FINDINGS 61): no-op `test` **0.24 -> 0.12 s** (n=12, helps); body-edit `check` 1.62 -> 1.46 s and reference loop 5.54 -> 5.25 s, within noise. 18-step edit sequence identical to plain cargo, fingerprints stay plain-cargo fresh. Opt-in: ~230 MB per parked key, no eviction yet. Next: LRU/memory cap, buildd prestarts servers for predicted commands, ship in the default jr toolchain, keep fingerprints warm in the parked process (57-95 ms) |

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
