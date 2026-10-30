# Scalar value retention on a real pinned project

The tracked, default-off `-Zjr-cache-small-query-values` experiment was rechecked
on the current frozen driver
`8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`.
All 22 native acceptance cases passed, including ordinary/const/async/closure,
owner and macro changes, cross-crate changes, intentional errors and recovery,
with incremental hash verification and matching diagnostics/status/runtime.
The exact report is retained in
`bench/results/real-scalar-retention-20261030/native-acceptance.json`.

## Actual workload and policy

The real source is the same pinned `f97fcc2` justrust library as the prior
`provider-cost-20261030` baseline. The client, Cargo, compiler executable and
driver hashes match exactly. Source hashes match for each compared epoch.
Normalized flags match except `jr-cache-small-query-values=no/yes` and the
private profile output root. Associated-item and lint experiments are disabled.
Both use `default,function-args` self-profiling and incremental hash verification.
Private target directories are independently prepared and kept warm.

The candidate sequence is prepare, forced unchanged compilation, the identical
one-line `std::hint::black_box(123u64);` edit in `src/runs.rs::ago`, and restored
source compilation. All compiler exits are 0. Original strict inventory decodes
fail on unmapped virtual labels and retain status 2 separately from successful
compiler status. Explicit supported unknown-label decoding is used only for the
observation comparison. Every Query interval in compared captures has a resolved
label. Other unresolved events remain represented and counted. No records are
silently discarded and no invocation-local numeric IDs are joined across runs.

## Observed mechanism

All numbers below are records in these captures. An absent marker is unknown,
not a general zero-work assertion. `Query` counts show provider intervals,
`IncrementalLoadResult` records show disk-result loads, and named native green
markers distinguish proven-green acquisition from actual provider fallback.

| Capture | Kind | Provider intervals off/on | Disk-load intervals off/on | Proven-green provider fallback off/on |
| --- | --- | --- | --- | --- |
| Unchanged | `hir_owner_parent_q` | 965 / not observed | not observed / 965 | 965 / absent |
| Unchanged | `constness` | 2,431 / not observed | not observed / 2,395 | 2,431 / absent |
| Unchanged | `coroutine_kind` | 2,729 / not observed | not observed / 2,729 | 2,729 / absent |
| Body edit | `hir_owner_parent_q` | 3,291 / 14 | not observed / 3,277 | 3,277 / absent |
| Body edit | `constness` | 2,560 / 121 | not observed / 2,439 | 2,545 / 106 |
| Body edit | `coroutine_kind` | 2,751 / 14 | not observed / 2,737 | 2,737 / absent |

This extends the earlier tiny-fixture result: real candidate warm builds do load
retained scalar values instead of running thousands of selected providers.
It does **not** establish identical demand for every key: unchanged constness
acquisition counts differ by 36. We do not equate all count differences with
avoided calls or infer a global reuse fraction.

There are also **106 residual proven-green constness fallbacks** in the edited
candidate capture. The local retention policy is not universal: the native
`QueryVTable::will_cache_on_disk_for_key` excludes external keys for queries with
separate external providers, and eligible local disk loads may also miss.
The present markers do not distinguish those causes, so these 106 records are
not claimed to be all external keys or all codec misses. More retention would
need a reason-specific measurement and correctness gate, not a blanket bypass.

## Storage and limitations

At the matched post-edit state, before compiling restoration, the main query
cache is 13,539,731 logical bytes off and 13,687,486 on, a **147,755-byte increase**.
Allocated sizes are 13,541,376 and 13,688,832 bytes, a 147,456-byte increase.
These are one-state observations, not universal overhead or isolated codec size.
Paths and hashes are retained in `comparison.json`. The comparison recipe now
refuses candidate storage resampling after source restoration, preventing a
restored cache from being mislabeled as the edited state.

This is not a wall-time comparison. Instrumentation, independent roots and
concurrent workloads make it unsuitable for a causal speed claim. Prior matched
Desktop measurements were uncontrolled, and the comprehensive performance gate
remains blocked by pinned test failures. The flag stays **default off**. No
maximum reuse, CPU reduction, or saved-time claim follows from this workflow.
