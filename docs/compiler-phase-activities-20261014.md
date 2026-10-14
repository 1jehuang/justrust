# Existing compiler phase activity coverage, 2026-10-14

## Scope

The same optimized BUILD capture reported in
[the dependency mapping](compiler-query-build-graph-20261014.md) contains more
than query providers. Existing GenericActivity records also describe codegen,
LLVM optimization, LTO, metadata, persistence and linker elapsed intervals.
Analyzing these records fills a coverage gap, but does not create missing causal
edges or CPU counters. No rustc changes, new instrumentation, hosted builds,
installs, default changes or hardware decisions are part of this follow-through.

The existing run is `20261013-232242825-2161355`, pinned nightly-2026-10-08,
client source `059674bf76fa218ba7e475279a86b4b6caf32aac`. Its real optimized
library BUILD took 77.277 seconds hosted / 89.474 seconds client time. It had
one foreign rustc, a fresh target and warm shared dependency cache with 107
misses. Neither this profile nor a comparison with CHECK is a causal performance
baseline. Raw profile SHA256 is
`a54af304b4ea2b0e075d0826936b89a698e51edcfba5024643e8f552ba788b6b`.

## Concrete existing-event coverage

The raw profile has 6,851,280 interval records, including 4,156,032
GenericActivity records with 266 distinct activity labels across 517 Rust thread
IDs. The number of thread IDs is not a simultaneous runnable-thread count or a
number of physical cores. The host has 24 physical cores.

The first inventory found these inclusive elapsed aggregates:

| Existing GenericActivity label | Records | Inclusive thread-elapsed seconds |
|---|---:|---:|
| `codegen_module_perform_lto` | 256 | 70.423775894 |
| `codegen_module_optimize` | 256 | 36.675429894 |
| `LLVM_module_optimize` | 257 | 34.788036807 |
| `LLVM_lto_optimize` | 256 | 33.542760579 |
| `LLVM_module_codegen` | 257 | 26.621427764 |
| `LLVM_module_codegen_emit_obj` | 257 | 26.024041768 |
| `generate_crate_metadata` | 1 | 4.483314210 |
| `finish_ongoing_codegen` | 1 | 3.673879102 |
| `monomorphization_collector` | 1 | 3.582807787 |
| `codegen_crate` | 1 | 3.359846080 |
| `LLVM_passes` | 1 | 3.193952150 |

**Do not sum this column as CPU work or sequential elapsed time.** These are
nested and overlapping intervals, often on different threads. For example,
`codegen_module_optimize` contains `LLVM_module_optimize`, and object emission
is nested under code generation. Query events also include backend queries, so
all Query records cannot be classified as frontend work.

## Semantics checked against the exact pinned compiler source

These are existing-source observations, not proposed implementation changes:

- `rustc_interface/src/passes.rs` emits separate parsing, expansion, validation,
  borrow-checking, lint and final-MIR activities. `codegen_crate` initiates an
  ongoing backend, so its elapsed duration is not the entire backend duration.
- `rustc_interface/src/queries.rs::finish_ongoing_codegen` wraps joining and
  finalizing ongoing codegen. It includes waiting, not just running code.
  `link_crate` wraps the subsequent backend link operation.
- `rustc_codegen_ssa/src/back/write.rs` emits module optimize/LTO and reused
  incremental-artifact copy activities. A copy activity is not fresh codegen.
  Its `LLVM_passes` guard spans coordinator activity and waits, not active LLVM
  CPU exclusively. It also owns the `LLVM_thinlto` / `LLVM_fatlto` wrappers.
- `rustc_codegen_llvm/src/back/write.rs` emits worker optimization/codegen and
  nested object/IR/bitcode/assembly emission activities. Its sibling `lto.rs`
  emits LTO optimization/import activities.
- `rustc_codegen_ssa/src/back/link.rs::run_linker` measures an elapsed subprocess
  operation, not separately attributed linker CPU.

