# Compiler architecture requirements

Status: design criteria, not a selected architecture or implemented capability.
Date: 2026-10-15.

## Mission and scope

Build a fresh, vertically integrated Rust source-to-test-result system optimized
for agents. Replace the legacy implementation rather than wrap Cargo, rustc,
or their vendored forks. The intended implementation has no third-party
dependencies. How we bootstrap it, and whether that restriction includes the
bootstrap compiler and standard library, remains an explicit open decision.

Optimize first for extremely parallel, high-core-count, server-grade remote
machines. Own compilation, dependency handling, code generation, linking,
execution, caching, scheduling, and diagnostics together. Existing tool
boundaries are not architectural constraints.

The objective is the shortest reliable path from an edit to useful feedback,
not maximum CPU utilization or a fast isolated compiler phase.

## 1. Parallelism and critical-path latency

- Expose parallel work within and across projects, crates, and compiler phases
  wherever semantics permit. Do not assume a crate is the scheduling unit.
- Measure and minimize serial spans, synchronization, and critical-path work.
- Account for task creation, coordination, serialization, transfer, and startup
  costs. Fine-grained tasks must earn their overhead.
- Prioritize work that advances the requested result. Avoid speculative work
  delaying useful diagnostics or selected tests.
- Evaluate scaling across core counts, including memory-bandwidth limits,
  contention, and NUMA effects on server hardware.
- Keep local versus remote placement a measured execution decision rather than
  a separate semantic pipeline.

## 2. Cache basically any piece of work

- Make reusable work explicit at each level, not just final crate artifacts.
- Permit reuse of intermediate semantic results, generated code, linked outputs,
  and other eligible work when their complete inputs match.
- Distinguish eligibility for reuse from profitability. Avoid spending more on
  lookup, validation, transfer, and decoding than recomputation would cost.
- Support both persistent state across edits and reusable results across runs
  and machines. Do not assume these require the same representation.
- Version compiler semantics, representations, configuration, and cache keys so
  incompatible results cannot be reused.
- Treat test-result reuse separately from binary reuse. Nondeterministic,
  time-dependent, networked, or externally stateful tests are not automatically
  eligible for result caching.

## 3. Correctness of reuse and hermetic inputs

- Declare and track every effective input, including source, configuration,
  target, flags, environment, generated files, external tools, and tool versions.
- Untracked or uncontrolled inputs must prevent unsafe reuse, not silently
  produce stale results.
- Track negative observations too, such as missing files or failed lookups,
  when they influence a result.
- Define how dynamic dependencies are discovered and validated before reuse.
- Distinguish deterministic reusable work from effectful operations. Build
  scripts and proc macros require explicit input and isolation policies.
- Never change a build's meaning merely to improve cache hit rate.

## 4. Fine-grained incrementality and invalidation

- Invalidate the smallest semantically affected work after an edit rather than
  automatically rebuilding a whole crate or its downstream dependents.
- Maintain stable identities across edits without tying identity to volatile
  positions or execution order.
- Separate implementation changes from externally observable semantic changes
  where correctness permits downstream reuse.
- Support additions, deletions, configuration changes, macro expansion changes,
  and dependency changes, not just edits inside function bodies.
- Explain why each invalidated result must be recomputed.
- Bound persistent-state growth and provide safe eviction and recovery.

## 5. Native work graph, analytics, and diagnostics

The scheduler, cache, and diagnostics must share the actual work graph.
Do not reconstruct the authoritative graph from logs or inferred timings.
Mapping work and dependencies was a recurring problem in the legacy stack.
Make that information a first-class design requirement from the beginning.

For every task, expose:

- Stable identity, operation kind, and associated source or semantic entity.
- Declared and discovered inputs, dependencies, and produced outputs.
- Demand origin: what request required this work and why.
- Cache eligibility, key/version, hit or miss, and the reason for that decision.
- Invalidation cause and the chain that propagated it.
- Queue time, execution intervals, dependency waits, and transfer time.
- Worker placement, attempts, resource usage, outcome, and diagnostics.

Distinguish potential dependencies, discovered dependencies, demanded work,
executed work, reused work, speculative work, cancelled work, and retries.
A dependency graph and an execution trace are related but not identical.

