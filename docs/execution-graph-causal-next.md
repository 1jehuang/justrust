# Next native causal edges

Code-inspected on 2026-10-14 against `96adb917c`. This is an implementation
plan, not implemented coverage or rebuilt-compiler acceptance. Guppy owns
acceptance. `docs/execution-graph-query-admission.md` remains the coverage contract.
All compiler paths below are relative to `toolchain/rustc/compiler/`.

## Baseline and edge vocabulary

`rustc_query_impl/src/execution.rs::try_execute_query` allocates through
`next_job_id`, inserts `ActiveKeyStatus::Started`, then calls
`QueryAdmissionContext::admit` after dropping the state lock.
`ActiveJobGuard::complete` publishes the cache value;
`ActiveJobGuard::drop_and_maybe_poison` removes or poisons active state and calls
`signal_complete`. Observer job completion/drop records completed/poisoned
terminals in `rustc_session/src/query_admission.rs`. Allocator snapshots and
root interface-return sealing inventory admissions, not reads or scheduler work.

Keep three explicit edge classes:

- **Semantic:** consumer depends on a particular query result or validated
  incremental dependency. Orient prerequisite/result -> consumer operation.
- **Control causal:** an observed completion, notification, join or resource
  grant enables a particular continuation. Not a Rust value dependency.
- **Descriptive:** admission nesting, inherited context, queue membership,
  worker placement and chronological adjacency. Never promote these alone to
  semantic prerequisites or critical-path edges.

## Ordered implementation slices

### 1. Query waits and resumes

Hooks: `rustc_query_impl/src/execution.rs::{try_execute_query,wait_for_query,
latch_wait_on,signal_complete,ActiveJobGuard::drop_and_maybe_poison}` and
`rustc_query_impl/src/job.rs::{find_and_process_cycle,break_query_cycle}`.

Capture waitee `job.id` in the occupied `Started` branch before releasing the
state shard lock, and pass its context-qualified identity into the wait path.
Allocate a wait ID when `latch_wait_on` registers `QueryWaiter` under the latch
lock. Carry it through normal notifications and cycle-removal notifications.
Record wait registration, notification reason (completed, poisoned, cycle),
condvar return and post-token-acquisition continuation as distinct events.
The already-complete latch branch is a no-block outcome, not a fabricated wait.

Emit completion/poison/cycle notification -> matching resume as control edges.
Record successful result consumption separately as a semantic edge, only after
`wait_for_query` obtains the cache result. A cycle recovery is not a successful
waitee result. Notification does not mean the worker has resumed: the pool
reacquires a jobserver allowance before returning. Preserve serial cycle handling
in `find_and_handle_cycle` without inventing parallel wait events.

### 2. Cache read dependencies, including paths without admissions

Hooks: `rustc_middle/src/query/calls.rs::{try_get_cached,query_get_at,
query_ensure_ok,query_ensure_result,query_feed}`;
`rustc_query_impl/src/execution.rs::{try_execute_query,wait_for_query,
execute_query_incr_inner,ensure_can_skip_execution,execute_job_incr,
load_from_disk_or_invoke_provider_green}`;
`rustc_query_impl/src/incremental.rs::{try_load_from_disk,promote_from_disk_inner}`;
`rustc_middle/src/dep_graph/graph.rs::DepGraph::read_index`.

Add request/read identities independent of admitted jobs. Cover fast hits,
admission-race hits, post-wait hits and green ensure success explicitly. Bind
current-context `DepNodeIndex` to result identities, with a separate namespace
for previous-session serialized indices and explicit promotion mapping. A green
ensure can depend on validated success without loading a value or admitting a job.
Feeds need producer provenance, not an invented provider job.

Observe `read_index` at the native dependency decision, preserving its
`Allow`, `EvalAlways`, `Ignore`, `Forbid` behavior and duplicate-read handling.
Accepted tracked reads are semantic edges; cache hit/load/promotion events alone
are descriptive access/provenance, not proof of a consumer dependency.
Instrument request-level consumption too: the dep graph can be disabled, and
ignored/eval-always reads cannot establish complete semantic coverage. Do not
change dependency recording to make observation appear complete. Disk decoding
uses `with_query_deserialization`; distinguish restored historical dependencies
from fresh provider execution and from decoder activity.

### 3. Continuations and joins

