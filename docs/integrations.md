# Cross-layer integrations

Why own the whole stack (rustc, cargo, std, linker, test runner) instead of
wrapping it: vendoring alone is neutral (pinned nightly vs system stable:
8% faster, below the 10% noise floor, `bench/FACTORS.md`). The gains come
from changing the interfaces *between* layers, which wrappers cannot do.
Finding 16 is the proof case: skipping downstream rebuilds after body-only
edits is impossible from a wrapper.

Companion to `docs/toolchain.md` (what we own, phases). This file is the
list of optimizations that only exist once we own both sides of a boundary.

## The reference loop today

Desktop, one-line edit, `justrust test -p jcode-desktop-ui --lib --
fps_counter`, 7.6 s (FINDINGS 20, `bench/suite.py`):

| phase | s |
|---|---:|
| cargo startup (resolve, fingerprints, plan) | 0.5 |
| frontend (macro expansion 2.7 of it, type check, borrow check) | 3.8 |
| codegen | 1.1 |
| incremental persist (dep graph + result cache) | 0.8 |
| link (670 MB test binary) | 0.5 |
| test run + other | ~0.9 |

What is already measured as *not* helping the warm loop: parallel
front-end, mold vs lld, cranelift, pinned nightly (`bench/FACTORS.md`). The
loop is bounded by redone work, not slow passes.

## Integrations, ranked by expected effect

| # | integration | layers | removes | expected |
|---|---|---|---|---|
| 1 | **One resident build process**: cargo calls rustc as a library; dependency metadata and query state stay in memory across crates and edits | cargo + rustc | process start, decoding ~740 dependencies' metadata per run, incremental load/persist | ~1.3 s (startup + persist) plus part of the frontend |
| 2 | **Test-filter-aware compilation**: the filter flows into rustc and libtest; only selected tests and what they reach are checked and codegened | cargo + rustc + libtest | most of frontend + codegen on filtered runs | possibly the largest single win |
| 3 | **Cross-crate early cutoff**: rustc emits a public-interface hash; cargo freshness uses it, so body-only upstream edits do not rebuild dependents | cargo + rustc | the finding 5 cascade (9.5 s -> 46 s) | large on upstream edits |
| 4 | **Cached macro expansion**: proc-macro and `div()` expansions reused when input tokens are unchanged | rustc + proc-macro server | most of the 2.7 s expansion | large |
| 5 | **Errors first**: rustc checks edited items first and streams diagnostics to justrust | rustc + justrust | waiting for the full build to see a type error | first error ~0.2 s instead of ~2 s |
| 6 | **Patch, don't relink**: rustc reports changed functions; incremental linker (wild) or hot patcher updates the binary in place | rustc + linker + test runner | link and binary rewrite | 0.5 s+ |
| 7 | **Dylib test builds**: std and dependencies as one shared library with our flags; only the small crate relinks | std + cargo + linker | relinking dependencies | part of link |
| 8 | **One global scheduler**: a jobserver that knows phases, shared across agents | cargo + rustc + justrust | the cgroup heuristics in `src/sched.rs` | contention cases (today 12.9 s median vs 24.5 s off) |
| 9 | **Deterministic outputs everywhere**: same toolchain and flags on every machine, so cache keys match | whole stack | depcache misses across machines and agents | cold builds minutes -> seconds |

1 to 3 all hinge on cargo and rustc becoming one resident system, which is
the main argument for forking both. Together they plausibly reach the
< 1 s body-edit target. That is an estimate, not a measurement.

## Memory budget for a resident compiler

Peak rustc RSS from run records (FINDINGS / `~/.justrust/runs`):
`jcode_desktop_ui` ~1.1 GB check, ~2.0 GB test (warm), 4.0 GB worst;
`jcode_app_core` up to 4.5 GB test. Keeping the 2 to 4 hot crates resident
is about 5 to 15 GB per workspace; several agents need LRU eviction and
sharing one resident compiler per source state, like slots. Long-lived
rustc was never designed for, so expect leaks to fix.

## Cheaper steps first

- Phase 2 flags on the pinned toolchain, each as a `bench/factors.py`
  factor: `-Zcache-proc-macros` (targets the 2.7 s), `-Zshare-generics`,
  `-Zhint-mostly-unused`.
- Skip cargo on the hot path: record the unit graph (`--unit-graph`) once,
  replay rustc + link directly when only one local crate changed, fall back
  to cargo when the plan may be stale (~0.5 s).
- rust-analyzer as an early-error oracle while rustc builds (toward 5).

## Rules

- Every integration ships only with before/after numbers from
  `bench/suite.py` or `bench/factors.py`, and gets an off switch so it can
  be a factor.
- Same test results as stock stable on the Jcode and Desktop suites.
- Fail open to the stock toolchain.
- Cost accepted: weekly rebases onto nightly for each fork
  (`docs/toolchain.md`, Infrastructure).