Support critical-path analysis, parallelism and contention analysis, accounting
for work avoided by reuse, and drill-down from a build summary to individual
tasks. Do not double-count overlapping intervals or equate summed CPU time with
wall time. Mark unavailable or sampled measurements honestly.

Observability must have a measured overhead budget. Recording failures must
not alter compilation semantics or corrupt results. Preserve full errors and
failing-test output, not just a summarized verdict.

## 6. Determinism and reproducibility

- Equivalent effective inputs should produce equivalent semantic results
  regardless of worker count, scheduling, cache state, or placement.
- Define which artifacts must be byte-for-byte reproducible and how incidental
  metadata such as paths and timestamps is handled.
- Detect and investigate nondeterministic results rather than hiding them
  behind whichever cache entry wins a race.
- Make failures reproducible from recorded input identities and configuration,
  subject to access controls and source-retention policy.

## 7. Resource-aware scheduling

- Budget CPU, memory, I/O, network, and storage explicitly.
- Prevent oversubscription, memory exhaustion, unbounded queues, and starvation.
- Provide backpressure and safe cancellation when an edit supersedes work.
- Support concurrent agents and projects with priority and fairness policies.
- Account for cache locality, transfer costs, and server topology when placing
  work. More distributed tasks are not automatically faster.
- Define cancellation boundaries for effectful operations and retain completed
  reusable work when it is safe to do so.

## 8. Isolation and trust

- Sandbox untrusted build scripts, proc macros, native tools, and tests with
  explicit filesystem, environment, network, and resource policies.
- Authenticate cache provenance and enforce authorization before lookup,
  retrieval, publication, or execution.
- Isolate tenants and private inputs. A shared cache must not leak private
  source, artifacts, diagnostics, or sensitive metadata.
- Validate externally supplied artifacts and prevent cache poisoning.
- Never trade correctness or tenant isolation for a faster build.
- Keep multitenant service implementation in the separate cloud repository.

## 9. Failure recovery

- Worker crashes, disconnects, cancelled requests, and interrupted uploads must
  not corrupt reusable state or publish partial artifacts as complete results.
- Publish completed results atomically and validate their integrity.
- Preserve completed work across retries where safe, and identify retries in
  analytics without counting them as new logical work.
- Distinguish source errors, unsupported features, infrastructure failures,
  resource exhaustion, and internal compiler bugs.
- Define recovery behavior for damaged caches and incompatible persistent
  state. Do not turn infrastructure failure into a misleading success.

## 10. Explicit external correctness contract

Define supported behavior and test it. Independence from the legacy internals
does not remove obligations to the projects being compiled.

The contract includes:

- Rust language semantics, type checking, borrow checking, and diagnostics.
- Existing manifests, dependency resolution, features, and project structure.
- Build scripts, generated inputs, declarative macros, and procedural macros.
- Native-library interfaces, target/platform ABIs, and linking behavior.
- Debug information, executable behavior, test execution, and exit status.

Track unsupported behavior openly. Do not imply full Rust compatibility before
it is demonstrated. A stock-toolchain reference used in verification must stay
separate from the native implementation and must not consume private formats.

## 11. Measurement and acceptance

Treat these as distinct optimization targets:

- Cold project discovery and dependency acquisition.
- Cold checks and executable builds.
- No-change requests and repeated warm edits.
- Checks, debug builds, release builds, and selected versus complete tests.
- Single-agent and concurrent-agent workloads.
- Warm and cold caches, local and remote execution, and partial failures.

Use representative, reproducible, session-weighted workloads to decide
tradeoffs. Report distributions and command/change breakdowns, not just an
aggregate mean. Include end-to-end wall time, critical-path span, CPU work,
memory, network/storage cost, cache overhead, and observability overhead.

Compare designs against the preserved implementation and stock tools with
identical workloads and controlled conditions. An architectural change,
process-boundary removal, higher core utilization, or cache hit rate alone is
not evidence of a speedup. Every performance claim requires measurements and
every reuse optimization requires correctness checks.

## 12. Observability as the engine's accounting system

Observability is part of the execution model, not logging wrapped around it.
Always-available profiling means bounded, measured overhead, not zero cost.
Use one always-on observability system, not three user-facing capture tiers.
Require exact work identities, dependency records, lifecycle accounting, and
cache decisions, with automatic profiling and explicit sampling/coverage
metadata. Exact structural accounting does not require recording every
instruction or memory access.

