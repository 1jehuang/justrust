# Shared work and single-build latency

Design intent recorded October 14, 2026.

## Objective

Minimize redundant work and deliver each build result as fast as possible.
Keeping a server busy is not the objective. Other tenants can consume spare
capacity, but aggregate utilization is not evidence that an individual build
is faster.

These are joint goals, not a license to sacrifice latency for CPU savings:

- Reuse correct results rather than recomputing them.
- Share identical in-flight work rather than compiling it once per requester.
- Execute the remaining work with enough parallelism to minimize user wait.
- Measure both result latency and total resource cost. Explicitly report
  tradeoffs when reducing one increases the other.

## The graph model

Model the source-to-test-result workflow as a dependency graph of tasks and
reusable results. This is a design model, not a claim that the current system
already has one unified scheduler or cache for every compiler phase.

A task is ready only when its prerequisites are available. Adding cores does
not accelerate a dependency chain whose next task must wait for the previous
one. Useful parallelism requires independent ready work, not merely many nodes.

For a fixed remaining task graph, let W be total CPU work, S the longest
weighted dependency path, and P the available cores. Ideal execution time is
bounded below by max(W / P, S). Real execution also pays scheduling,
synchronization, transfer, memory-bandwidth, and other resource costs. This
CPU-only bound does not predict actual wall time by itself.

Hypothetical example: 960 CPU-seconds of independent, perfectly balanced work
could execute in 10 seconds on 96 cores. A separate 3-second serial prerequisite
makes the ideal total at least 13 seconds. These are explanatory numbers, not
benchmark results.

## Making 96 cores useful to one build

After cache reuse, the remaining graph must expose enough substantial,
independent, ready work to occupy the machine. It need not have exactly 96
nodes, but it needs roughly 96 cores' worth of runnable work during the phases
we want to accelerate.

Prioritize experiments that shorten the critical path or expose concurrency
inside its expensive tasks. Splitting a task helps only if the pieces can
execute independently and the saved time exceeds partitioning, coordination,
and result-merging overhead. Compiler, codegen, linker, and test boundaries
are redesignable, provided external correctness is preserved.

Do not impose 96-way parallelism everywhere. Small tasks, memory pressure,
contention, and narrow dependency chains can make lower concurrency faster.
Choose task granularity and concurrency from measurements.

## Sharing before scheduling

Proposed execution order:

1. Resolve reusable results using complete, versioned identities.
2. Coalesce equivalent in-flight requests into one producer with multiple
   consumers. Cancellation by one consumer must not break the others.
3. Schedule uncached work, prioritizing tasks that unblock the build's
   critical path and respecting memory and other resource limits.
4. Use spare capacity for other tenants without allowing their work to
   unnecessarily delay latency-sensitive ready tasks.

Cross-tenant reuse must obey the cloud security policy. Share only authorized,
trusted results with matching relevant inputs, toolchain, flags, target, and
execution assumptions. Keep private inputs and artifacts isolated. Build
scripts, proc macros, native dependencies, and environmental inputs require
correct identities or conservative non-reuse. Cache hits must never change
program behavior or diagnostics correctness.

## Measurement and acceptance

Evaluate proposals against the comprehensive session-weighted synthetic
workload under the repository's benchmark rules, with identical inputs and
execution mode before and after. Supplement it with representative large
single-build and hosted multi-tenant workloads.

Record:

- End-to-end latency, including queueing, cache lookup/download, execution,
  and delivery, with cold, warm, and incremental cases distinguished.
- Total CPU work and redundant work avoided, including duplicate in-flight
  requests and any speculative/background prebuild cost.
- Critical-path duration and which tasks determine it.
- Ready-task concurrency, effective core use, memory pressure, and contention.
- Correctness, cache isolation, cancellation, failure propagation, and
  diagnostics behavior.

A speedup claim needs measured before/after evidence. High CPU utilization,
more graph nodes, or removal of a process boundary is not sufficient.

## Related plans

- [Toolchain ownership and integration](toolchain.md)
- [Parallelism map](parallelism-map.md)
- [Remote strategy and service boundaries](remote-strategy.md)
