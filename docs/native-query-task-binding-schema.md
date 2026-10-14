# Explicit native query-to-task point binding

Status: implemented and accepted on the rebuilt native compiler. See
`native-query-task-binding-results-20261014.md` for requirement-to-check evidence.
This does not claim a
full compiler graph or a parallelization ceiling.

## Versioned authority

Query receipt schema 4 retains typed admission, wait and read records and adds
`task_bindings` schema 1. Pool receipt schema 2 names its reciprocal
`query_capture_id`. Query and pool capture IDs remain distinct. The query binding
header names the exact expected pool capture ID and observer ID. A filename,
shared numerical ID or currently installed thread context is not authority.

Native Cargo negotiates the exact `reserved-fd-v2,query-schema=4` capability.
The existing descriptor-only transport remains invisible to argv and environment.
Its decoder recognizes historical schemas 3 and 4, but this compiler admits only
schema 4 tickets. Entry publication declares the ticket's exact expected schema.
The client accepts supported entry schemas and requires the final query schema
to equal the independent entry declaration. Historical query schemas 1 through 3
and pool schema 1 remain accepted, with task lineage unknown rather than zero.

Standalone query capture identity is minted before pool attachment. Transport
capture identity is preserved. Pool attachment retains authority only after
successful attachment to the actual compiler registry.

## Point contract

The mandatory `task_bindings` object contains:

- `schema: 1`
- `records_complete`, `all_targets_bound`
- mandatory nullable `expected_points`
- `lost_events`, `loss_overflowed`
- mandatory nullable `pool: {capture_id, observer}`
- mandatory nullable `unavailable_reason`, either null or `pool_unavailable`
- `events`, a bounded array of point records

Each record has a strictly typed `source`, mandatory nullable
`target: {task, at_pool_ns}`, and mandatory nullable `unbound_reason`.
Source kinds are `admitted {context_id, job_id}`, `request_started {request_id}`,
`dep_read {read_id}`, `result_produced {result_id}`,
`wait_registered {wait_id}`, and `wait_resumed {wait_id}`. All IDs except
admitted jobs are root-global independent allocator identities. Admitted jobs
are qualified by their query context. The enclosing query capture qualifies all
source identities.

A bound target has a positive task identity and time from the pool's own epoch.
Unbound reasons are `pool_unavailable`, `no_active`, `suspended`,
`foreign_observer`, or `clock_overflow`. An early unavailable point stays
unavailable even if the pool is attached later. No later request or admitted
parent is used to infer a point's target. Points do not assign a task owner to an
entire query job or request lifetime.

Expected point count comes from checked independent allocator arithmetic:

```
sum(context job high water - 1)
+ request high water - 1
+ read high water - 1
+ result high water - 1
+ 2 * (wait high water - 1)
```

Missing allocator snapshots, missing contexts or arithmetic exhaustion keep the
total unknown. Retained row budgets do not cap independent counts. Clients
validate large partial inventories using ranges rather than constructing a set
for millions of dropped points. Duplicate, unknown and malformed typed fields
remain rejected. The theoretical allocator-wrap path can retain invalid zero or
historical IDs and therefore fail strict decoding. Unknown totals prevent false
completeness, but do not make those malformed histories supported records.

## Capture and synchronization

Point sampling borrows the active logical-task slot, verifies the exact observer,
and copies task/time without cloning contexts or taking a receipt lock. Opaque
helper dispatch has its installed task masked by the accepted dispatch isolation
fix. Suspended fork state remains explicitly suspended until its continuation.

Wait registration samples immutable local facts under the existing latch lock.
It does not acquire the binding buffer mutex there. Registration and resume rows
are published after allowance reacquisition, outside native locks. Dropping an
abandoned token publishes nothing, and independent inventory exposes the loss.
The permanent cap skips further sampler/TLS/clock/mutex work while independent
source allocators and loss accounting continue. Native compiler dynamic threading
uses `IntoDynSyncSend` for the std `Send + Sync` sampler trait object.

Poisoned pool publication buffers permanently report dropped observations and
cannot authorize query bindings. Query, read, wait and binding loss remain
separate. Read-record inventory is distinct from admitted endpoint availability.
A complete local read or point record inventory does not manufacture an omitted
admission endpoint or resolved semantic edge.

## Consumer outputs

Public graph commands collect typed receipts and reconcile authorities before
flattening summaries. Duplicate query or pool capture authorities are excluded
from eligible reconciliation. Selected compiler-manifest commands only use the
pool explicitly named by their selected query. Missing, stale, foreign or
nonreciprocal pool authority is a mapping failure, independently of valid compiler
launch accounting.

Binding assessment distinguishes:

- `records_complete`: complete point row inventory with no binding-local loss
- `source_endpoints_complete`: underlying recorded source endpoints are present
- `all_targets_bound`: every point has an explicit target, with no unbound rows
- `graph_binding_valid`: reciprocal pool authority and bound target intervals
  validate, with underlying endpoint coverage
- `graph_binding_complete`: complete point inventory, all targets bound and
  valid graph binding

Task interval validation uses the pool's actual definition/start/finish records
and inclusive `start <= at_pool_ns <= finish`. It never compares query and pool
clock epochs. Valid bound subsets can coexist with explicit unbound points, so
validity does not imply complete target mapping.

All receipts continue to report full-pipeline coverage false and CPU work/span
null. Scope scheduling, resource/token causality, previous-session/feed/direct
provenance and nonquery/codegen/link/subprocess coverage remain incomplete.
Actual rebuilt private-compiler fixtures and source-matched launch, artifact,
diagnostic and proc-macro neutrality gates passed for this slice, as recorded in
the results matrix. Focused capture-cost controls are not a comprehensive
session-weighted speed gate or default promotion evidence.