Keep retention and buffers bounded. Start with capture enabled in the prototype
and measure execution, storage, transfer, and scheduling interference against
controlled instrumentation-disabled runs. Introduce selective deeper capture
only if measurements demonstrate that a particular measurement is too costly
to retain continuously. Unexpected slow runs should already have useful
evidence without requiring the user to reproduce them with profiling enabled.

### Identity, lifecycle, and causality

Distinguish a logical operation from its executions, retries, cache validation,
and scheduling batches. Connect each operation to source entities, compiler
operations, generated-code provenance, inputs, outputs, and demand origins.

Specify a lifecycle state machine covering created, dependency-blocked, ready,
queued, running, suspended, reused, completed, failed, and cancelled work.
Record transition reasons. Every unfinished operation must have an explainable
current state, including what it is waiting for. The exact legal transitions
and handling of concurrent demands remain design work.

Link client requests, coordinator decisions, workers, cache operations, and test
execution through causal identities. Use monotonic durations within a clock
domain and expose cross-machine clock uncertainty. Do not infer exact global
ordering from unsynchronized timestamps.

### Scheduler and resource visibility

Expose ready-queue depth, active and idle workers, concurrency limits, memory
pressure, placement/locality choices, stealing, priority changes, and why ready
work did not run. Account for hashing, encoding/decoding, cache validation,
network transfer, startup, coordination, cleanup, and background work, not just
compiler task bodies.

Separate elapsed time, CPU work, dependency waits, queueing, and resource
contention. Include memory, allocations, I/O, and sampled stacks where supported.
Attach samples to task context, including task migration and suspension.
Keep allocation and high-volume capture enabled where measured costs permit;
any sampling or selective capture must report its coverage and justification.
Label sampled, estimated, unavailable, and directly measured values distinctly.

Track instrumentation coverage, dropped events, dropped samples, incomplete
captures, and unexplained time. A partial trace must not look complete.
Historical execution costs attached to cached results are provenance, not CPU
work or elapsed execution time performed in the current request.

### Questions and interfaces

Provide live state, timeline, dependency and critical-path exploration, sampled
profiles, and machine-readable exports that can answer:

- What is happening now, and what is blocked or idle?
- Why am I waiting for this result?
- Why did this work run, become invalid, or miss the cache?
- Which request demanded it, and which result depends on it?
- What was reused, from where, and what did reuse itself cost?
- What changed between these two runs?
- Where is time or resource consumption still unexplained?

Before selecting a telemetry backend, specify the work identity model,
lifecycle state machine, event schema, and accounting invariants. Include
logical work versus attempts, shared work attributed to multiple requests,
overlapping intervals, cancellation, and incomplete captures in those invariants.

### Accounting correctness and architectural comparability

- Test accounting against known small graphs, cache hits, shared demands,
  failures, retries, cancellation, and incomplete captures. Validate legal
  transitions and reconcile totals within documented measurement accuracy.
- Every request, task evaluation, diagnostic, and artifact must identify its
  immutable input snapshot. Show superseded work, and never present mixed-edit
  results as one coherent build.
- Separate semantic dependency edges from scheduling constraints such as locks,
  memory admission, worker capacity, and transfers. Explain both without
  misrepresenting a resource wait as a language dependency.
- Attribute shared work to all beneficiary requests while counting physical
  execution once. Specify latency attribution separately from cost allocation.
- Explain memory ownership and lifetimes by representation, cache, and request,
  including retained state and eviction behavior, not just peak process RSS.
- Record optimization decisions for reuse, recomputation, placement, batching,
  speculation, and eviction, including predicted costs and observed outcomes.
- Support selective compiler-result explanations connecting conclusions to
  contributing facts and inputs. Detailed semantic proof capture is distinct
  from the complete structural work graph and must justify its overhead.
- Keep user-level metrics and workload identities comparable when internal
  task boundaries change. More tasks or cache hits alone are not progress.
- Bound telemetry storage and define overflow, collector-failure, retention,
  redaction, and export policies. Never leak source, environment values, paths,
  or tenant-sensitive metadata through traces or profiles. Mark lost evidence
  explicitly rather than silently claiming complete accounting.

