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
* There are no time sums, including nested wall spans. CPU work, elapsed savings,
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
Its report is reproduced below, formatted without changing fields or values.

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
