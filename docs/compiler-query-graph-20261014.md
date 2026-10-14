# Compiler invocation graph and measured query mapping, 2026-10-14

## Result and scope

We now decode an actual same-invocation rustc dependency graph and join numeric
query-provider and query-blocked event IDs to its nodes. This is a substantial
improvement over occupancy: individual dependencies, observed providers, waited
producers, and missing coverage are inspectable. It is **not** a complete
execution DAG, a measured CPU-work/span model, or a demonstrated speedup.

The ordinary hosted service, compiler selection, billing, tenant isolation,
permissions and cache defaults were not changed for this capture. Graph/profile
flags and incremental directories were isolated. The account host was explicitly
released at 06:00 UTC. Last capture build ended 05:46:52 UTC, and the last
read-only artifact fetch ended 05:48:16 UTC.

## Exact provenance

- Compiler: `nightly-2026-10-08`, rustc `1.101.0-nightly (1d81eb4ad 2026-10-07)`.
- Compiler source: `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`.
- Compiler binary SHA256: `66877d44efc77e5d5478c756abf48574c5ce4d41a1102518b20e2ff3f3a0ba56`.
- Capture client source: `059674bf76fa218ba7e475279a86b4b6caf32aac`.
- Immutable capture client SHA256: `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`.
- Host: west `m8azn.6xlarge`, 24 physical cores, unchanged preserved volume.
- Desktop snapshots: `jcode-desktop@7d38fb7`, `jcode@abff94eeb`, including the
  existing pinned-suite API compatibility fixups. No workload source was edited.

The trusted owner-generated private capture receipts check the two input hashes,
compiler pin, numeric PID and profile filename, final incremental-directory
argument, graph containment, crate name, and recorded compiler-unit start
interval. These are consistency checks, **not cryptographic attestation of an
untrusted supplied receipt**. A full command/path record stays private. Published
results retain its SHA256 rather than exposing private paths or command arguments.

## Actual observations

| Capture | Hosted run ID | Graph nodes | Edges | Observed providers | Untimed nodes | Blocked intervals |
|---|---|---:|---:|---:|---:|---:|
| Tiny, one frontend thread | `20261013-223840760-1863795` | 3,379 | 8,215 | 3,336 | 43 | 0 |
| Tiny, four frontend threads | `20261013-223842465-1863794` | 3,379 | 8,215 | 3,336 | 43 | 23 |
| Desktop UI, four frontend threads, light events | `20261013-224631463-1915834` | 1,052,237 | 6,496,600 | 973,105 | 79,132 | 151 |

All 973,105 Desktop provider records join saved node IDs. None require an invented
node or label. The profile contains 2,531,221 records: 1,557,965 GenericActivity,
973,105 Query, and 151 QueryBlocked intervals. All event labels resolve.

The tiny profiles retain all 41,482 / 41,509 raw records, respectively. Each has
21,427 records referencing 1,379 virtual IDs without string mappings. Default
strict decoding rejects these captures. Explicit
`--allow-unmapped-event-labels` preserves their numeric IDs and reports a null
label, not an inferred name. Their 3,336 provider IDs still join the saved graph.

Desktop has 340 sparse-index holes. The tiny four-thread graph has 366 holes.
Physical serialization order is not node identity. The longest Desktop
read-dependency chain contains 129 nodes. Its largest structural frontier has
362,173 nodes. Neither figure describes a runnable-thread count or time span.

### Times, with the command classification preserved

The light Desktop capture is a **hosted `check`**, not a build or test:

- Recorded whole command: 20.0 seconds, client wall time 20.6 seconds.
- UI rustc unit: 3.607524395 seconds.
- Reported UI CPU time: 10.048405 aggregate CPU seconds, about 2.79 CPU-seconds
  per elapsed second. This is observed process utilization, not maximum
  parallelizable work or per-query CPU attribution.