Specify worked example traces and accounting invariants before formats or
dashboards. Rank optimization hypotheses by measured impact and confidence:
operation duration is not necessarily recoverable request latency. Validate
critical-path savings through interventions, not just profile percentages.

### Architecture experiments and bottleneck hill-climbing

Store versioned experiment receipts with source snapshot, compiler revision,
configuration, hardware, cache state, workload, and instrumentation level.
Record correctness outcomes alongside performance results.

Compare repeated controlled runs on end-to-end latency, critical-path span,
CPU work, memory, and infrastructure cost. Rank suspected bottlenecks by measured
impact. Label hypothetical savings and historical avoided-work estimates as
estimates, not demonstrated speedups. Test one intervention at a time where
possible, retaining correctness and regression gates across workload types.
Measure the instrumentation overhead itself so a profiling change cannot be
mistaken for a compiler improvement or regression.

## 13. One reusable graph across build, check, and test

Build, check, and test request different results from the same reusable work
graph, not independent pipelines. The target is for a check or test request
following a build to require almost no repeated compiler work when its inputs
are already covered. This is a measurement target, not a promise of free work.

- A check reuses completed parsing, expansion, type checking, and borrow checking
  when semantic inputs and relevant settings match.
- A test request reuses compatible semantic results and machine code, computing
  only the missing test-specific work.
- Explicitly model differences such as `cfg(test)`, feature selection, targets,
  profiles, test harnesses, instrumentation, and code-generation settings.
  Ordinary build completion does not prove these variants were computed.
- Do not key reusable semantic work by command name when the command does not
  change its meaning. Equally, never omit an input that does change its meaning.
- Keep test binary reuse distinct from test-result reuse. Executing tests still
  costs time, and result caching requires an explicit eligibility policy.

Observability must separate reused work, newly required work, validation and
materialization costs, and actual test execution. Benchmark build-to-check,
build-to-test, test-to-build, and edit-followed-by-command sequences, not just
isolated invocations. Verify equivalent results against clean recomputation.

## 14. Reuse across sessions, projects, libraries, and tenants

Reuse must not stop at one command or workspace. Support eligible work from
this session, earlier builds, other projects, library prebuilds, and authorized
multitenant shared caches.

Library results and intermediate artifacts require identities derived from
exact source, complete effective semantic inputs, target, relevant settings,
and versioned compiler operations/formats. A package name and version alone
are not sufficient. Intermediate sharing must preserve dependency closure,
configuration, and provenance, not just reuse a matching-looking final file.

Public inputs can produce work eligible for cross-tenant sharing under an
explicit policy. Public source alone does not make a result public: private
inputs, environment, generated data, or dependency results can make the derived
work private. Keep private work and its metadata tenant-scoped unless explicitly
shared through an authorized mechanism. Cache keys are not authorization.

Require verified provenance, artifact integrity, poisoning defenses, and
publication authorization. Do not trust arbitrary tenant-produced results
merely because their claimed keys match. The trust and verification mechanism
remains a design decision. Implement multitenant service policy in the separate
cloud repository, not here.

Report reuse origin: current session, previous build, library prebuild, or
authorized shared cache. Expose lookup, validation, download, decoding, and
materialization cost. A hit is not automatically a win: compare net request
latency and resource cost against recomputation, and label estimates honestly.
Do not expose another tenant's identity, private inputs, or sensitive cache
metadata when explaining reuse.

## 15. Memory-first compilation and asynchronous persistence

Agreed initial design candidate, subject to correctness and performance tests.
Compilation is memory-first, not a sequence of disk-mediated compiler phases.

Keep the complete active project's graph, dependency indexes, and live execution
state resident in memory. Keep compiler representations, intermediate results,
and newly produced artifacts in RAM while useful. Pass results directly between
operations without serializing and reloading merely to cross phase boundaries.
Full active-graph residency does not mean retaining the entire fleet's cache,
every tenant's history, or all historical result payloads in RAM.

Release dead intermediates when safe rather than retaining everything until the
request ends. Memory accounting must distinguish active computation, reusable
resident state, pending persistence, and reclaimable data. Any move toward
partial graph residency must be justified by measured capacity or latency,
not assumed as an architectural requirement.

