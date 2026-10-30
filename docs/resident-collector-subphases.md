# Resident collector subphases (v26)

Source-only attribution against accepted `90164925ce021dbb56a735a7d99efacab261d732`.
No cache, semantic, scheduling or diagnostic selection optimization is included.
Native builds, tests and representative timing gates belong to the root coordinator.

## Output and API

`hir::Module::diagnostics_with_collector_timings` is the new optional HIR entry point.
It accepts the same optional body predicate used by the existing collector and an
optional caller-owned `Vec<hir::diagnostics::CollectorTiming>`. Each row has a static
`label`, `calls: u64`, and `cumulative_ns: u128`. Existing diagnostic APIs explicitly
pass `None`. The existing optional timed IDE entry point
`semantic_diagnostics_of_bodies_with_timings` gains a separate optional inventory
argument. Its five outer actual `(static label, Instant, Instant)` intervals remain
unchanged, including callback time inside collection. Untimed IDE APIs pass `None`
for both outputs.

The existing startup-only `RA_JIT_DIAGNOSTICS_STATS=1` gate requests both outputs.
Main emits one stderr line per observed category:

```text
diagnostics collector: label=hir.collect.item_case calls=42 cumulative_ns=12345 inclusive=true
```

The example numbers illustrate format only, not measurements. Inventory rows never
become trace events. Main transports the five outer actual intervals through its
existing bounded details collector on the same caller thread as before. No new
per-body spans consume the 4000-event transport cap. Disabled main takes its old
untimed branch and does not lock the details collector for this instrumentation.

## Bounded and clock-free implementation

An enabled top-level HIR walk creates a local fixed array of 16 `Cell` counters.
Each RAII guard reads an actual start clock, then measures elapsed nanoseconds at
scope exit and increments its category. A shared reference to non-`Sync` `Cell`
storage prevents guards from crossing threads. Nested guards never hold dynamic
borrows or locks. Only nonzero counters are merged into the caller's vector once
the walk returns. A fresh vector contains at most 16 rows regardless of module,
item or body count. Reusing that vector accumulates the same labels, not more rows.
Unrelated preexisting caller rows are retained. Counts and durations saturate at
their integer maxima. No event cap, record loss, or dropped-event timeline is
involved in the inventory.

With `None`, no counter array is initialized, no inventory allocation occurs, no
collector mutex is used, and neither entering nor dropping a guard reads a clock.
The disabled helper test supplies a panicking clock closure to verify that the
entry helper does not call it. The guard's absent target also bypasses its exit
clock. The root-owned native acceptance below exercises these source invariants.

## Category boundaries

All categories are **inclusive**. They may nest or recursively include themselves.
Do not sum them as wall time, derive a critical path from their sum, or subtract
nested totals without accounting for recursive and cross-category overlap.

| Label suffix (`hir.collect.`) | Actual scoped work |
| --- | --- |
| `module_inclusive` | Entire `collect_module`, including nested modules, definitions, macros, impls and selected bodies |
| `item_case` | Case/naming query and appending its diagnostics |
| `signature_setup_inclusive` | Generic signature store/source acquisition and generic checks for functions, consts, statics, ADTs, aliases and traits, ending before their selected-body/field/wellformed/associated-item work |
| `signature_generic_inclusive` | Signature expression store diagnostics, generic lowering diagnostics and anonymous-const checking |
| `signature_fields_inclusive` | Variant field store/source acquisition, expression diagnostics, lowering and anonymous-const checking |
| `wellformed` | Wellformed query and mapping/appending its diagnostics |
| `impl_inclusive` | Entire impl collection, including signature, associated items, selected bodies and impl checks |
| `impl_checks_inclusive` | Work after associated-item collection: impl source/trait metadata, coherence, orphan, safety, missing/redundant associated items, and the impl wellformed check |
| `selected_body_inclusive` | Only accepted bodies, beginning **after** the body predicate, including body/source acquisition through borrowck diagnostic mapping |
| `body_expr_inclusive` | Selected-body expression store diagnostics, including block-module recursion |
| `body_inference_inclusive` | Selected-body inference query and diagnostic mapping/appending |
| `missing_unsafe` | Selected-body missing-unsafe query and all its diagnostic mapping/appending |
| `body_validation` | Selected-body validation query and diagnostic mapping/appending |
| `body_borrowck` | Selected-body borrowck query and diagnostic mapping/appending |
| `signature_anon_inference_inclusive` | Anonymous-const type-owner acquisition, inference query and mapping/appending from signature/field lowering |
| `signature_anon_borrowck` | Anonymous-const borrowck query and diagnostic mapping/appending from signature/field lowering |

