# Compiler goals

Status: design goals recorded 2026-10-28, not claims of achieved performance.

## Three goals

### 1. Correctness is non-negotiable

Preserve the external Rust compatibility contract in
[RUST-COMPATIBILITY.md](RUST-COMPATIBILITY.md) and the structural requirements in
[GRAPH-INTEGRITY.md](GRAPH-INTEGRITY.md). A faster incorrect result, stale cache
hit, skipped required check, or approximate translation is not an improvement.
Unsupported input must remain explicit rather than silently changing behavior.

### 2. Fast end-to-end compilation

Optimize actual source-to-result latency across cold builds, small edits,
checks, executable builds, and test workflows. Measure the operation the agent
or user is waiting for, not just an isolated compiler stage. Keep cold and
incremental results distinct, and include the preparation, cache handling,
linking, and test execution needed to deliver that result.

### 3. Good generated programs

Target runtime speed, memory use, startup time, and binary size comparable to
stock Rust under matched settings. Compilation speed and generated-program
quality are separate dimensions: neither a faster compiler nor a faster
executable alone establishes success on both.

Comparisons must identify the source program, target, behavior settings,
optimization settings, toolchain versions, hardware, and measurement conditions.
Do not conflate how the compiler itself was built with optimization of the
program it produces. Report unmatched pipelines or capabilities explicitly.

These are goals, not assertions that the current compiler meets them.

## Six levers for compilation speed

Parallelism and caching are important, but do not cover the entire problem.

1. **Avoid unnecessary work.** Demand only the work needed for the requested
   result, without skipping mandatory validation. Avoid duplicate computation
   and unrelated prerequisites.
2. **Use efficient algorithms for necessary work.** Reduce the time, allocation,
   and memory traffic required to compute each answer. Parallelizing an
   unnecessarily expensive algorithm is not a substitute for improving it.
3. **Reuse valid results with precise invalidation.** Share equivalent
   computations and retain reusable facts across applicable edits or sessions.
   Identity and invalidation must preserve correctness. Include cache lookup,
   validation, loading, transfer, and storage costs when judging reuse.
4. **Shorten the sequential dependency chain.** Remove unnecessary barriers and
   publish independently usable answers as soon as they are final. Also examine
   algorithm choice: narrow dependencies alone do not eliminate genuinely
   sequential work.
5. **Execute parallel work efficiently.** Expose independent work, balance ready
   work across workers, choose appropriate execution granularity, and control
   scheduling and synchronization costs. Logical fact boundaries need not be
   identical to scheduling-task boundaries.
6. **Minimize surrounding overhead.** Account for loading, allocation, hashing,
   serialization, linking, process startup, and other orchestration or I/O costs.
   Moving work to a cache or service does not make those costs disappear.

## What dependency-minimality does and does not establish

Dependency-minimality addresses unnecessary prerequisites relative to a stated
semantic contract and fact granularity. It does not by itself establish an
optimal algorithm, optimal parallel execution, inexpensive cache reuse, or
competitive generated code. A small number of broad input facts can still hide
unnecessary information or upstream work. The proof scope must distinguish
request behavior, sufficient inputs, necessary inputs, and transitive
properties; see [GRAPH-INTEGRITY.md](GRAPH-INTEGRITY.md).

The guiding principle is:

> Do less work, reuse more work, shorten unavoidable waiting, then execute what
> remains cheaply, without compromising correctness or generated-program quality.

Evaluate these dimensions separately. Node counts, cache-hit rates, and worker
utilization are diagnostic signals, not substitutes for end-to-end latency or
matched measurements of generated programs. Performance claims must follow the
repository's existing measurement requirements; this document does not replace
those gates or supply benchmark evidence.

## Background

The Rust Compiler Development Guide's
[query evaluation model](https://rustc-dev-guide.rust-lang.org/queries/query-evaluation-model-in-detail.html)
describes memoized query evaluation as a way to avoid repeating computations.
The goals above additionally require accounting for the cost of maintaining and
using that reuse, not just whether a query result is cached.
