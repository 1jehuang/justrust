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
