# Graph integrity

Status: enforced by `tests/graph_integrity.rs`. Date: 2026-10-18.

## The rule

The compiler is the graph. Every compiler fact is a typed graph output, and
every dependency is a read of that output through an engine handle. Nothing that
holds compiler state or orders compiler work lives outside the graph engine.
This is ARCHITECTURE.md section 17, made mandatory.

Allowed outside the graph:

- The engine itself: scheduler, workers and its internal synchronization.
- Ordinary code inside one node: local variables, loops, `Cell`/`RefCell`
  scratch that never leaves the computation.

Not allowed:

- Results stored in side slots (`OnceLock`, `Mutex`, shared structs) and read
  by other nodes, with a placeholder value published to the graph.
- Shared mutable state, global locks, lazy statics or process-global counters
  that order or couple compiler tasks without the graph seeing it.
- Whole-program nodes (keys without per-file or per-item identity), except the
  requested `check`/`build` roots. `check` is the explicit "every required check
  passed" fact; `build` gates the linked artifact on it.
- Per-item nodes that wait on all items: a join written as a loop of reads.
- Whole-compilation resource budgets that cap total program size. Limits are
  per task and scale with the task's own input.

How finely work is split into scheduled tasks is a measured choice. Where facts
live and how they are read is not.

## How this happened

ARCHITECTURE.md section 17 required this from the start. The first compiler
commit (405611808, 2026-10-15) stored results in `OnceLock` slots and published
a dummy `1`. Every later feature copied the pattern and added whole-program
joins (`effects_ready`, `mono_plan`, `concrete_proofs`) and shared budgets.
Milestones were gated only on correctness against stock rustc, so nothing
checked structure or scaling for three days. The 2026-10-17 ladder found the
result: 64 functions 2.17x slower than stock, more workers slower, and a hard
failure at 100+ functions from the single shared work budget.

## Gates

`justrust test --test graph_integrity` runs five checks, under a second in release:

| Test | Fails when |
|---|---|
| `compiler_code_adds_no_out_of_graph_state` | Any file in `src/compiler` or `src/project` gains a `OnceLock`, `LazyLock`, `Mutex`, `RwLock`, `Condvar`, `Atomic*`, `UnsafeCell`, `thread_local!` or `static` item beyond `tests/graph_integrity/out_of_graph_state.baseline`. Comments, strings and `#[cfg(test)]` code are ignored. |
| `no_new_whole_program_barriers` | A new parameterless node kind appears; a per-item node's direct dependency count grows when the program doubles (a disguised join); or an independent leaf function's transitive closure grows faster than recorded. |
| `compile_work_scales_linearly_ratchet` | Planning work growth from 16 to 32 functions exceeds the recorded ceiling (linear is about 2.0). |
| `program_size_ceiling_ratchet` | The required program size stops compiling. |
| `scanner_ignores_comments_strings_tests_and_lifetimes` | The static scanner misclassifies code. |

Each gate was verified against an injected violation: a new static `Mutex`, a
new parameterless join node, and a per-object loop reading every other body.

## Ratchets: debt may only shrink

Existing violations are recorded so the build passes today, and every record
may only move toward the target:

| Debt | Recorded 2026-10-18 | Target |
|---|---|---|
| Out-of-graph state primitives | `out_of_graph_state.baseline` (compiler/mod.rs side slots removed 2026-10-18; global budget mutexes and ID counters remain) | none |
| Whole-program nodes | `parse`, `resolve`, `signatures`, `effects_ready`, `mono_plan` (`source` split per file and `concrete_proofs` removed 2026-10-18) | only `check`, `build` |
| Leaf closure growth 16 to 32 | 1.205 (ceiling 1.23, was 1.481) | 1.0 |
| Work growth 16 to 32 | 1.757 (ceiling 1.81, was 1.92; six fixed source-library method proofs lower the ratio, not an algorithmic speedup) | about 2.0 |
| Largest compiling flat program | 540 functions (570 fails: main's dense per-block ownership bitsets exceed its per-task byte limit; was 64/100) | no fixed ceiling |

When a change improves a number, the tests fail until it is recorded:

- Removed primitives: `GRAPH_INTEGRITY_SHRINK=1 justrust test --test graph_integrity`
  rewrites the baseline. Commit it.
- Removed whole-program kinds, lower growth, larger compiling size: edit the
  constant named in the failure message.

Raising any recorded number, adding a baseline line, or adding a whole-program
kind requires explicit user approval. Making the test pass by loosening it is
not a fix.

## Limits of these gates

The static scan is lexical. It catches the primitives, not every way to share
state (an `Arc` of a type defined elsewhere with interior locking, for example).
The dynamic checks use one generated workload of independent leaf functions.
Treat the gates as a tripwire, not a proof. Reviews of graph-facing changes
still check the rule directly.
