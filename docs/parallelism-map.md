# Parallelism map: what the measurements do and do not prove

Updated October 15, 2026. This supersedes the work/span interpretation in the
initial `critpath.py` report and the initial FINDINGS critical-path section.

Latest accepted milestones: [named query-execution CPU ownership](query-cpu-ownership-20261015.md)
assigns validated exclusive CPU costs to actual admitted query executions, and the
[observed native fork/join task DAG](observed-task-dag-20261015.md) retains genuine
instrumented task prerequisites and CPU-chain witnesses. The cold accepted task
DAG attaches only about 10% of observed CPU. The accepted
[invocation-level execution flow](execution-flow-20261015.md) connects actual
native query scope and fork/join tokens, assigns every retained CPU segment in
its cold/warm/mixed cohorts, and passed twelve actual-source tests plus
[twenty-five independent public replay cases](../bench/results/20261015-execution-flow-public-acceptance.json).
It does not infer edges from query nesting, worker chronology or whole-job
completion. The cold retained known chain is 44.280185 ms, including
37.521239 ms of `typeck_root`, but 175 explicit cuts prevent interpreting that
chain as the complete dependency span.
The accepted [latch-readiness provenance](latch-readiness-20261015.md) adds exact
row-bound producer/waiter obligations and passed a fourteen-test harness (seven
actual-source and seven safety tests) plus 34 independent public replay cases.
Cold qualified 138 of 139 encounters while preserving one genuine unbound race
and incomplete aggregate/wait flags. Its 20.787135 ms known chain includes
17.556408 ms of `typeck_root`; these different instrumentation captures are not
a paired performance comparison with the earlier flow measurements.
None of these accepted milestones establishes the complete execution DAG or a
useful-thread ceiling. Visibility and wake-to-consume cuts remain unknown. Cache
publication remains a separate authority boundary, and paired performance
promotion remains blocked by the documented A2 baseline defects and noisy samples.

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
Compiler-query/codegen experiments must preserve external Rust semantics and
pass the repository's native correctness and measured-performance gates.

The 96-core sweep already showed the current reference loop plateauing around
4-8 allocated cores. That is a measurement of today's implementation, not an
immutable ceiling. This map guides the next fork experiments without claiming
a speedup or a proven 96-core utilization target.

## Decision after the first scheduling experiment

The immutable 24-job OFF/ON/ON/OFF experiment is now published in
`cargo-scheduler-abba-20261014.md`. Forty measured samples and sixteen warmups
selected the exact candidate Cargo and pinned compiler. Uniform-cost priority
did not demonstrate a full-loop improvement. Four comparisons were uncontrolled
and body check was within noise, with additional known overlapping work retained.
Do not promote this candidate or treat that outcome as a limit on compiler
parallelism. Its rank changes which ready crate starts, not the internal work of
the final crate.

The subsequent installed-client acceptance confirms that a real reference edit
compiled only one UI unit with 745 fresh units. The immediate unchanged repeat
compiled zero with 746 fresh. The edited unit took 5.18 seconds and recorded
about 7.5 CPU-seconds in `20261013-214047661-1443635`. Their ratio is observed
average CPU use of that invocation, not the processor requirement, dependency
span, or maximum attainable speedup. This was one functional acceptance sample,
not a pinned before/after performance comparison.

The next investigation priorities follow the measured workflow boundary:

| Workflow | Observation | Next decision boundary |
|---|---|---|
| Warm edited filtered test | One dirty UI unit; changing ready-crate order cannot parallelize its internal tasks | Focus localization on the UI compiler invocation before another scheduler or hardware promotion |
| Reduced-category reference profile | t=1 exclusive labels include mono graph walk 0.561 s, expansion 0.465 s, late resolve 0.295 s, query-result encoding 0.233 s | These identify locations to investigate, not independent tasks or proven serial costs |
| Reduced-category check profile | t=1 expansion 0.189 s, late resolve 0.137 s, metadata generation 0.123 s | A mono-only change cannot automatically explain a check-loop gain; retain both workflows |
| Upstream generic-symbol decode | t=1 reference exclusive label 0.084 s | Do not mistake this narrow label for the entire mono walk or a route to a large full-loop win |
| Cold multi-crate check | Current candidate uses uniform depth, not measured cost | Weighted costs remain untested; measure unit identity and Metadata/All release events before treating a coarse package hint as a causal DAG |
| Unchanged hosted test | No compiler units rebuilt | CPU parallelism cannot remove client sync, connection and test-launch latency; keep host-local and service latency separate |

The numerical labels above come from the committed schema-2 t=1 captures in
`bench/results/critpath/`, not from summing inclusive timers. Their instrumentation
and filtering caveats still apply, and the t=16 traces are substantially perturbed.
No label is substituted for `w(v)` in a mathematical speedup bound.

### What must be observed before a hardware decision

1. Identify actual independent work and prerequisite releases, not merely
   overlapping timestamp intervals or a Cargo package dependency graph.
2. Establish costs on pinned inputs without intrusive profiling, retaining
   CPU, memory, job-count and contention observations.
3. Validate the resulting toolchain on filtered tests, body checks, upstream
   edits, diagnostics, failure/retry and incremental artifact reuse.
4. Compare the whole same-binary loop at multiple core allocations using
   `bench/suite.py`, then compare throughput with multiple independent builds.
   A single-crate latency plateau does not imply a multitenant throughput plateau.

Measurement fixes `4809f23` and `e629a00` are installed and accepted, as recorded
in `measurement-acceptance-20261014.md`. Start any new sweep in that distinct
recorder epoch. Never rewrite the earlier ABBA noise flags to manufacture a win.

The earlier experiments did not change compiler semantic/query/cache/codegen
implementations. The root `AGENTS.md` is authoritative for the owned compiler
and Cargo source trees. Subsequent native instrumentation is validated against
external behavior, useful diagnostics, artifact correctness and fail-open
recording, rather than inherited upstream contribution policies.