Exact source pin: `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`. No existing
label supplies a complete cross-thread fork/join graph, ready-queue history,
jobserver token identity or per-query CPU accounting. Policies against
LLM-originated soundness-sensitive compiler implementation still apply. This
work only reads existing source and analyzes already captured events.

## Measurement cost remains visible

The capture contains 2,809,610 `incr_comp_encode_dep_graph` activities, totaling
3.400720840 aggregate inclusive thread-elapsed seconds. The same capture also
contains dependency-graph serialization and persistence activities. These
observations belong to the diagnostic capture, not an uninstrumented baseline.
It would be incorrect to call their elapsed sum a measured profiling slowdown
without matched controlled runs, or to subtract it from the BUILD wall time.

## All-kind exclusive elapsed accounting

`bench/query_activities.py` consumes every decoded record and groups by exact
`(kind, label)`, without treating Query as frontend or classifying unknown
labels away. Per Rust thread, it orders intervals by start, descending end and
original record order. It rejects crossing intervals, subtracts only direct
nested children of **all recorded kinds**, and retains a union of covered root
intervals. Equal endpoints use arbitrary but stable raw-order attribution and
are counted explicitly. Instant and integer records retain counts, never become
durations. Memory is bounded but not constant: interval tuples are retained.

The real public CLI processed **6,851,280 intervals**, **584 kind/label groups**
and **517 Rust thread IDs**. There were no crossing intervals and no equal
endpoint duplicates. Sum of group-exclusive elapsed equals sum of per-thread
covered unions: **148.396651246 thread-elapsed seconds**. The observed timestamp
window was **15.826062441 seconds**, within the separately recorded UI compiler
unit wall time of **15.908533573 seconds**. Neither window is the entire
77.277-second hosted BUILD.

Selected rows now partition nesting instead of double counting it:

| GenericActivity label | Inclusive seconds | All-kind exclusive seconds |
|---|---:|---:|
| `codegen_module_perform_lto` | 70.423775894 | 1.441279971 |
| `codegen_module_optimize` | 36.675429894 | 1.887412597 |
| `LLVM_module_optimize` | 34.788036807 | 34.788036807 |
| `LLVM_lto_optimize` | 33.542760579 | 33.542760579 |
| `LLVM_module_codegen` | 26.621427764 | 0.597385996 |
| `LLVM_module_codegen_emit_obj` | 26.024041768 | 26.024041768 |
| `generate_crate_metadata` | 4.483314210 | 4.326772275 |
| `finish_ongoing_codegen` | 3.673879102 | 3.658896283 |
| `monomorphization_collector` | 3.582807787 | 0.023190353 |
| `LLVM_passes` | 3.193952150 | 3.193952150 |
| `run_linker` | 0.094113251 | 0.094113251 |

**Exclusive still does not mean active CPU.** For example, ongoing-codegen join
and the `LLVM_passes` coordinator guard can include waiting without nested
records on that thread. LLVM leaf intervals may still contain unreported
waiting and descheduling. The 148.397 aggregate thread-elapsed seconds exceed
the UI process's separately recorded 129.698 CPU seconds. This is not a CPU
accounting error or proof of full utilization: the quantities are different.
Do not divide these elapsed aggregates by the window to infer CPU capacity.

The previous dependency mapping's **21.568632374 query-only exclusive elapsed
seconds** subtracts nested queries and known blocked intervals, not every
GenericActivity child. The new all-kind partition has a different definition.
Both outputs are retained without silently replacing one metric with the other.
No per-label start/end windows are emitted. Per-thread gaps are only uncovered
elapsed timestamps, not proven idle time.

## Public interface and actual acceptance

Implementation commit: `2be8d4c0a6f0cf4865eb76f9da5392ba7b1b4423`.

```sh
python3 bench/query_activities.py \
  --profile desktop-release-t4.mm_profdata \
  --output fresh-activities.json --large-capture
python3 -m unittest discover -s bench -p 'test_*.py'
python3 bench/results/compiler-phase-activities-20261014/reference-check.py
```