- Target directory was cold, shared dependency artifacts were warm: 887 units,
  zero Fresh, 833 dependency-cache hits, zero misses; 13 local-cache hits.
- Another crate, `jcode_base`, took 7.73 seconds. The UI unit alone is not the
  entire workspace critical path.

An earlier all-event capture, `20261013-224426718-1907396`, recorded a 44.7-second
check and an 18.465456-second UI unit. Its profile is 1,120,194,875 bytes, beyond
our 256 MiB decoder limit, and was not downloaded or decoded. It also had 143
dependency-cache misses, versus zero in the light capture. **Do not divide these
runs and call the result an optimization or isolated profiling overhead.**

The tiny compiler units were approximately 0.02239 / 0.02428 seconds. This single
fixture pair is not a scaling benchmark or evidence for hardware selection.

### Concrete query and wait targets

The following are individual provider records, named using the exact pinned
337-kind declaration, not the current fork's shifted query enum:

| Query | Exclusive elapsed ms | Inclusive elapsed ms |
|---|---:|---:|
| `resolver_for_lowering_raw` | 396.254078 | 409.818985 |
| `analysis` | 150.433751 | 2,809.986191 |
| `typeck_root`, first instance | 85.173765 | 95.227561 |
| `typeck_root`, second instance | 74.500743 | 83.421705 |
| `check_type_wf` | 49.762591 | 573.072065 |
| `typeck_root`, third instance | 45.534261 | 100.206413 |
| `mir_borrowck`, first instance | 40.081917 | 40.866778 |

The mapped exclusive provider elapsed total is 10.759626971 seconds, summed
across threads after subtracting immediate nested query/wait intervals. It still
contains unreported locks and OS descheduling, so it is **not CPU work W**.
The observed blocked total is 129.890026 ms across 151 records; the largest is
32.758394 ms. These are aggregate thread waits, not all compiler waiting or a
serial fraction. In 55 blocked records, the observed caller-to-producer read edge
is absent from the saved graph. This is direct evidence that the saved read graph
alone does not capture all observed synchronization.

Next targets are the resolver/expansion stage and large type-checking or
borrow-checking providers, plus producer/waiter scheduling around the actual
blocked pairs. `resolver_for_lowering_raw` in pinned `rustc_interface/src/passes.rs`
constructs a resolver and calls `configure_and_expand`; pinned
`rustc_resolve/src/lib.rs::resolve_crate` has separate import, visibility, macro,
late-resolution, and checking activities. These are investigation targets, not
claims that a provider is safe to parallelize or that its entire elapsed time is
removable.

## Why we cannot calculate the hardware ceiling yet

For a **complete** execution DAG with CPU-work weights W and critical-path work S,
the usual lower bound is `T(P) >= max(W/P, S)`, and `W/S` describes average
available parallelism under that model. We do not yet possess those inputs.

The saved incremental graph omits `eval_always` and ignored read dependencies.
Anonymous nodes can alias other identities. Green/promoted nodes can have no
current provider timing. A parent provider has ordered pre-child/post-child
continuations, not one task that can simply run alongside its dependency.
Unreported locks, jobserver tokens, scheduler handoffs, subprocesses, and OS
waiting are missing. QueryBlocked's numeric ID is the **producer**, not its caller.

Both `execution_span_ns` and `cpu_work_ns` are consequently null in every
published mapping. Unknown timings stay unknown. There is no 16/24/96-core
recommendation, serial fraction, 10x claim, or default promotion derived from this
capture.

The next measurement milestone is a continuation-level execution trace with
explicit task scheduling and synchronization edges, per-thread CPU accounting,
and separately attributed subprocesses, followed by cold/build/test/edit-loop
coverage. Compiler-semantic changes require the repository's review gate. This
session made no rustc query-semantic, cache, or codegen changes.

## Public interfaces and format pins