Hooks: `rustc_query_impl/src/execution.rs::start_query`;
`rustc_thread_pool/src/join/mod.rs::join_context`;
`rustc_thread_pool/src/scope/mod.rs::{Scope::spawn,ScopeFifo::spawn_fifo,
ScopeBase::heap_job_ref,ScopeBase::complete,ScopeBase::execute_job_closure}`;
`rustc_thread_pool/src/job.rs::{StackJob::run_inline,StackJob::execute,
HeapJob::execute}`; `rustc_thread_pool/src/registry.rs::WorkerThread::execute`.

Give logical tasks and continuation segments capture-local IDs. Carry diagnostic
context with tasks, independently of inherited query TLS, and restore it on return.
Instrument both queued and inline execution: `join_context` runs A directly and
can run B through `run_inline`, so worker dispatch alone misses work. Split the
caller at fork/wait/return and associate both branch terminals with the join
continuation. Scope completion waits for its pending jobs, restores TLV and
propagates panic: emit normal and exceptional continuation outcomes distinctly.

Fork -> branch entry and required branch terminals -> join continuation are
control edges. TLS/query ancestry and task placement remain descriptive.
No implicit dependency between sibling branches, and no edge between unrelated
jobs merely executed while a joining worker helps the pool.

### 4. Resource scheduling

Hooks: `rustc_thread_pool/src/registry.rs::{Registry::inject_or_push,
Registry::inject,WorkerThread::push,WorkerThread::push_fifo,
WorkerThread::steal,WorkerThread::wait_for_jobs,WorkerThread::execute,
mark_blocked_and_wait,mark_unblocked,Registry::acquire_thread,
Registry::release_thread}`;
`rustc_data_structures/src/jobserver.rs::{Proxy::new,Proxy::acquire_thread,
Proxy::release_thread}`. `rustc_interface/src/util.rs` wires the pool's
`acquire_thread_handler`/`release_thread_handler` to that proxy.

Record enqueue/dequeue/steal and worker segments as descriptive scheduling facts.
Record allowance request, grant, release and reassignment under the proxy's
existing synchronization, including the helper callback in `Proxy::new`, buffered
handoff and permanently held allowance. Introduce explicit grant/request IDs:
`notify_one` and counters do not currently identify which waiting request receives
which release. Do not infer a release -> specific waiter edge from timestamp order.
Only an observed assignment supports a resource-control edge; external jobserver
availability remains an opaque boundary unless separately observed.

`mark_blocked_and_wait` releases allowance, waits, then reacquires it;
`mark_unblocked` changes pool blocked accounting, not allowance ownership.
Represent query-ready and resource-ready prerequisites separately. Token holding,
thread counts and wall intervals are not measurements of CPU work.

## Protocol and acceptance handoff

Implement slices in the order above with versioned typed records and
capture/context/task/request/wait/dep-node namespaces. Keep scheduler task IDs
separate from query jobs and do not serialize raw pointer `JobRefId` as durable
identity. Add independent inventories and terminal accounting per new entity
class, endpoint validation, causal sequence checks, loss markers and per-slice
coverage. Unknown endpoints, foreign contexts, dropped records and unobserved
paths must remain unknown. Do not impose a DAG on cyclic semantic query requests;
cycle recovery outcomes belong in the event/control graph.

Preserve opt-in, fail-open observation, compiler diagnostics, exit status and
cache behavior. Capture ordering facts inside existing critical sections, then
serialize outside them without adding lock-order dependencies. Never claim
full-pipeline completeness from these four slices: nonquery work, direct paths,
Cargo/process binding and external resources still require their own accounting.
Keep CPU work and execution span null until independently justified.

Guppy's acceptance checklist for subsequent implementation (not run here):
actual owned-compiler rebuild; serial/parallel nested queries; shared-query wait
fan-in; completion-before-registration race; poison and cycle wakeups; cold/warm
memory and disk hits; green ensure and feed provenance; inline/stolen joins;
scope panic and helping unrelated work; buffered/helper allowance grants;
loss/duplicate/foreign-endpoint rejection; capture-on/off artifact identity and
unchanged diagnostics/exit status with an unwritable sink. Extend the existing
`tests/test_execution_graph_query_native.py` workflow only under guppy's ownership.
Measure recording overhead on representative cold and incremental workloads
before promotion. This document makes no CPU, span or speedup claim.