The large capture remains private and is not embedded in this repository. Its
compact output is published at
`bench/results/compiler-phase-activities-20261014/desktop-release-activities.json`.
Output SHA256 is
`23d71f4e533a0c733522e7310a11c24aad6370f133cf366ef8cea2511a99689b`.
The analyzer stores only the command hash, not the captured command or paths.
The profile hash, PID, start time and command hash match the already committed
BUILD graph mapping. This consistency check is not a new attestation or a
complete graph receipt from the profile-only tool.

`--large-capture` explicitly enables existing decoder limits for this capture.
Default limits still reject it. `--allow-unmapped-event-labels` is separate and
explicit: strict mode rejects missing labels, opt-in retains them as null.
Activity limits are 8 million records, 16,384 Rust thread IDs, 2,048 groups,
4,096 UTF-8 bytes per label and 1 MiB compact output. Existing decoder byte,
string and format limits still apply. Output is created exclusively with mode
0400. Existing files and input/output symlinks are refused.

The final accepted implementation was exercised through the actual CLI on the
real large profile and small synthetic boundary inputs. Full raw stdout/stderr,
exit codes and hashes are retained beside `acceptance.json`. Analysis runtime is
not compiler runtime and is not a performance comparison.

| Requirement | Observed check and result |
|---|---|
| Account for real optimized BUILD, not only CHECK | Existing run `20261013-232242825-2161355`, exact profile hash and original graph metadata match. Real large CLI reproduces the compact output byte for byte. |
| Avoid nested double counting across event kinds | Group-exclusive sum equals per-thread covered union, 148,396,651,246 ns. Each row is bounded by its inclusive duration and each thread's covered plus gap equals its endpoint window. |
| Validate mathematical partition independently | Portable fixed-seed reference enumerates every unit-time cell in 250 shuffled nested forests across three threads. It agrees with every group's exclusive allocation and total coverage. |
| Exercise whole implementation and integration boundaries | 177 offline tests pass, including eight activity methods covering generators, all event roles, unknown labels/kinds, timestamps, equal/zero/crossing intervals, caps, privacy and safe output. |
| Preserve large-input default guard and small-input compatibility | Real oversized default invocation exits 2 without output. Real explicit large invocation exits 0. Small default invocation exits 0. |
| Preserve failures and controlled opt-ins | Actual CLI rejects malformed input and strict unmapped labels. Explicit unmapped mode succeeds and keeps null labels. Existing output stays byte-identical. Input/output symlink rejection creates no target. |
| Verify source semantics, not name-based CPU assumptions | Six exact pinned Git-object hashes and 17 literal label locations rechecked. Independent read-only review confirms scoped semantics and inclusive inventory parity. |
| Keep privacy and provenance | Captured command is never exported. Source, output and CLI log hashes are recorded. The original BUILD's two cited run records were already exported and committed, no new build was run here. |
| Leave unsupported conclusions unknown | `cpu_work_ns` and `execution_span_ns` stay null in real and synthetic results. No speedup, hardware ceiling, runnable thread count, pure-idle duration or default promotion is reported. |

Independent review by ant passed the eight activity tests, compact real-output
hash/identity, inclusive-inventory parity and all per-row/per-thread invariants.
That review did not perform a second giant parse or a compiler run. The owner
also reran the actual full CLI workflow against the whole final implementation.

## Remaining limits

Existing activities can describe phase elapsed coverage and nesting, not CPU
work W or execution span S. Rust thread IDs are not proven OS TIDs. Coordinator
and worker scopes overlap; waits and OS descheduling can remain inside any
elapsed interval. Untimed query nodes and missing read edges from the dependency
mapping remain unresolved. No new inference about optimal 16/24/96-core sizing,
maximum parallelism, speedup or compiler default is justified.
