# Real TT-cache eligibility diagnostic, October 30, 2026

`dp41` measures the bounded, default-off declarative cache on the pinned
`f97fcc2` justrust library. This is mechanism evidence, **not a latency benchmark
or a time-saved claim**. The compiler includes the audited raw TT repetition
admission in `87ab34640`. Procedural macro caching, associated-items caching,
small-query-value caching and the empty-lint-level fast path were disabled.

| Workflow | Eligible / lookup / success | Provider executions | Ineligible |
|---|---:|---:|---:|
| Prepare | 633 / 633 / 633 | 633 | 1,329 |
| Body edit | 634 / 634 / 634 | 1 | 1,330 |
| Restore | 633 / 633 / 633 | 0 | 1,329 |

These are exact resolved GenericActivity interval counts from the selected main
justrust crate profiles, not inferred macro kinds or percentages. All three
profiles have zero unresolved records. Both warm workflows have 633 lookups
without provider execution. The body edit introduces one additional eligible
lookup and one provider execution. The observed edit output includes the unique
`JR_RUNTIME_EXECUTED_39` marker. Each process actually executed the same four
selected tests successfully, without hotpatch fallback. All 63 pinned source
files were restored and recorded tool identities were unchanged.

The previous literal-only diagnostic admitted 106 expansions in each workflow.
The larger observed admission count motivates investigation but does not establish
that admitted expansions are expensive. Lookup, hashing, span/hygiene keys,
token copying and disk decoding still have costs. The earlier literal-only
`br37` timing comparison did not show a latency win. This run has no paired
flag-off baseline, no comprehensive session-weighted comparison, and unknown
machine contamination. Its elapsed times must not be used as a speedup claim.
The flag remains off.

## Identity and provenance

The frozen compiler driver SHA256 is
`ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595`.
The launcher SHA256 is
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
These match the compiler used by the native correctness and independent literal
and TT hit gate. See `docs/native-declarative-tt-cache.md` for the deliberately
narrow side-effect audit and contract.

The exact recorded runs are:

* Prepare: `20261030-051823510-2375716`
* Body edit: `20261030-051853305-2385022`
* Restore: `20261030-051857330-2386556`

Original state, runner, all three raw-output journals and stdout/stderr receipts,
three losslessly compressed main-crate profiles, and original offline interval
scripts/results are retained in
`bench/results/declarative-tt-real-20261030/`. Dedicated path-redacted exports
containing only those three runs are under
`data/runs/xps14-declarative-tt-real-20261030/`. No global export file was modified.
Original scripts retain absolute private paths as provenance. They are not a
portable rerun promise. The portable validator reads only committed artifacts
and the repository's Python profiling helpers, without the original scratch
paths, compiler, build products or user home.

## Descriptive persistence intervals

The retained `persistence-intervals.json` contains per-thread interval unions for
incremental persistence activities in all three profiles.
`encoding-query-intervals.json` further splits edit-profile encoding/recomputation
activities by query argument. The validator independently recomputes both from
the raw profiles, using the original scripts' grouping and interval-union rules.

For example, the edit profile reports `incr_comp_persist_result_cache`
139.570204 ms, `incr_comp_serialize_result_cache` 86.538629 ms,
`encode_query_values` 80.791455 ms, and `incr_comp_encode_dep_graph` 52.050400 ms.
These are **nested or overlapping descriptive intervals**, not independent
additive costs. They cannot be summed into wall time, attributed wholly to the
declarative cache, or interpreted as recoverable time. Query encoding intervals
are not measurements of time saved by an alternative representation.

## Offline replay

```console
python3 bench/results/declarative-tt-real-20261030/validate.py
python3 bench/results/declarative-tt-real-20261030/test_validate.py
```

Replay verifies the immutable-artifact SHA256 manifest, unchanged source and tool
identity receipts, exact independent expected marker counts, decompressed profile
hashes, full metadata/record totals, raw test outcomes and edit execution marker,
both interval reports, and the dedicated export's exact run IDs and test results.
Mutation tests reject missing workflows, unrestored source, changed driver,
incorrect 633/634 admission or provider counts, enabled procedural caching,
process failure, and an edit that did not alter source. Retention and validation
run no native compiler or test workloads.
