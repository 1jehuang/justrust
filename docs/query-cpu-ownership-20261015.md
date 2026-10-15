# Named query-execution CPU ownership, 2026-10-15

## Question and status

The previous cold `itoa` capture measured 198.098740 ms of exclusive thread CPU,
of which 131.955880 ms was opaque and 66.142860 ms had pool-task ownership.
Those are retained diagnostic measurements, not all frontend CPU or a benchmark
baseline. Result-availability provenance did not fix this ownership gap.

This follow-up adds an explicit native query-execution owner at the actual
`start_query` boundary. The implementation is shipped and real-source acceptance
passed. It is not an accepted performance improvement or a completed execution
DAG.

## Ownership protocol

- The new explicit `QueryAdmissionObserver::new_with_event_capacity_and_query_cpu`
  constructor enables timing and independent query ownership. Existing constructors
  retain their old output and do not opt into this mode.
- The actual receiving `TyCtxt`, admitted context/job identity, and compiler-static
  `QueryVTable.name` qualify the owner. Inherited query TLS on a helping worker is
  not an ownership authority.
- Pool schema 4 and timing schema 2 add mandatory nullable `query_context`,
  `query_job`, and `query_name` to every exclusive CPU segment. The three values
  are present together only for `owner_kind: query_execution`. Names are bounded
  ASCII compiler identifiers, never source keys, paths, or formatted user types.
- Nullable `task` remains genuine, independent pool ownership. It is not a
  fabricated task root or a query-to-task dependency edge. Query-execution,
  non-query task, and opaque CPU categories form a disjoint retained sum.
- Query entry/exit saves and restores the worker-local owner. Dispatch, foreign
  task entry, forks, and continuations explicitly suspend/restore ownership.
  Stolen work does not inherit its helper's query identity. Real nested query
  execution can establish its own owner even within otherwise opaque dispatch.
- Query root schemas 5/6 and the reciprocal clock descriptor are unchanged.
  A pool receipt alone cannot certify query admissions. The consumer requires
  original same-capture admission lifetimes, validated reciprocal clocks,
  per-query/per-thread nonoverlap, and consistent names before exposing costs.

The owner means **query execution**, including incremental green loading or
recomputation. It does not mean a fresh provider invocation. Partial event
inventories, clock failures, loss, unknown endpoints, and unavailable reciprocal
authority remain explicit. Full CPU work, execution span, semantic/control graph
coverage, and theoretical parallelism stay unknown.

## Public interface

```sh
justrust graph /private/capture --large-capture --query-cpu-ownership --json
python3 bench/frontend_mapping.py \
  --capture /private/capture --large-capture --query-cpu-ownership \
  --graph-output /private/new-output/full-graph.json \
  --output /private/new-output/summary.json
```

The native mode is opt-in and the public flag does not itself enable compiler
recording. Pool4 requires the explicit public flag. Cache-call and dispatch-result
availability analyses retain their own independent opt-ins. Root6 can be decoded
for query CPU analysis without automatically exposing dispatch-result relations.

The analyzer retains exact full public JSON in an exclusive-created private
artifact outside the source capture, with its size and SHA-256 in the compact
report. It copies only validated costs, groups them by compiler query name,
and retains the top 20 queries and query kinds with explicit omission counts.
Rankings are observed cost, not critical-path membership, serial CPU, or proof
that any query can safely be parallelized.

## Acceptance requirements

1. Native fake-clock and real-pool tests: nested queries, unwind, actual helper
   dispatch, stolen work, foreign observers/registries, forks and continuations.
2. Default serialization/no-clock compatibility and unchanged query roots.
3. Strict consumer mutation tests: mandatory nullable fields, query-name grammar
   and consistency, actual admission endpoints/lifetimes, duplicate captures,
   loss and reciprocal clock failure, and legacy output parity.
