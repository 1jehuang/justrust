# Reuse evidence inventory, not a ceiling

`bench/reuse_ceiling.py` is an offline first measurement step. Its filename names
the investigation, not a calculated theoretical maximum. It does not change
compiler execution and makes no speedup claim.

## Inputs and meaning

The required `--profile` is an original MMPD format-9 capture, read and streamed
through `query_profile.py`'s bounded decoder. No decoded JSON is trusted as raw
capture authority. Output retains the original SHA-256 and byte count, but not
private commands or labels. Optional `--large-capture` and
`--allow-unmapped-event-labels` have the existing decoder's explicit opt-in
meaning. Strict decoding remains the default.

* `Query` intervals are counted by invocation-local numeric ID, not query-kind
  label. Repeated records beyond the first per ID are **hypothetical record
  candidates for investigation**, not proven avoidable execution. Zero candidates
  says nothing about redundant computation across different keys or sessions.
* `QueryCacheHit` instant records and `QueryCacheHitCount` integer **values** are
  separate overlapping channels. Counter record count is not hit count. The
  channels are never added. Shared-ID disagreements are reported, not repaired.
  These describe in-memory hits, not cross-session reuse. The source authority is
  `toolchain/rustc/compiler/rustc_data_structures/src/profiling.rs`, especially
  `query_cache_hit` and `increment_query_cache_hit_counters`.
* `IncrementalLoadResult` intervals are counted separately. They describe result
  loading, not every green query or all provider avoidance across sessions.
* Missing provider observations for hit IDs are coverage facts, not proof of
  disk reuse. Filters, request denominator, losses and capture completeness
  remain unknown. No hit-rate percentage is inferred.
* There are no duration sums, including nested wall spans. Native markers below
  additionally support per-thread elapsed interval unions. CPU work, elapsed savings,
  execution span, semantic avoidability and maximum reuse stay null.

Optional `--before-graph BEFORE --after-graph AFTER` accepts only the pinned
stock binary format supported by `query_graph.py`. Compiler revision and command
hash must agree. It compares `(kind, key_fingerprint)` with duplicate keys
rejected, independently of serialization indices. It inventories matching and
changed value fingerprints, added and removed keys. Identical bytes are marked.
The caller must establish ordering, crate identity and session continuity.
**Even matching fingerprints do not prove execution was unnecessary.** Neither
graph execution status nor cross-session reuse is inferred. The graphs are not
joined to the profile: numeric IDs cannot cross invocations. Unsupported current
fork graphs are deliberately rejected instead of guessed.

I also inspected `frontend_mapping.py`: its native receipts use a separate
validator, capture epochs, admission and wait/read coverage. A receipt's cache
result availability is not interchangeable with stock profile query identity or
cross-session semantic eligibility. This tool does not invent that join.

## Retained-data demonstration (2026-10-30 UTC)

Exact command, run from repository root:

```sh
python3 bench/reuse_ceiling.py --profile /home/jeremy/.jcode/scratch/toolchain-profile-20261026-crab-client/traces/syn-3560712.mm_profdata --allow-unmapped-event-labels
```

The original retained capture is local, not committed. SHA-256:
`1d25795172c75d893f16ac264015bf865bf61dcdfce2e8b1eabcf99177a21f8e`.
Its original report at commit `c363484a3` is reproduced below, formatted without
changing fields or values. The current tool additionally emits `native_green_reuse`.

```json
{
  "schema": 1,
  "profile_sha256": "1d25795172c75d893f16ac264015bf865bf61dcdfce2e8b1eabcf99177a21f8e",
  "profile_bytes": 31896191,
  "graph_comparison": null,
  "profile": {
    "records": 797693,
    "event_kind_records": {"ArtifactSize": 50, "GenericActivity": 173753, "Query": 376681, "QueryCacheHitCount": 247209},
    "unresolved_label_records": 0,
    "observed_provider_interval_records": 376681,
    "distinct_provider_ids": 376681,
    "repeated_provider_ids": 0,
    "hypothetical_repeat_provider_record_candidates": 0,
    "in_memory_hit_instant_records": 0,
    "distinct_in_memory_hit_ids": 0,
    "hit_ids_without_observed_provider": 0,
    "in_memory_hit_counter_records": 247209,
    "in_memory_hit_counter_value_sum": 8306563,
    "counter_ids_without_observed_provider": 19237,
    "hit_ids_in_both_channels": 0,
    "hit_ids_with_channel_count_disagreement": 0,
    "incremental_load_interval_records": 0,
    "coverage": {"capture_complete": null, "request_count": null, "enabled_filters": null, "cross_session_reused_query_count": null, "semantic_avoidable_query_count": null},
    "cpu_work_ns": null,
    "time_saved_ns": null,
    "execution_span_ns": null,
    "theoretical_maximum_reuse": null
  },
  "limitations": [
    "Capture filters, dropped instrumentation and unrecorded requests are unknown. Absence is not zero execution or zero reuse.",
    "QueryCacheHit instant records and QueryCacheHitCount integer values describe overlapping in-memory hits and must not be added.",
    "IncrementalLoadResult intervals record disk-result loading, not all cross-session reuse or provider avoidance.",
    "Repeated Query records for an invocation-local ID are candidates for investigation only, not proven redundant provider executions.",
    "Matching graph fingerprints do not establish semantic dispensability, execution, cache eligibility, or sound reuse.",
    "No nested wall spans are summed. Counts are not time saved, CPU work, execution span, or a theoretical maximum."
  ]
}
```

