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
