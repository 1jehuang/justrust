# Parallelism map: what the measurements do and do not prove

Updated October 14, 2026. This supersedes the work/span interpretation in the
initial `critpath.py` report and the initial FINDINGS critical-path section.

## Mathematical target

For a *known dependency DAG* with processing cost `w(v)` per node:

- `W = sum(w(v))` is total processing work.
- `S = max_path sum(w(v))` is the longest dependency chain.
- Any schedule on `P` processors takes at least `max(W/P, S)`.
- `W/S` bounds average available parallelism for that DAG, not an observed
  trace's elapsed-time ratio.

These bounds require actual dependency edges and processing costs. rustc's
Chrome self-profile events do not supply either in full. An interval can
include a wait, descheduling, nested work or an external linker. The event
start/end envelope is the *observed schedule length*, not the dependency span.
Amdahl-style projections from observed single-thread intervals are hypotheses,
not mathematical proof that those intervals cannot run concurrently.

## Corrected analyzer

`bench/critpath.py` schema 2 reports elapsed-event occupancy. It does not report
CPU work, a causal span, or a theoretical speedup ceiling. Those fields are
explicitly null. `QueryBlocked` intervals now suppress the enclosing thread,
not just the wait event itself. Nested events count once per thread. Conversion
rounding overshoots up to one microsecond are clipped and counted explicitly.
Crossing intervals larger than this, invalid numbers and empty traces fail.

Fifteen regression tests exercise nested events, waits, parallel workers,
process/thread identities, equal boundaries, gaps, malformed input, rounding,
JSON filtering, the actual CLI and the crox subprocess argument boundary.
The four original captured profiles were reprocessed using `--min-us 50`.
Their schema now records the threshold and missing-edge limitations. Short
waits omitted by filtering cannot be reconstructed. Raw captures are retained
locally and on the original host volume, not checked into Git.

The large 16-thread traces remain heavily perturbed by profiling. Neither
those timings nor reported waiting thread-seconds establishes a production
contention regression. Use low-overhead phase captures for localization and
non-profiled matched `bench/suite.py` runs for performance conclusions.

## Where to investigate next

| Candidate | Dependency boundary to preserve | Required test/measurement |
|---|---|---|
| Weighted Cargo critical-chain scheduling | Only choose among already-ready units; preserve Metadata/All readiness, jobserver tokens, fingerprints and commands | Real cold CLI order, proc-macro/build-script/link success, failure/retry, cross-toggle artifact parity, inherited jobserver barriers, same-binary OFF/ON/OFF controls |
| Parallel decode of upstream generic-symbol metadata | Decode independent crate queries, merge in original deterministic crate order; respect conditional dependencies | Same binary off/decode/prefetch, incremental verification, symbol/result parity, reference and upstream-edit workflows |
| Earlier upstream-monomorphization prefetch | Must not steal more time from lints than it removes from mono collection | Measure full loop, not only the mono timer |
| Resolve/HIR work partition | Expansion order, resolver mutable state, visibility and cross-module names | Cross-module/error diagnostics and actual query dependency instrumentation before rewriting |
| Incremental persist overlap | Outputs, diagnostics and incremental-session publication must remain consistent | Existing fork path needs parallel frontend; back-to-back edits and linker-error recovery |
| Macro batching/caching | Macros can read env/files and use process-global state, so purity cannot be assumed | Explicit side-effect and invalidation model, ordered diagnostics, changed env/file regressions |

## Current permitted scheduling experiment

The default-off Cargo prototype ranks an already-ready unit `v` by the estimated
longest remaining consumer chain:

```text
rank(v) = estimated_cost(v) + max(rank(consumer(v)))
```

A leaf consumer has no suffix. The prototype changes only that priority, not the
prerequisite graph or the number of jobserver tokens. Uniform costs are a depth
heuristic, not measured compiler work. Optional bounded package-name hints are
coarse estimates: build-script, library and test units of one package can have
different costs. Metadata/All edges also allow pipelining, so this unit-level rank
is not a complete causal span for the compiler's internal tasks.

This can improve a multi-crate schedule only when choosing ready work affects
which dependency chain finishes last. It does not make one remaining crate use
more cores, remove dependency edges, or prove a hardware utilization ceiling.
Independent builds across tenants are a separate scheduling dimension, with CPU,
memory, billing and isolation constraints still intact.

`bench/scheduler-sweep.py` exercises pinned suite workflows with one immutable
candidate binary in OFF/ON/ON/OFF order. Real public Cargo integration tests are
required before interpreting those performance numbers. The production default
stays unchanged unless the correctness and full-loop performance gates pass.
Compiler-query/codegen implementations remain subject to the compiler checkout's
local review/policy gates, not bypassed by this experiment.

The 96-core sweep already showed the current reference loop plateauing around
4-8 allocated cores. That is a measurement of today's implementation, not an
immutable ceiling. This map guides the next fork experiments without claiming
a speedup or a proven 96-core utilization target.
