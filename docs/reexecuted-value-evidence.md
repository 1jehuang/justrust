# Retrospective provider re-execution value evidence

This is measurement ladder 2 instrumentation, not a reuse change. The corrected
native markers classify an existing-key query result **after a query provider
actually returns**, using its normal optional stable result fingerprint. New
keys are excluded. Feeding an already supplied value into the cache and executing
non-query tasks must not produce provider-reexecution evidence.

## Native caller boundary

`hash_result_and_alloc_node` is a shared allocation path, not itself a provider
boundary. Its callers include both task execution and `with_feed_task`. The
original instrumentation overlooked the feed caller. Its six legacy labels and
retained reports therefore describe **mixed hash/allocation-path outcomes**,
not proven provider executions. They must never be silently relabelled as
provider evidence.

The corrected implementation carries an explicit `provider_execution` boolean:

- `rustc_query_impl::execution::execute_job_incr` calls
  `DepGraphData::with_task` with `|| (query.invoke_provider_fn)(tcx, key)`.
  This entry point invokes `with_task_impl(..., true)`.
- `DepGraph::with_task`, used by codegen backends and metadata encoding for
  non-query task execution, invokes `with_task_impl(..., false)`.
- `DepGraph::with_feed_task` supplies a value rather than invoking a provider.
  It passes `false` directly to `hash_result_and_alloc_node`.
- Only `tcx.prof.enabled() && provider_execution` allows the extra previous-node
  lookup and classification. Normal task execution, result hashing, graph
  allocation/coloring, dependency edges and read-recorder release are unchanged.

The task closure has returned before the helper computes the existing optional
result fingerprint and classifies it. Proven-green provider recomputation uses
`with_ignore` and does not traverse this path. Its existing green-route markers
remain separate and unchanged. Eval-always providers are classified separately.

## Corrected marker contract

The new `GenericActivity` interval labels use a static dep-kind name as their
sole function argument:

| Ordinary provider | Eval-always provider | Meaning |
| --- | --- | --- |
| `jr_incremental_provider_reexecuted_same_value` | `jr_incremental_provider_eval_always_same_value` | Current and previous stable result fingerprints compare equal. |
| `jr_incremental_provider_reexecuted_different_value` | `jr_incremental_provider_eval_always_different_value` | Current and previous stable result fingerprints compare unequal. |
| `jr_incremental_provider_reexecuted_no_hash` | `jr_incremental_provider_eval_always_no_hash` | Current provider execution has no result-hash function. Value equality is unknown. |

The names come from `define_dep_nodes!` without formatting a key or allocating a
formatted query name. Profiling off adds no previous-graph lookup or result
hashing. The instrumentation does not change graph decisions, stable hashes,
serialized cache formats, cache keys, diagnostics, or exit status.

The original `jr_incremental_reexecuted_*` and
`jr_incremental_eval_always_*` labels are **legacy mixed allocation markers**.
The corrected compiler no longer emits them. The acceptance harness requires
the new provider-only labels, so a legacy-only capture cannot pass its evidence
gate. The offline analyzer may retain a separate explicitly mixed legacy field.

## Interpretation limits

Equal result fingerprints do **not** prove that inputs changed, a provider was
avoidable, an execution was redundant, or future reuse is sound. Result hashes
are the compiler's existing stable hashes, not a new semantic equality oracle.
No-hash events do not establish equal or different values. These counts describe
records, not distinct keys or providers that could safely be skipped. Absent
markers remain unknown, not measured zero work.

The intervals cover marker recording after provider execution. They do not
measure provider elapsed time, CPU work, critical path, or savings. No elapsed
bound, speedup, reuse default, or performance promotion follows from this work.
No hosted or comprehensive benchmark is part of this acceptance gate.

## Direct native acceptance

Run against a frozen compiler with its matching freshly built standard library:

```sh
python3 bench/reuse-reexecuted-accept.py \
  --rustc /path/to/frozen/sysroot/bin/rustc \
  --output /existing/private/parent/new-capture
python3 -m unittest discover -s bench -p test_reuse_reexecuted_accept.py -v
```

The harness runs profile-off and profile-on with independent persistent
incremental directories, both with `-Zincremental-verify-ich`. Profile-on uses
`-Zself-profile-events=generic-activity,function-args`, explicitly recorded in
the report. This includes the required marker family but not query-provider
intervals. In a failed compilation, unfinished query-provider intervals can
retain the native `EventId::INVALID` sentinel and the existing strict whole-profile
decoder rejects them. Selecting only generic activities avoids those unrelated
records without relaxing decoder integrity checks.

Ordered cases are initial, unchanged, actual body edit, type error, and recovery.
The harness compares exact compiler status and diagnostics, normalizing only
each source directory, and compares successful program status/stdout/stderr
against each other and expected output. A type error must return ordinary
compiler status 1. Profiles must contain valid generic intervals with a resolved
static dep-kind argument. An initial fresh incremental directory must not contain
prior-node markers. The fixture must observe both ordinary provider same-value
and different-value records across subsequent captures. Unobserved families
remain unknown.

Schema-2 `acceptance.json` explicitly records provider-only allocation origin.
It retains source text, commands, complete diagnostics, output, compiler `-vV`,
compiler SHA-256, sysroot query, exact rustc_driver paths/SHA-256, successful
binary hashes, native profile paths/hashes, and per-kind counts. Partial or
failed reports are retained. Existing output directories are never overwritten.
Raw profiles stay in the private capture directory.

## Corrected validation status

The twelve standalone tests cover all six provider-only labels, separate
eval-always counting, explicit capture filter, absent-marker unknowns,
repeated-record counts, malformed arguments/event shapes, missing outcome
coverage, fresh-directory rejection, edit/error/recovery ordering, preservation
of existing outputs, and failure-report provenance. Added regressions ensure
legacy mixed labels do not establish provider coverage and inspect the explicit
native caller flags and gate. The source-boundary check is structural, not a
substitute for a fresh native build and direct acceptance.

