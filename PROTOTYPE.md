# First prototype: in-memory fact-driven execution

Status: agreed execution-model baseline and proposed concrete prototype contract.
Date: 2026-10-15.
Parent requirements: [ARCHITECTURE.md](ARCHITECTURE.md).

This specification finalizes what to test first. It does not claim an optimal
compiler, an implemented runtime, or measured performance. Implement in Rust
using standard Cargo, rustc, and the standard library as bootstrap/build tools.
Do not embed or delegate compiler internals to Cargo, rustc, LLVM, or the old
vendored implementations. Selective third-party utility crates are allowed;
minimize dependencies without imposing a zero-crate rule.

## Bootstrap and build workflow

Create a fresh Cargo package for the new implementation, not the deleted legacy
package or its dependency graph. Record the exact Rust toolchain with the first
implementation and performance receipts. Use `cargo fmt` and the installed
stable `justrust check`, `justrust test`, and `justrust clippy` for development
checks, tests, and lints. That existing client invokes the bootstrap toolchain;
it is not part of the new engine. Keep hosted experiment binaries, workspaces,
and caches separate from the stable installation. Do not replace the globally
installed stable client with this prototype.

A standard-library-only initial skeleton is fine, but utility crates can be
added when useful. Document their purpose and measure performance-critical
choices. Exact runnable commands and fixture options accompany implementation;
there is no remaining bootstrap-policy blocker.

## Scope

Build a single-process, multicore engine for one immutable input snapshot.
Use deterministic compiler-shaped fixtures rather than implement Rust semantics.
Keep the active graph and retained results in RAM. Support dynamically discovered
work, resumable computation, independent output publication, shared demands,
and canonical node interning. Always-on accounting is part of the prototype.

Out of scope: persistent or remote caches, cross-build invalidation, tenant
sharing, distributed scheduling, real code generation/linking, and full Rust
compatibility. The asynchronous persistence design remains a later integration
requirement. Synthetic build/check/test fixtures demonstrate engine behavior,
not actual compiler compatibility or compiler speedups.

## Node identity and representation

A node represents a logical computation and owns references to its output slots.
A fact is a finalized value in an output slot, not another unit of executable
work. Logical operations, execution segments, and result storage can have
different granularities without requiring separate heap objects for each.

Intern nodes by a canonical full key:

- Operation kind and implementation/schema version.
- Canonical parameter identities.
- Relevant semantic configuration and snapshot context.

Use hash-based lookup, compare full keys for collisions, and atomically
create-or-find so concurrent equivalent requests join one evaluation. Use a
partitioned index as the starting candidate, not a single global index lock.
Do not repeatedly hash whole subgraphs just to locate a node. Test collisions
using an intentionally collision-heavy test hasher.

Canonicalization preserves meaningful order and context. It normalizes only
representations explicitly defined as equivalent. Equivalent algorithms or equal
outputs are not automatically the same computation. Private contexts must not
be merged across authorization boundaries in future extensions.

Within this single-snapshot prototype, the graph instance can supply snapshot
identity implicitly. Stable cross-snapshot lookup and content-addressed result
identities are separate later concerns.

## Engine interface, independent of implementation language

- `request(key)`: intern/find the node, attach demand, and return output handles
  without waiting for completion. Providers define the outputs for each kind.
- `read(handle)`: return a finalized value, propagate terminal failure, or
  suspend the current dependent continuation and demand the output's producer.
- `fork(branches)`: expose independent continuations without ordering them.
- `publish(slot, value)`: finalize one immutable output and notify its waiters.
- `finish()`: complete the producer only when all required outputs are finalized.
- `fail(error)`: terminate unresolved outputs with an explicit error.
- `cancel(demand)`: detach a request; do not cancel work still needed by others.

These are semantic operations, not mandatory function names or literal API
signatures. The implementation must support atomic registration/rechecking so
publication racing with a read cannot lose a wakeup. A suspended continuation
must retain its state and resume without overlapping execution of itself.

Independent prerequisites must be requested together before dependent waits.
The runtime cannot recover parallelism hidden in arbitrary sequential code.
Batching is allowed, but completed output publication cannot wait for the rest
of a batch. Dependency waits yield workers rather than block worker threads.

## State and publication contract

Node lifecycle: discovered -> queued -> running, then suspended -> queued as
needed, ending completed, failed, or cancelled. Record readiness versus resource
admission separately. Repeated reads of completed outputs do not enqueue work.
Model state per continuation where multiple branches of one node progress
concurrently, rather than forcing them into one misleading running/waiting flag.

Output lifecycle: unavailable -> finalized(value) or terminal(error/cancelled).
An output finalizes once and never mutates within the snapshot. Producer
completion and individual output availability are distinct.

