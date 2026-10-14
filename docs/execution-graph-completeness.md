# Execution graph completeness contract

## Purpose and present scope

A dependency graph is useful for architectural analysis only if omitted work is
visible. The initial native producer captures the Cargo scheduler's
**unit-prerequisite graph**, not
an entire source-to-test-result execution graph. `full_pipeline_complete` is
always false for this producer. A valid receipt is not a proof that every Rust
semantic dependency was captured, nor a bound on possible redesigned compilers.

Implementation lives in `toolchain/cargo/src/compiler/job_queue/jr_execution_graph.rs`
and its scheduler hooks. The independent client validator is
`src/execution_graph.rs`, exposed through `justrust graph <file-or-directory>`.
No wrapper, stderr parser or reconstructed logical Cargo graph supplies these
edges: declarations come directly from the prerequisite list passed to the
native scheduler, including its synthetic prerequisites and metadata handoffs.

## Capture and validation

Set `JUSTRUST_EXECUTION_GRAPH_DIR` to a private capture directory when executing
our rebuilt jr-cargo. Each scheduler invocation writes a separate schema-1
receipt with a unique capture identity. Stock Cargo does not implement this
producer. An empty capture directory is **unknown coverage**, never success.

```sh
JUSTRUST_EXECUTION_GRAPH_DIR=/private/capture justrust build --local
justrust graph /private/capture
justrust graph /private/capture --json
```

The selected local toolchain must contain the updated jr-cargo. Hosted capture
propagation, cross-machine transport and receipt reconciliation are not shipped
by this initial change. Use local execution for this diagnostic until those
boundaries have a tested native implementation. Capture-off execution remains
the default. No speedup or 96-core ceiling is claimed.

Each receipt contains:

- `schema`, `producer`, `scope`, and `capture_id`.
- `declared_nodes` and `declared_edges`, independently compared to stored counts.
- Anonymous numeric unit IDs, kinds, monotonic start/end nanoseconds and outcomes.
- Edges directed **dependent to prerequisite**, labeled `rmeta` or `all`.
- Start, successful metadata-ready and terminal events.
- `structural_complete`, `full_pipeline_complete`, issue codes and explicit opaque
  boundaries. Outcomes include success, fresh, failure, cancelled and not_started.

Every enqueued unit is declared before graph sealing. Every started unit has a
start event and terminal accounting. Unstarted units are accounted for on normal
build failure without inventing execution time. Abnormal unwinding marks the
receipt incomplete. Limits and invalid lifecycle transitions mark it incomplete
rather than silently losing coverage. Hard process termination can prevent any
receipt from being written, so absence cannot prove coverage.

The independent consumer checks declarations, unique identities, endpoints,
acyclicity, lifecycle counts and timestamp consistency. It also checks recorded
prerequisite availability against dependent starts. Metadata edges can unblock
before a prerequisite's terminal event, preserving build pipelining. Successful
or fresh terminal events can supply otherwise unobserved artifact readiness.
Invalid, unsupported, oversized or symlink receipts are rejected. Independent
validation means validation of the recorded structure, not independent proof
that the producer has instrumented all native execution paths.

## Fail-open and external correctness

The producer is observational. Capture errors must never change Cargo results,
diagnostics, cache identities or scheduling decisions. The scheduler sink is
removed from native compiler, build-script, rustdoc and executed-target
subprocess environments. No compiler flag is added. Receipts contain no source
paths, command arguments, package names or diagnostics. Publication is atomic,
create-new, and mode 0600 on Unix. Serialization is streamed through a 64 MiB
cap. Raw receipts remain local and are not telemetry uploads.

A dropped event, exceeded budget, undeclared task, interrupted capture or missing
receipt cannot support a completeness claim. Write failure produces no published
final receipt. The private temporary file is removed, and missing output means
unknown coverage. Normal compilation continues even when capture fails.

## Explicitly uncovered boundaries

This initial producer declares these opaque scopes:

- Rust compiler internals, including frontend queries and synchronization.
- Build-script subprocess internals and their spawned native tools.
- Proc-macro execution.
- Linker internals.
- Test execution.
- Cargo resolution and fingerprint computation.
- Cargo jobserver/resource waits and their synchronization dependencies.
- Cache restoration and client/service work outside the native unit scheduler.

Cache restoration, hosted admission, transport and other client/service work
also remain outside the scheduler scope. Unit lifetimes include waiting and
subprocess execution. Therefore CPU work, execution span and thread ceilings
are **unknown**, not inferred by summing task elapsed times.

## Requirements for expanding the guarantee

1. Define scope manifests listing required producers and execution boundaries.
2. Give native task/spawn/join/result-transfer interfaces enforced identities.
   Make bypasses mechanically detectable rather than relying on logging habits.
3. Track artifact and cache identities across Cargo/compiler/linker handoffs.
   Introduce versioned protocols without changing semantic artifact cache keys.
4. Add native wait reasons and synchronization relationships. Query read
   dependencies alone do not establish every execution prerequisite.
5. Reconcile producer receipts against independent process and lifecycle
   observations, including failed, cancelled and abruptly killed runs.
6. Add bounded reliable event transport with sequence/count/loss receipts and
   cross-process/cross-machine identities. A parent must expect child receipts.
7. Only then permit a full scoped completeness assertion. Expand opaque tasks
   before using the graph to infer internal architectural parallelism.

Structural accounting is the first gate, not the final guarantee. All remaining
unknowns must stay visible as integration proceeds.