Observed: 8,306,563 reported in-memory hit counter increments versus 376,681
provider interval records, with no repeated provider IDs. Counter records for
19,237 IDs have no observed provider interval, an unresolved coverage fact, not
proof of cross-session reuse. This is not a
complete-request denominator or reuse ceiling. There is no graph-pair or
cross-session claim from this demonstration. To progress toward sound additional
reuse requires ordered edit-pair provenance, stable query-key mapping, explicit
execution/cache routes and semantic eligibility authority, then separate
nonoverlapping cost evidence. Unchanged results alone are insufficient.

## Tests

```sh
python3 -m unittest discover -s bench -p 'test_reuse_ceiling.py'
```

Output: `Ran 11 tests`, `OK`. Fixtures cover overlapping hit channels, missing
coverage, repeated and nested provider records without time sums, disk-load
separation, unresolved labels, counter disagreements, invalid event types,
duplicate counters/graph keys, index-independent graph matching, command
mismatch, identical snapshots and the public binary-input CLI's failure status.
The full regression command `python3 -m unittest discover -s bench -p 'test_*.py'`
also passed: `Ran 435 tests in 10.611s`, `OK`.

The existing decoder suites own raw format and filesystem-boundary validation.

## Native proven-green recomputation follow-up

Compiler instrumentation commit `7cc77351b` adds two `GenericActivity` interval
markers in `load_from_disk_or_invoke_provider_green`:

* `jr_incremental_green_query`: entered only on the native already-green
  dependency-node route. Includes loading or recomputation and subsequent
  verification. This is not every query request or every green dependency node.
* `jr_incremental_green_recompute`: the `None` disk-result branch around provider
  invocation. This is observed provider recomputation despite an already-green
  node, a **resident-value-reuse investigation opportunity**, not proof the value
  can be retained safely or the interval removed in full. It does not distinguish
  nonserializable values from disk misses and does not give stable query keys.

The additive `profile.native_green_reuse` object counts each marker independently,
groups counts by safe query-name argument when available, and computes interval
**unions separately per profiler thread and marker**. Nested, crossing, identical,
touching and zero-length intervals cannot inflate a union by duration summation.
No sum across threads or marker classes is reported. These are elapsed coverage
windows, not CPU work or time saved. The green outer marker and recompute inner
marker overlap and must not be added. Profiler threads are not proven OS TIDs.

`query_profile` deliberately exposes only the label before `\x1e`. The diagnostic
resolves the original marker event string with `strings.resolve(event_id)` and
uses the second component only when it is one safe Rust-style query name. Missing
or unexpected arguments count as unknown, rather than guessing from labels.
When a marker is absent its count is **null**, with `unknown-marker-absent`, not
zero. Enabled marker filters, completeness, semantic avoidability and saved time
remain unknown even when some marker intervals are present. The input hash pins
bytes, not compiler provenance. Marker interpretation relies on the caller's
trusted native-instrumented capture, not attestation of an arbitrary input file.

Exact commands retaining compact reports, from repository root:

```sh
python3 bench/reuse_ceiling.py --profile /home/jeremy/.jcode/scratch/reuse-ceiling-real-20261030/profiles-macro-cache/justrust-1486494.mm_profdata --allow-unmapped-event-labels > bench/results/reuse-ceiling-20261030/macro-cache.json
python3 bench/reuse_ceiling.py --profile /home/jeremy/.jcode/scratch/reuse-ceiling-native-20261030-v5/unchanged/reuse_fixture-1441144.mm_profdata --allow-unmapped-event-labels > bench/results/reuse-ceiling-20261030/unchanged.json
```

Both exited 0. The committed JSON files are the complete exact stdout reports,
7,385 bytes combined. Original private profiles are not committed. Observations:

| Capture | Green-route records | Recompute records | Green-route thread-3 union ns | Recompute thread-3 union ns | Marker records missing query name |
| --- | ---: | ---: | ---: | ---: | ---: |
| Warm justrust macro-cache | 119,322 | 70,506 | 248,143,496 | 110,519,125 | 0 / 0 |
| Unchanged small fixture | 712 | 602 | 2,828,751 | 1,595,160 | 712 / 602 |

The application capture has 68,987,990 bytes and SHA-256
`89becb2c1b2cb0d851a55bbf61fd05391450e7337efe20a93649cbd8986adbc9`.
It contains 1,922,417 raw records, including 268,111 unresolved event labels
explicitly permitted by the opt-in. Marker arguments are nevertheless all
resolved. The fixture has 294,058 bytes, SHA-256
`1b06c8ca2eb3f1719c99a531d93fa49f53333be4e8f520fc0eb35bb5c9eec898`,
8,352 records and 139 unresolved labels. Its marker names are resolved but query
arguments are absent. These are separate captures and epochs, never combined.