The coordinator's corrected compiler and matching std build passed in 3m11s.
The fresh frozen schema-2 native gate then passed all five cases with verify-ich,
matching full diagnostics/status and successful runtime output:

- Compiler: `/home/jeremy/.jcode/scratch/provider-profile-20261030/sysroot/bin/rustc`
- Launcher SHA-256: `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`
- Driver: `sysroot/lib/librustc_driver-e2bd696299265a4a.so`
- Driver SHA-256: `8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`
- Report: `/home/jeremy/.jcode/scratch/provider-native-accept-20261030/acceptance.json`
- Report SHA-256: `123992d2482f782056202c633eff875b097dedeb8b3de407da18d42b12b9c514`

| Case | Status off/on | Ordinary provider same/different/no-hash | Eval-always provider same/different/no-hash |
| --- | --- | --- | --- |
| Initial | 0 / 0 | absent / absent / absent | absent / absent / absent |
| Unchanged | 0 / 0 | 5 / absent / absent | 113 / absent / 2 |
| Body edit | 0 / 0 | 39 / 3 / 3 | 111 / 2 / 2 |
| Type error | 1 / 1 | 33 / 7 / 3 | 62 / 2 / 2 |
| Recovery | 0 / 0 | 39 / 3 / 3 | 111 / 2 / 2 |

All six provider-only marker classes were observed across the matrix. Totals are
ordinary `116 / 13 / 9` and eval-always `397 / 6 / 8`, strictly record counts,
not avoidable work. Absent entries remain unknown, not general zero-execution
claims. The successful runtime outputs were `9`, `9`, `10`, `9` with newlines;
the type-error retained matching full E0308 diagnostics. All twelve standalone
tests passed. Strict native-profile replay through the analyzer's separate
`native_provider_reexecution_results` field matched every evidence state,
interval count and per-dep-kind count in all five cases. Legacy mixed labels
were absent in these new captures. Conversely, replay of all five historical
mixed profiles preserved their legacy counts and found no provider-only labels.
Historical mixed capture success below does not validate the corrected
provider-only evidence contract. The changed counts reflect the corrected
measurement boundary, not a performance improvement.

The schema-2 harness was run against the frozen mixed driver `47caa852...` as a
new native negative control. All five profile-off/on diagnostic/status/runtime
comparisons passed, including type-error/recovery. It correctly failed the new
marker gate with `Missing native ordinary same_value evidence`. The immutable
failed report is
`/home/jeremy/.jcode/scratch/provider-only-legacy-negative-20261030/acceptance.json`,
SHA-256 `52f514f1afbdabb412e878f46b7756b5245a86e7d3b5f1773e9eaa3b2c994b80`.
No original mixed reports were overwritten or reinterpreted.

## Historical mixed allocation evidence, not provider evidence

Commit `46a2e24eb` emitted the original six mixed labels. A coordinated compiler
and matching std build took 176 seconds and produced this frozen compiler:

- Compiler: `/home/jeremy/.jcode/scratch/reexecuted-profile-20261030/sysroot/bin/rustc`
- Launcher SHA-256: `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`
- Driver SHA-256: `47caa85292e729553328c669d88631e0b90550a173609941aebcd14c0a3cf10c`
- Version: `1.101.0-nightly (23efcc929 2026-10-14)`, host `x86_64-unknown-linux-gnu`, LLVM `23.1.3`
- Retained schema-1 report: `/home/jeremy/.jcode/scratch/reexecuted-native-accept-20261030/acceptance.json`
- Report SHA-256: `b31ff98c723bcafa477f322bc5f70f37045ba7bbbc3293d58d4ea843b047ac59`

That report's historical passed verdict covers diagnostic/status/runtime
comparison and the original mixed-label gate only. Its provider-specific scope
fields were overclaims and are superseded by this correction, not rewritten in
the retained file. All five compiler comparisons passed, successful runtimes
returned `9`, `9`, `10`, `9` with trailing newlines, and type-error status was 1
with matching complete E0308 diagnostics. Its counts are **mixed allocations**:

| Case | Status off/on | Ordinary same/different/no-hash | Eval-always same/different/no-hash |
| --- | --- | --- | --- |
| Initial | 0 / 0 | absent / absent / absent | absent / absent / absent |
| Unchanged | 0 / 0 | 20 / absent / 1 | 113 / absent / 2 |
| Body edit | 0 / 0 | 55 / 3 / 4 | 111 / 2 / 2 |
| Type error | 1 / 1 | 48 / 7 / 4 | 62 / 2 / 2 |
| Recovery | 0 / 0 | 55 / 3 / 4 | 111 / 2 / 2 |

Totals `178 / 13 / 13` and `397 / 6 / 8` must not be described as provider
execution counts. Matching the old standalone inventory to the old analyzer
proved decoder consistency, not correctness of the original semantic boundary.

The older frozen driver
`77c3da92b7795833b871b5c7e965700ee707949dceac910d75cbdfb10db17c1d`
was an unsupported-marker negative control. The generic-only five-case run
passed native status/diagnostic/runtime comparisons but failed its marker gate.
Its failed report remains at
`/home/jeremy/.jcode/scratch/reexecuted-old-negative-generic-20261030/`.
An earlier default-filter attempt remains separately at
`/home/jeremy/.jcode/scratch/reexecuted-old-negative-20261030/`; it stopped on the
strict decoder's `invalid string ID` rejection of a Query interval retaining
`EventId::INVALID` during the type-error compilation. This limitation was not
silenced, filtered after capture, or treated as a passing correctness gate.