```bash
python3 bench/query_graph.py CAPTURE-dep-graph.bin
python3 bench/query_profile.py --input CAPTURE.mm_profdata --output NEW-summary.json --summary-only
python3 bench/query_mapping.py --graph CAPTURE-dep-graph.bin --profile CAPTURE.mm_profdata --receipt PRIVATE-receipt.json --output NEW-mapping.json
python3 -m unittest discover -s bench -p 'test_*.py'
```

For explicitly retained unmapped virtual event labels, add
`--allow-unmapped-event-labels`. Existing outputs and ancestor/leaf symlinks are
rejected. Receipts are bounded to 64 KiB, profiles/graphs to 256 MiB, profiles to
4 million events, and compact mapping output to 1 MiB. Full graph JSON is an
optional structural export, not the compact report. The full profile API can
materialize millions of Python records; the summary API streams validated
records instead.

`query_graph.py` implements this exact rustc RSIC version, sparse saved indices,
38-byte node headers, variable-width edges, kind counts, and the three-u64 footer
before `rust-end-file`. The kind count is exactly 337 in the pinned compiler.
The current fork adds a query and must not supply this capture's labels.
`query_profile.py` implements measureme 12.0.3 MMPD format 9: interleaved pages,
64-bit string IDs, 16-byte index entries, 9-byte references and 32-byte records.
The older decodeme 10.1.3 32-bit layout is not used.

## Requirement-to-observation map

| Requirement / changed output | Concrete check and observed result |
|---|---|
| Actual invocation IDs, not kind DOT or serialization ordinal | Tiny one/four-thread and Desktop public graph CLI passes; sparse/out-of-order fixtures and independent shuffled-real-record review preserve exact IDs/edges. |
| Exact source/compiler format | Pinned core serialization source compared byte-identical; 337-kind table derived from pinned source; unsupported version, footer corruption, LEB overflow and malformed edges rejected. |
| Same-session graph/profile join | Three public mapping CLIs pass all receipt checks; wrong real receipt independently rejected exit 2 without output; hash/PID/path/crate/time mismatch cases reject. |
| No nested time double-count | Per-thread laminar interval fixtures, nested waits, cross-thread producer/caller, equal labels and repeated identities pass. Crossing intervals reject. Stable equal-boundary ordering is arbitrary attribution, not recovered causality. |
| Missing coverage retained | 79,132 Desktop nodes remain untimed; 55 wait read edges absent; tiny missing virtual labels strict-fail and explicit-opt-in counts reproduce exactly. |
| CPU/span claims constrained | All real outputs retain null CPU-work/span; counters never become elapsed durations; graph frontiers never become runnable parallelism. |
| Public failure semantics / packaging | Final whole offline package has 164 passing tests: graph 14, profile 27, mapping 15 plus prior package tests. Eight final actual public CLI checks pass, including deliberate strict-label and tampered-pair exit-2 checks. |
| Filesystem/resource safety | Output overwrite refused without changing prior bytes; read-only output mode observed; ancestor/leaf symlinks rejected; receipt cap enforced before large read; malformed/oversize fixtures reject. |
| Actual real workload, not only synthetic fixtures | Final Desktop pair rerun reproduces the original output exactly; independent reviewer reproduces 10 key metrics from the actual pair. |
| Improvements are useful, not claimed performance gains | Numeric dependencies and producer waits now join actual timings, whereas previous occupancy could not. Complete execution DAG and maximum-parallelism objective remain explicitly unfinished. |

Raw captures and private command receipts are not published. Compact mapped
results, exact source-kind provenance, actual public-interface output hashes and
slim cited run records accompany this document. Harness mistakes are retained:
`justrust rustc` is unsupported; the first debug `build --remote` attempt routed
locally by the documented debug-build rule and failed the remote-only paths
(`20261013-223745790-1859236`). The successful captures use the supported billed
hosted `check --remote` route. These mistakes are not reported as hosted compiler
failures or default-routing defects.
