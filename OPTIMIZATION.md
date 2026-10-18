# Graph-native optimizer and code generation

Status: **decided direction, nothing implemented.** Decision recorded 2026-10-18.

## Decision

The optimizer is written from scratch in Rust and embedded in the graph engine.
It is not LLVM, a vendored or forked LLVM, Cranelift, or any external optimizer
process. This follows the clean-room rule in `AGENTS.md` (no embedded LLVM) and
the mission: minimize agent wait, regardless of implementation effort.

Rationale: the ceiling we care about is compile time, not only runtime speed.
LLVM optimizes whole modules with unstable, uncacheable intermediate results,
heavy in-memory IR and one global ThinLTO barrier. A graph-native optimizer can
optimize, cache and invalidate per function instance, with each inlining or
interprocedural decision recorded as an explicit dependency. The hypothesis is
that incremental optimized edits can approach debug-edit cost (estimated
10-100x faster than stock release edits) and cold optimized builds can scale
across server cores. **These are unmeasured targets, not claims.** Every
number must come from `bench/` receipts under `ARCHITECTURE.md` section 11.

Runtime quality must stay competitive. A fast compile that produces slow test
or release binaries is not a win. Generated-code runtime and size are tracked
beside compile time from the first slice.

## Starting point (2026-10-18)

- `src/compiler/backend.rs` emits x86-64 directly from checked IR as a stack
  machine: every value in a fixed rbp slot, arguments pushed as stack words,
  all registers caller-saved, aggregates by snapshot pointer, private ABI.
- No SSA form, register allocation, constant folding, inlining, DCE or any
  other optimization pass exists. Debug and release bootstrap profiles refer
  to how *our* compiler is built, not to optimization of user code.
- Stock rustc at `opt-level=0` still runs LLVM IR generation, FastISel and the
  fast register allocator, so stock debug output is likely faster than ours.
  This is unmeasured.

## Tiers

One code generator, several tiers selected by profile, all graph-native:

| Tier | Purpose | Content |
|---|---|---|
| T0 | current emitter | stack-machine baseline, kept as a correctness reference |
| T1 | debug / test (`opt-level=0`) | SSA lowering, cheap linear-scan allocation, trivial folding, no cross-function work. Goal: beat stock `opt-level=0` on compile time and match its runtime |
| T2 | fast optimized (`opt-level=1`) | local and intraprocedural passes, small-function inlining with recorded dependencies |
| T3 | release (`opt-level=2/3`) | full interprocedural optimization, loop and vector passes, target-tuned selection and scheduling. Goal: match then exceed stock release runtime |

T1 matters most for agent wait (check/test loop). T2/T3 are where the largest
relative compile-time gains over stock are expected.

## Graph integration

Optimization stages are ordinary graph nodes over canonical identities
(`ARCHITECTURE.md` sections 17-18), never a whole-program phase barrier.

```mermaid
flowchart LR
  A[checked lowered instance] --> B[SSA function]
  B --> C[local opt per tier]
  C --> D[function summary]
  D --> E[interprocedural decisions<br/>inline, specialize, dedup]
  E --> F[optimized function]
  F --> G[machine fragment + relocs]
  G --> H[link]
```

Rules:

1. **Unit of work and caching is the function instance.** Keys include tier,
   target/features, optimizer version, input SSA hash and the hashes of every
   fact the result consumed.
2. **Interprocedural facts flow through summaries.** A caller depends on a
   callee's summary (size, effects, purity, inlinable body hash), not its final
   machine code. An edit that leaves a summary unchanged does not dirty callers.
3. **Every inlining/specialization decision is a recorded dependency** of the
   caller on the exact callee body hash. Invalidation follows these edges only.
4. **Decisions are deterministic** for identical inputs regardless of worker
   count or schedule, so results are reusable across sessions and machines.
5. **Optimization never bypasses checking.** Unused bodies are still validated
   (`IMPLEMENTATION.md` G4). Success is published only after mandated checks.
6. **Telemetry per pass and per instance** feeds the observability graph, so
   optimizer cost and benefit are attributable.

## Rust-specific advantage

Our optimizer consumes facts that stock LLVM receives only partially:

- Borrow/region facts: precise no-alias and immutability, not just `noalias`.
- Validity and range facts: enum niches, non-null, alignment, integer ranges,
  enabling bounds- and overflow-check elimination.
- Drop elaboration and move facts: removal of unnecessary drop flags/copies.
- Whole-program instance knowledge: generic instance dedup, devirtualization.

These facts must be **preserved into lowered instances now**, rather than
discarded before the backend. That requirement applies to current G2/G3/G6 work.

## Shared prerequisites (start now)

1. Real Linux x86-64 data layout and calling convention (G6), replacing the
   private ABI. T1 register allocation and ABI design are coupled.
2. Retain alias, validity, range and drop facts in lowered instances.
3. Define the SSA function representation and its canonical hashing.
4. Express codegen as per-instance graph nodes with explicit summary edges.
5. Runtime/size measurement harness: same programs under stock `opt-level=0/3`
   and our tiers, recording compile time, run time and binary size.

## Verification

- Differential execution: every tier must produce identical observable behavior
  to T0 and to stock on the conformance corpus. Optimizer bugs are miscompiles.
- Pass-level fuzzing and translation validation where feasible.
- Determinism checks: identical outputs at workers 1/N and across machines.
- Invalidation checks: an edit dirties exactly the recorded dependents.

## Measuring the ceiling

Before T2/T3 investment, measure the share of stock build time spent in LLVM
(`justrust show` codegen phases) for debug edits, release edits and cold builds
on the representative workload. That share bounds the stock-comparable gain from
replacing the backend alone. The end-to-end 100x target additionally requires the
frontend, linking and test execution to become equally fine-grained.