After publication, later producer failure cannot retract a finalized value.
Consequently a provider must publish only values independently valid despite
later failure. If that is impossible, delay finalization or represent a
provisional value explicitly outside the finalized-result contract. Successful
root completion requires all its mandated correctness checks, not merely
availability of an artifact-shaped output.

Retain published results for the prototype's graph lifetime and measure memory.
Do not introduce eviction/recomputation policies before that baseline is known.
Temporary working memory can be reclaimed when no longer used.

Reject unproductive dependency cycles with an explained diagnostic. Include an
explicit small fixed-point fixture demonstrating how supported recursive work
can converge without waiting on its own finalized output. Do not pretend this
fixture implements general Rust inference or recursion handling.

## Accounting invariants

- Each canonical node has one shared successful evaluation, absent explicit
  failure/retry. Initial prototype need not automatically retry failed work.
- Dependencies name particular output slots and identify the consuming branch.
- Every suspension has an observable reason and wakeup/terminal disposition.
- No continuation executes concurrently with itself or is stranded after its
  dependency becomes available. Independent branches can execute concurrently.
- Execution segments, node results, and request demands have distinct IDs.
- Shared physical execution is counted once, with all beneficiaries recorded.
- Ended requests have a result or explicit error/cancellation. An incomplete
  capture is labeled incomplete, never silently treated as reconciled.
- Every selected test/build/check root declares its own success dependencies.

Record transitions, observed edges, queue/run/wait intervals, intern hits,
publication, deduplicated demands, resources, and coverage. Use bounded buffers
and report overflow. Keep accounting enabled in normal runs, with CPU sampling
where supported and explicit coverage metadata. A controlled disabled baseline
is only for measuring instrumentation overhead.

## Worked acceptance scenarios

| Scenario | Required observation |
|---|---|
| Diamond: root requests A and B, both request C | C evaluates once; A and B reuse C and can progress independently afterward. |
| Multi-output producer | A signature consumer resumes before body output finishes; no producer-completion barrier. |
| Concurrent root demands | Equivalent requests share nodes and outputs; execution counted once. |
| Independent prerequisites | Both producers become demanded before either result arrives. |
| Publication/read race | No missed wakeup, duplicate publication, or concurrent resume under repeated stress. |
| Hash collisions | Unequal full keys remain distinct; equal canonical keys share one node. |
| Combined result | C(A,B) evaluates once and subsequent consumers reuse C without redoing its work. |
| Partial failure | Independently finalized outputs remain valid; unresolved outputs fail; required-check failure prevents root success. |
| Cancellation | One demand detaches without stopping work needed by another; no abandoned waiter hangs. |
| Cycles | Unproductive cycles explain their wait chain; explicit fixed-point fixture converges to the reference result. |
| Build -> check -> test fixtures | Common nodes execute once; test-only variants and test execution remain additional work. |
| Worker/engine fault injection | Requests terminate or capture is explicitly incomplete; no fabricated successful root. |

Compare every deterministic fixture with a simple sequential reference evaluator
and expected dependency/output sets. Randomize execution order and stress shared
requests. Use fixed seeds and retain failures for reproduction. Body/interface
edits can initially run as separate clean snapshots; proving cross-edit cache
validation is not an acceptance claim for this prototype.

## Performance experiments

Use cheap, medium, and expensive deterministic node bodies with varied graph
shapes: wide fan-out, deep chains, diamonds, high fan-in, dynamic discovery, and
multi-output publication. Report useful work separately from engine overhead.

Measure canonicalization/lookup, edge recording, waiter registration/wakeup,
queueing, result publication, allocation, instrumentation, retained bytes per
node/edge/output, contention, and total peak memory. Compare execution batching
without altering logical dependency identities or output availability.

Run worker counts 1, 2, 4, 8, 16, 32, 48, 64, and 96 on the agreed M8a target
when provisioned. Smaller local-machine tests validate behavior but cannot
establish 96-core scalability. Record hardware, configuration, seeds, workload,
source revision, and instrumentation settings. Separate one-request latency
from concurrent-request throughput.

Report elapsed time, CPU work, graph span where calculable, frontier width,
idle-worker causes, and accounting coverage. No numeric speedup or overhead
budget is asserted until measured. A working prototype passes correctness and
accounting checks; promotion of a scheduler/layout default additionally requires
representative performance evidence. Do not call synthetic engine wins Rust
compiler wins.

## Delivery order

1. Create the fresh Rust package using the bootstrap policy above, record the
   toolchain version, and document exact runnable fixture commands.
2. Canonical interning, output slots, and single-worker evaluator with accounting.
3. Suspensions, independent branches, early publication, and race-safe wakeups.
4. Multicore scheduling, shared demand, failure/cancellation, and cycle fixtures.
5. Acceptance suite, memory/overhead measurements, and 1-to-96-worker experiments.
6. Use evidence to select batching/layout policies, then integrate a small real
   compiler workload. Design cross-build validation and persistence separately.