`signature_setup_inclusive` is not a partition covering every signature read.
For example impl signature acquisition is included in `impl_inclusive`, before
`impl_checks_inclusive`. Generic checks themselves have their own category.
`body_inference_inclusive` measures the collector's inference request, not all
possible inference induced by downstream checks. Query dependencies and cache hits
remain part of whichever actual operation invokes them. Block-module recursion can
cause body expression work to include nested signatures and bodies.

Start by comparing item/signature/wellformed totals with selected-body inference,
unsafe, validation and borrowck totals, using counts to explain repeated work.
Use the actual outer collection interval as the wall-time denominator. These
observations guide further profiling, not an automatic caching prescription.

## Correctness gates

The distinct `resident_collector_subphases` filter includes:

- Actual IDE API old/new complete Debug comparison and exact callback visitation
  for all, none and selective body selection, plus disabled diagnostic filtering.
- Actual HIR API old/new complete Debug comparison and exact callback visitation
  on naming, generic/anonymous-const, impl and nested-module fixtures, with inventory
  enabled and disabled. It also compares the unfiltered old/new HIR APIs.
- Positive observed category/counter evidence and selected-body phase counts.
- Disabled API comparison and the HIR disabled clock-helper assertion.
- A helper-only repeated-scope test showing inventory cardinality remains bounded
  and repeated append merges counts rather than appending duplicate labels.

The existing five `resident_semantic_subphases` tests remain, including exact outer
labels, contiguous actual intervals, sentinel preservation, and callback containment.
The implementation lane executed no build, test or benchmark before its source-only
handoff. Root subsequently executed the bounded acceptance described below.

## Root-owned bounded acceptance (2026-10-30)

The root's `final-v26-snapshot.exit` is `0`. Against its frozen native snapshot:

- Six actual IDE collector API tests, two HIR helper tests and the five existing
  outer-phase API tests passed.
- Focused resident gates passed with counts 2/3/1/1/3, and the complete native
  engine suite passed all 351 tests.
- Positive telemetry inventory admission passed all six runtime rows. Normal edit
  rows selected all 373 bodies. The reported nested inclusive category durations
  are not additive and do not establish a critical-path partition.
- Strict disabled-runtime RSB17/RSA23 comparison reported an engine median of
  99.526 ms before versus 106.166 ms after: +6.640 ms, or +6.67% raw. The
  reference-normalized engine/reference ratio changed from 0.901716 to 0.915673,
  a relative +1.55%. The absolute median delta did not exceed the requested 10%
  floor. This descriptive, uncontrolled scoped diagnostic has unknown noise and
  provides neither a significance verdict nor a speedup claim. It is not the
  comprehensive workload or a default-promotion gate.

Root evidence was read from
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/`: the final exit marker,
`candidate-unit-v26-snapshot/` gate exits, `suite-v26-snapshot-compare.log`, and
`profile-semantic-subphases-v26.log` (`status=passed`, `rows=6`). Root retains
ownership of source snapshot admission and further benchmark regression gates.
No caching decision, performance optimization, speedup, or exclusive category
dominance is inferred from this bounded acceptance.