This demonstrates native green-provider recomputation, not a theoretical maximum
or a measured speedup. The native results do not reclassify all unchanged
fingerprints as unnecessary execution. No compiler or client file was changed by
this diagnostic follow-up.

Follow-up validation: `python3 -m unittest discover -s bench -p
'test_reuse_ceiling.py'` reported `Ran 16 tests`, `OK`; the complete command
`python3 -m unittest discover -s bench -p 'test_*.py'` reported
`Ran 440 tests in 10.271s`, `OK`. New fixtures exercise nested/crossing/duplicate
intervals, independent threads, absent markers, unknown/safe query arguments,
invalid marker types/timestamps and real MMPD CLI argument decoding.

## Optional empty lint-owner branch inventory

`profile.optional_branch_inventory` is an additive, optional inventory separate
from `profile.native_green_reuse`. Its `OPTIONAL_BRANCH_MARKERS` family currently
contains only `jr_empty_lint_level_fast_path`, the default-off native
`GenericActivity` interval inside the guarded empty lint-owner return in
`compiler/rustc_lint/src/levels.rs`. It counts observed entries into that branch,
not green-route queries, provider recomputations, distinct owners, or avoided
work. It requires no query-name argument and does not resolve one.

Each optional marker reports `interval_records` and `per_thread` counts and
`union_elapsed_ns`, with overlap merged independently within each profiler
thread and capture. No cross-thread or cross-marker duration total is emitted.
Kind must be `GenericActivity`, record type must be `interval`, thread must be
a nonnegative integer, and both endpoints must be nonnegative integers with
end at least start. Booleans are not valid integers for this contract.

Absent means `evidence: unknown-marker-absent`, a null count and an empty
per-thread list, never a claim of zero branch execution. Capture completeness
and filter enablement remain unknown. CPU work, time saved and critical-path
duration remain null even when intervals are present. This optional family
does not alter the existing two `NATIVE_MARKERS`, their fields, interpretation,
or acceptance requirements. It is not required to accept green-route evidence.
There is no default promotion, broad performance conclusion or speedup claim.

Validation: `python3 -m unittest discover -s bench -p 'test_reuse_ceiling.py'`
reported `Ran 20 tests`, `OK`. The four additional tests cover absence and native
field compatibility, wrong kinds/types/threads/endpoints, nested and crossing
interval unions on multiple threads, no argument resolution, and a real binary
CLI fixture without arguments. No native workloads were run for this change.
The complete Python regression command `python3 -m unittest discover -s bench
-p 'test_*.py'` also passed: `Ran 493 tests in 16.918s`, `OK`.

## Retrospective re-execution result inventory

`native_reexecution_results` is an independent optional inventory for six native
classification markers. Each has one query-kind name argument when function
arguments are enabled:

* `jr_incremental_reexecuted_same_value`
* `jr_incremental_reexecuted_different_value`
* `jr_incremental_reexecuted_no_hash`
* `jr_incremental_eval_always_same_value`
* `jr_incremental_eval_always_different_value`
* `jr_incremental_eval_always_no_hash`

These describe a provider that already executed and a stable query key found in
the previous graph. Ordinary and eval-always categories remain separate. Equality
compares its current result fingerprint with the previous result fingerprint.
`no_hash` means equality is unknown, not different. New keys are outside this
inventory. A same-result observation does **not** establish that inputs changed,
that the provider could have been skipped prospectively, or that narrower
dependencies would be sound. It is distinct from acquiring a proven-green value.

The decoder reports observed interval-record counts, safe query-name counts and
unknown argument counts per marker. Absent marker counts remain null/unknown.
Arguments outside a single identifier are not exposed as query names. Marker
shape validation rejects non-interval/non-generic activity, invalid thread IDs
or invalid interval endpoints. Durations are deliberately not reported: these
markers cover post-provider classification, **not provider execution**.
Changed-input counts, avoidable calls, provider elapsed time, CPU work and time
saved remain null. No IDs are joined across invocations and no global reuse
percentage or ceiling is inferred.

Five additional focused regressions pass, bringing this module to 25 tests.
They cover absent/unknown fields, separate families/outcomes, malformed argument
and interval shapes, preservation of earlier inventories, and a real binary CLI
fixture. Native acceptance and actual capture evidence are tracked separately in
`reexecuted-value-evidence.md`; synthetic decoder tests alone are not native proof.

The five strict real-capture reports and native positive/negative receipts are
retained in `bench/results/reexecuted-value-20261030/`. A separate same-source-path
profile-off/on native gate also passed all five cases, including binary artifact
equality for successful compilations. Full benchmark Python discovery passed
508 tests in 9.617 seconds. These are correctness and observation checks, not a
performance comparison or an established maximum reuse fraction.
