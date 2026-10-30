# Incremental reuse: measuring the gap and closing it

## Objective

Minimize expected agent-visible source-to-result wait across the pinned,
session-weighted workload. A maximum fraction of cached query calls is not the
objective: ten thousand cheap hits can hide one expensive recomputation.

There is no established architecture-independent numerical maximum. Compiler
query boundaries are implementation choices, not language requirements. The
useful target is a measured frontier for specified edit classes, correctness
requirements, hardware and memory budgets. Report unknowns rather than assuming
all elapsed frontend time is removable.

## First directly observable gap

`rustc_query_impl::execution::load_from_disk_or_invoke_provider_green` first
proves a dependency node green, then tries to obtain its value. When the value
cannot be loaded, it invokes the provider again despite the green node.

Two native self-profile generic activities now distinguish this case:

* `jr_incremental_green_query`: the value-acquisition scope for a proven-green
  node, including loading/recomputation and subsequent fingerprint verification.
* `jr_incremental_green_recompute`: only the provider invocation when no disk
  value was obtained, excluding the subsequent verification.

These are opportunities to investigate for retaining values in memory, not a
promise that the entire interval can disappear. Providers can perform nested
queries, diagnostics and other required work. A resident design must preserve
those effects and safely translate or retain the result's references. Counts
are observed scope entries, not distinct semantic queries across sessions.

The markers require self-profile generic activities. Query-name arguments also
require the profiler's argument filter. Missing markers in older or differently
filtered captures are unknown coverage, never a measured zero opportunity.
Profiles alter runtime overhead, so instrumented elapsed values are diagnostics,
not baseline production performance.

## Measurement ladder

1. **Proven-green recomputation.** Capture cold, unchanged forced rebuild,
   body edit, revert and type error. Count marked scopes and measure their
   union per thread rather than summing nested intervals. Keep separate
   wall-envelope, thread-elapsed, CPU and critical-path claims. Rank query
   kinds only when the argument filter actually recorded them.
2. **Changed-input, unchanged-result recomputation.** Bind stable key and value
   fingerprints to actual execution decisions in ordered edit pairs. An
   unchanged result discovered after executing is not proof execution could
   soundly have been skipped beforehand. Determine which conservative input
   dependencies can actually be narrowed.
3. **Opaque frontend work.** Account for expansion, resolution, metadata,
   parsing and nonquery state separately. Query metrics alone cannot establish
   whole-compiler coverage. Cache macro results only with correct environment,
   filesystem and diagnostic dependencies.
4. **Backend and delivery.** Identify changed codegen units, monomorphizations,
   link input changes, artifact copies and test execution. Preserve ABI,
   relocations and initialization semantics. Frontend reuse does not eliminate
   required codegen, linking or test bodies.
5. **End-to-end frontier.** Compare ordinary execution with each sound candidate
   under matched workload and memory limits. Include cold start, eviction,
   repeated edits, cross-crate signature edits, macros, errors and contention.
   The fastest measured correct candidate is an empirical frontier, not proof
   of a global optimum.

## Implementation order after profiling

Choose by expected agent wait removed per unit of retained state, not call
count. For disk-serializable results, evaluate extending retention only where
load/validation beats recomputation. For arena-owned nonserializable results,
a resident context must retain valid references or use generation-safe handles.
Do not reuse raw values across freed compiler sessions.

A native resident compiler needs source-map and diagnostic generations,
query invalidation, dependency metadata ownership, cancellation and failure
isolation. Bound resident memory with eviction. A forked warm compiler that
only avoids loading the old graph is a different, narrower experiment and must
not be described as retaining all live IR across edits.

Keep the stock fallback on its own compatible formats. Version private cache
and toolchain identities before changing their semantic contract. Never serve
artifacts from an incompatible compiler to the stock path.

## Acceptance and promotion

`bench/reuse-native-accept.py` invokes the newly built compiler directly because
it tests this native profiling protocol. It checks cold, unchanged, body-edit,
revert and failing builds with profiling on and off. Successful binaries must
be byte-identical and produce the expected output. Diagnostics and exit status
must match, and `-Zincremental-verify-ich` is enabled. This small fixture is a
correctness gate, not a broad performance benchmark or complete Rust coverage.

