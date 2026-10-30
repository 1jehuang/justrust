# Resident semantic subphase telemetry

## Scope and contract

This source-only change adds actual caller-thread intervals to the existing opt-in
resident diagnostics telemetry. It is instrumentation, not a semantic traversal
optimization or a subquery performance claim. The accepted flat helper and outer
phase instrumentation remain in place. In particular, the existing syntax walk's
`descendants().filter(|_| bodies.is_none())` expression is unchanged. The rejected
selective traversal guard is not restored.

`semantic_diagnostics_of_bodies_with_timings` has the same diagnostic arguments as
`semantic_diagnostics_of_bodies`, plus
`Option<&mut Vec<(&'static str, std::time::Instant, std::time::Instant)>>`.
Both share the existing `semantic_diagnostics_asked` body. All old APIs pass
`None`. With `None`, the recorder neither reads the clock nor allocates interval
storage. With `Some`, it appends actual start/end Instants measured synchronously
inside that body. Existing vector contents are retained. No trace dependency,
query, cache key, dependency, diagnostic policy, or body-selection policy changes.

## Five boundaries

| Label | Measured work |
| --- | --- |
| `diagnostics.semantic.prepare_parse` | Construct Semantics, attach/unpack edition, initialize diagnostics and parse file |
| `diagnostics.semantic.syntax_walk` | Existing syntax descendant walk and syntax handlers, including its unchanged selective filter |
| `diagnostics.semantic.module_context` | Module lookup, channel/crate/display-target lookup and DiagnosticsContext construction |
| `diagnostics.semantic.collect` | Existing module diagnostic collection, including actual selection callbacks, or unlinked-file handler |
| `diagnostics.semantic.render_and_filter` | Diagnostic rendering, disabled/experimental filtering, lint handling and macro-origin filtering |

Adjacent intervals share the actual transition Instant. A caller-local recorder's
Drop captures the final phase. If the pre-existing no-crates early return is
reached in module-context setup, only the three phases actually reached are
emitted. No empty collection or rendering intervals are invented. Unwinding can
likewise record only reached work, but does not return a successful diagnostic
vector. These are coarse inclusive phase measurements, not per-query timings.

## Resident transport

The existing startup-only exact `RA_JIT_DIAGNOSTICS_STATS=1` gate is the only
switch. Enabled callers use the new API, then convert its actual intervals with
`trace::span` on the same caller thread. Disabled callers use the original API
and allocate no interval vector. Every converted interval is pushed through
`DiagnosticsDetails::push`, retaining its existing 4000-span cap and saturating
drop count. There is no duration threshold and no synthetic positioning.

`diagnostics.semantic_inclusive_asked`, the existing outer phases, and the summed
selection-callback intervals are unchanged. The five subphases sit inside that
semantic outer interval, and callbacks sit inside `collect`. These levels are
inclusive, not additive. Never sum an outer duration with its children or add the
callback sum to the collect duration.

## Native regression gate

The native test filter is `resident_semantic_subphases`. It selects **5 tests**:

1. `tests::resident_semantic_subphases_all_bodies_exact`
2. `tests::resident_semantic_subphases_no_bodies_exact`
3. `tests::resident_semantic_subphases_selective_bodies_exact`
4. `tests::resident_semantic_subphases_disabled_diagnostic_exact`
5. `tests::resident_semantic_subphases_disabled_clockfree_api_exact`

The first four call both actual APIs, compare complete Debug representations
(the Diagnostic type has no PartialEq), compare exact callback visitation,
exercise all/rejected/alternating selection and disabled diagnostic filtering,
and verify all five labels, append behavior, real enclosing timestamps,
contiguous ordering, positive collect duration and actual callback containment.
The fifth compares the actual disabled optional API with the old API and checks
that the disabled recorder has neither a start Instant nor an output across
all transitions. Source inspection verifies every newly added clock read is
behind the optional recorder's enabled state.

## Validation ownership

No native build, test, workload, benchmark, lockfile resolution, or commit was
run by this source worker. Source formatting and diff hygiene are not native
validation. Root subsequently performed the native acceptance below from its
exact 2444-file snapshot with private Cargo.lock resolution, without changing
the live nested lock. No speedup is claimed.

The handoff bundle is
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/semantic-subphases-v25/`.
It contains the three original source files, the original absence marker for
this new document, four frozen full files, a four-path patch, SHA-256 manifests,
and readiness details. Frozen sources are the root gate inputs.

## Actual native acceptance, October 30, 2026

Root reported all native acceptance gates passing against the frozen v25 source:

- Original resident check and build both exited 0.
- The exact 2444-file source snapshot used a privately resolved lockfile and
  preserved every source file. All **5 actual owned ide-diagnostics API tests**
  passed in run `20261030-133307142-3479390`.
- Focused native gates passed with test counts `2 + 3 + 1 + 1 + 3`.
- With telemetry disabled, RSB16/RSA22 runtime, error and recovery gates passed.
- The full native suite passed **351 tests** in run
  `20261030-133629708-3652589`.

The actual positive profile receipt is
`/home/jeremy/.jcode/scratch/resident-semantic-subphases-v25-profile-20261030/state.json`.
It records `status: passed`, `unchanged: true`, and six successful rows. Every
row contains all five semantic subphase labels and the existing outer labels,
with **zero transport drops**. Durations below are actual inclusive observations
in milliseconds, using the receipt's zero-based row numbers:

| Profile row | prepare_parse | syntax_walk | module_context | collect | render_and_filter |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.011 | 0.255 | 0.001 | 254.759 | 1.000 |
| 2 | 0.013 | 0.268 | 0.001 | 72.131 | 1.103 |
| 3 | 0.014 | 0.251 | 0.002 | 112.705 | 1.216 |
| 4 | 0.013 | 0.242 | 0.001 | 97.967 | 1.238 |
| 5 | 0.018 | 0.241 | 0.001 | 100.109 | 1.577 |

Each of rows 1 through 5 retained nine spans and reported 373 selection callbacks.
The first row (row 0) instead measured **two files on distinct caller threads**,
with callback counts **373 and 231**, 17 retained spans, and 249 stale spans
excluded by the existing collection window. Its two `collect` intervals were
116.165 ms and 148.160 ms. They must not be presented as a single-file measurement
or summed into critical-path time. Stale exclusions are not transport drops.
The transport retains microsecond precision, so a recorded zero-microsecond
module-context interval in row 0 is not evidence that no native work occurred.

Collection dominates these observed per-file semantic intervals. This is phase
attribution only, not evidence of a faster implementation or a subquery-level
cause. Callback sums, semantic outer intervals and child phases remain inclusive,
not additive. The profile's noise proxy explicitly does not establish a quiet
machine. The disabled five-replicate comparison's raw -1.60% change was below the
acceptance floor and is **not a speedup claim**. No general performance or default
promotion claim follows from these bounded correctness and telemetry gates.