### Publication and durability are separate

Publish completed immutable results immediately to authorized in-memory
consumers. Persist immutable batches asynchronously instead of waiting until
the entire build finishes. This allows checkpointing during long builds and
persistent edit sessions without making disk the normal communication path.

Track computed, persisted, and available-remotely states separately. In-memory
publication does not claim crash durability or remote availability. A remote
consumer needs an authorized transfer or a published remote artifact, not just
a local completion event. Persisted manifests must never reference incomplete
or unavailable payloads as if they were durable.

Persistence should not block dependent computation unless durability or
cross-machine availability is required. Bound pending writes and account for
persistence CPU, memory, bandwidth, and I/O interference. Define backpressure
and safe reclamation rules so an asynchronous queue cannot grow indefinitely
or discard the only copy of a result promised for persistence.

Provide an explicit request-completion flush/checkpoint boundary. Distinguish
result-ready latency from durable-checkpoint completion and define which one a
command's completion contract promises. The exact flush policy remains design
work, not an implicit requirement that every request synchronously uploads all
results. On a crash, unpersisted work may be lost and recomputed; persisted work
must recover consistently without treating partial writes as completed results.

### Initial data-model proposal

Use explicit operation, snapshot evaluation, dependency edge, result, execution
attempt, and demand records linked by IDs. Keep semantic dependencies separate
from scheduling waits and execution history. Reverse-dependency indexes are
rebuildable acceleration structures. Handle cycles explicitly rather than
assuming all compiler relationships form a DAG.

Compact IDs, chunked tables and adjacency storage, identity hash indexes, and
partitioned ownership are initial layout candidates, not settled choices.
Avoid assuming a single global graph lock or per-node heap allocation.

### Validation

Measure bytes per node/edge, graph and payload residency, peak total memory,
persistence interference, pending-write growth, loading/recovery cost, and
end-to-end request latency across core counts and concurrent projects. Compare
asynchronous persistence with completion-only persistence under controlled
conditions. Exercise crashes, incomplete batches, cancellation, and shared
consumers. No performance advantage is claimed before those measurements.

## 16. Initial server target and parallel scaling

Initial target: AWS EC2 `m8a.24xlarge`, with 96 physical CPU cores and
384 GiB RAM. M8a exposes one physical core per vCPU. This is the starting
measurement platform, not a claim that 96 workers is optimal for every request.
No instance is provisioned by this decision.

Use worker limits of 1, 2, 4, 8, 16, 32, 48, 64, and 96 to measure scaling.
Periodically compare with `m8a.48xlarge` at 192 physical cores and 768 GiB RAM.
Worker limits on the smaller instance do not substitute for measuring the
larger instance's actual topology, bandwidth, and contention behavior.

Measure single-request latency separately from concurrent-request throughput.
Track critical-path span, CPU work, scheduler overhead, memory footprint and
bandwidth pressure, NUMA/locality effects, persistence interference, and cost
per completed workload. Capture actual CPU topology, affinity, resource limits,
and competing activity in experiment receipts. Do not assume physical core
count alone predicts compiler performance.

Keep scheduling adaptive to the work available and memory budget rather than
hard-coding the initial machine size. Consider memory-optimized alternatives
only when measurements justify them. Recheck regional availability and pricing
before provisioning; prices are not architectural constants.

Hardware specification reference: https://aws.amazon.com/ec2/instance-types/m8a/
(checked 2026-10-15).

## Open architectural decisions

These requirements do not yet choose the mechanisms below:

1. Bootstrap language/toolchain and the precise scope of no dependencies.
2. Initial supported language subset, target, project inputs, and acceptance
   workload, followed by a compatibility expansion plan.
3. Stable identities, task granularity, graph representation, and dynamic
   dependency validation.
4. Compiler representations, ownership/borrow analysis, code generation,
   linking, and standard-library strategy.
5. Persistent process/state boundaries and on-disk result formats.
6. Shared-cache protocol, trust/provenance model, and publication rules.
7. Local/remote placement, distributed scheduling, isolation, and recovery.
8. Telemetry schema, retention/redaction, and instrumentation overhead budget.

Choose these together using correctness constraints and measured agent wait.
Do not recreate legacy component boundaries by default.