For actual reuse changes, require before/after `bench/suite.py` results with the
same comprehensive seed, points, snapshots, mode and weights. Hosted synthetic
measurements use `bench/versus.py --remote`, as enforced by the current harness.
Inspect errors and sample coverage even if a harness exits zero. Record a
blocked aggregate as blocked, not as a speedup. Supplement with real workloads,
negative controls and output-equivalence checks. Publish remaining unobserved
work and memory cost alongside any claimed gain.

## Observed frontier, 2026-10-30

| Boundary | What is established | What remains unestablished |
| --- | --- | --- |
| Proven-green value acquisition | Native named markers distinguish acquisition from provider recomputation; real warm application reports retain per-thread unions | Whole-compiler coverage, removable critical-path time, and a universal reusable-IR percentage |
| Three scalar query values | Default-off tracked retention; 22-case native edit/error/cross-crate gate; selected fixture recomputations decrease; observed extra query-cache storage is 316 bytes in that fixture | Real-workload benefit, general memory/storage cost and exhaustive correctness |
| Associated-item lookup values | Independent default-off tracked codec rebuilds symbol index in the new arena; 34-case native gate and five existing incremental fixtures pass in both modes; selected fixture recomputations 10 to 0 | End-to-end benefit, memory/decode cost and broad regression coverage; see `associated-item-retention-20261030.md` |
| Empty lint-owner setup | Default-off tracked fast path; 22-case native diagnostic gate, 88 invocations with incremental fingerprint verification | Performance, arbitrary macro/custom-lint/query-cycle behavior, broader Rust regressions |
| End-to-end scalar workload | Exact blocked comprehensive baseline and failed named diagnostic retained, including all failure output | A passing matched before/after comparison and default promotion |
| Resident arena-owned state | Lifetime and diagnostic-generation requirements identified | A bounded, cancellation-safe implementation and measured benefit |

The scalar experiment persists disk-serializable values. It does **not** retain
a live compiler context, HIR arenas, or arbitrary references across processes.
The lint experiment avoids setup on a particular empty-input branch. It does
**not** cache an entire lint map or remove an existing HIR traversal. These are
different mechanisms and must be measured independently with the other flag
held fixed. Neither establishes that the maximum possible reuse has been reached.

The broad baseline failed before enabling either candidate. Runtime isolation
repairs some credential/Git-context failures, but other pinned-source assertions
and state-sensitive tests remain. Do not turn their failures into a compiler
speed claim, silently omit them, or compare inherited-context baseline samples
against isolated-context candidate samples. See
`query-retention-benchmark-20261030.md` and `../bench/RUNTIME-CONTEXT.md` for the
retained evidence and limitations.

## Later matched frontier, October 30, 2026

The earlier table is historical, not the final coverage inventory. Subsequent
native acceptance and real-workload evidence establish additional mechanisms:

* Narrow literal/TT declarative expansion reuse passed 357 differential semantic
  compilations and independent positive cache-hit gates. The real TT edit had
  634 eligible lookups and one provider execution. This did not establish a
  useful end-to-end latency win. See `declarative-tt-real-20261030.md`.
* Demand-only query persistence passed 288 main differential compilations and
  ten profiled mechanism invocations. Removing eager promotion reduced a
  recorded persistence interval, but the matched total median was 1.431 seconds
  versus 1.421 seconds with the option off. See
  `blaze-demand-br42-20261030.md` and `demand-query-real-20261030.md`.
* The br44 backend/thread frontier accepted seven stacks and 35 executed edits.
  Cranelift plus four frontend threads measured 1.187 seconds, versus 1.289
  seconds for its owned baseline and 0.101/0.112 seconds for the Blaze stacks.
  See `native-backend-frontier-br44-20261030.md` for exact gates and artifacts.

These are descriptive medians from scoped, seeded warm workflows, not
significance tests, comprehensive-workload wins, or default-promotion evidence.
Fresh Blaze-store failures remain separate from the accepted warm workflow.
The native experimental compiler was built with incremental compiler compilation
and thin-local LTO, rather than a trained distribution PGO/BOLT build. Compiler
build quality is another comparison boundary, not evidence that it explains the
remaining gap or that release compilation alone will close it.

The next investigations must distinguish repeated frontend construction from
backend graph work and orchestration. A parked Cargo process does not retain a
live compiler context. A query value saved to disk is not a generation-safe live
HIR/MIR graph. Neither mechanism alone establishes function-granular frontend
invalidation. Native resident frontend work still needs explicit ownership of
source maps, spans, diagnostics, macro external inputs, query identities and
arena lifetimes, with edit/error/recovery gates rather than pointer reuse across
destroyed sessions.