4. Coherent compiler/library build, immutable runtime plus matching private
   metadata, then untouched locked `itoa` cold/warm compilation and executed
   linked consumers. OFF/ON/OFF compiler argv/environment/diagnostic and artifact
   controls are required. Warm archive link-metadata instability remains excluded
   only when independently reproduced under OFF/OFF controls.
5. Every real retained query-owned CPU segment independently checked against the
   original admissions and per-query nonoverlap, every public ranked cost checked
   against original native segment arithmetic, and actual named hotspot reports.
6. Hosted check/full tests/clippy, format/diff checks, installed public CLI replay,
   exact run exports, and committed retained evidence summaries.

Before/after compile-time or overhead claims additionally require the comprehensive
matched synthetic gate. The earlier A2 series remains ineligible: 39 noisy points
and seven unexpected failing-test points. The locking/recovery fixes in
`bench/synth/ISOLATION-A2.md` do not establish their root causes or retroactively
repair that evidence. No speedup/default promotion is justified by this diagnostic
feature alone.

## Accepted evidence

The immutable stage1 compiler and matching private metadata passed all eight
native harness tests in 216.390 s, including untouched locked `itoa` cold/warm
compilation, linking, executed consumers, OFF/ON/OFF controls, original-byte
ownership reconciliation, and actual analyzer reports for root5 and root6.
The native ownership suite passed 235 tests with 28 ignored. Hosted client checks,
655 tests (3 ignored), and strict all-target clippy passed. Exported runs are in
`data/runs/query-cpu-20261015/2026-10.jsonl`, including
`20261015-052355236-808478`, `20261015-052358578-808947`, and
`20261015-052414363-808296`.

Retained compact reports are `bench/results/query-cpu-20261015/{cold,warm,with-dispatch}.json`.
Every reported cost was reconciled with original native segments and admitted
query lifetimes, with complete retained timing inventories and aligned clocks.

| Actual capture | Named query CPU | Non-query task CPU | Opaque CPU | Query owners | Query kinds |
|---|---:|---:|---:|---:|---:|
| Cold | 532.930074 ms | 18.681657 ms | 4.015578 ms | 18,199 | 244 |
| Warm | 19.811304 ms | 15.138516 ms | 1.047598 ms | 1,833 | 94 |
| Cold plus dispatch | 471.826907 ms | 18.460919 ms | 3.942291 ms | 18,199 | see retained report |

The cold capture's largest named query category was `typeck_root` at
206.496525 ms, followed by `mir_borrowck` at 36.384997 ms and `mir_built` at
28.280237 ms. Warm was led by `resolver_for_lowering_raw` at 6.106205 ms and
`lower_to_hir` at 3.058462 ms. These are observed exclusive execution costs,
not serial bottleneck proofs or permission to parallelize those queries.
This opt-in mode samples many more boundaries. These diagnostic captures are
not matched performance measurements and must not be compared with the earlier
198 ms capture to claim a speedup or regression.

Accepted compiler driver SHA-256:
`b1435d02eabb44c87652b6483c66a27cd00567db64318cb6fb94556144ec1727`.
Matching private metadata SHA-256:
`fc83d8e88f7e283e338aec5645f677be41dec7f129abf1df764ab6fbd43ba474`.
Installed public client SHA-256:
`fa071f8e500d8c19a7536cac73a37e8f7680c01bb4ac517e4223aa4835ebcbb0`.
Full public graph artifacts remain private and are identified by exact byte
size and SHA-256 in each committed compact report. Native runtime, source,
fixture, controls, and analyzer hashes remain in the retained acceptance cohort.

## Broader work still open

Named query CPU ownership is a prerequisite for useful bottleneck localization,
not the endpoint of the dependency mapping project. Event/continuation-level
execution dependencies, route-specific waits/races/previous-session provenance,
non-query/resource/backend/link/test work, larger-crate capture capacity and
work conservation still need explicit native authority before computing full
work/span or a useful-thread ceiling. Availability relations must never be
combined with whole-job nesting into an invented critical-path graph.
