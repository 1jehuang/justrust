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

## Structural enforcement (approved 2026-10-20, in progress)

The ratchets below detect coarse nodes after they are written. The target is
an engine API in which they cannot be expressed. Approved by the user
2026-10-20. Every agent touching `src/engine.rs` or `src/compiler/` must
follow these rules now, even before the API enforces them:

1. **Typed keys.** Each node kind declares a key type (`FileId`, `ItemId`,
   `InstanceId(DeclId, TypeArgs)`, ...). Raw `Key::new(kind, vec![..])` is
   being removed. Only the sealed `Check` and `Build` roots are parameterless.
2. **No for-all-items reads.** Node code must not iterate `0..*_count()` and
   read each item. Bulk needs go through an explicit, keyed completeness fact
   (per file, per module, per trait) or the two roots.
3. **Request, then await.** A node declares all independent prerequisites
   before awaiting any (`cx.request_all` then await). Requesting inside a loop
   body that awaits is a bug.
4. **Facts are provably immutable.** Published types implement `Fact`, which
   is rejected for anything reaching `Mutex`, `RwLock`, `OnceLock`, `Cell`,
   atomics or budget handles.
5. **Per-node budgets.** The engine hands each node its own meter, sized from
   its input. No shared budget handle may be stored in a fact or node state.

Lanes (coordinator: shark): typed-key API first, then in parallel
`MONO_PLAN` -> per-instance `INSTANCE`, `CHECK` fan-in via per-file item
facts, `parse`/`resolve`/`signatures` split, `PlanCtx`/`ValidatedTraitWorld`
memo state into keyed nodes, budgets per node. Coordinate with shark before
editing `src/engine.rs` or the dispatch in `src/compiler/mod.rs`.

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
| Largest compiling flat program | 1208 functions (1209 fails at the unchanged 32,000,000 work limit; immutable normalized-schema index reuse removes repeated full callable scans, was 1165/1166) | no fixed ceiling |

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

## Reachable-fact mutable state (added 2026-10-19)

Facts must be immutable values. A fact that carries a `Mutex`, `OnceLock` or
atomic anywhere inside it is shared mutable state in disguise: consumers read
it through a graph handle, but writes through it are invisible to the graph.
The lexical scan above misses this when the primitive is declared in a
different file from the fact.

| Test | Fails when |
|---|---|
| `graph_facts_reach_no_new_mutable_state` | A type read or published as a graph fact reaches a new `(root, carrier, primitive)` triple not in `tests/graph_integrity/fact_reachable_state.debt`. Or the inventory has shrunk and not been recorded. |
| `reachable_fact_gate_follows_aliases_wrappers_and_generics` | The analysis stops following `Arc`/`Vec`/`Option`/map generics, type aliases, tuple structs, enum struct variants and `where` clauses, stops terminating on cycles, or starts flagging test, comment or string text. |

How it works: fact roots are every type named in a typed fact read
(`facts.get` with `Arc<T>`, `read_fact`, `fact_of`, typed helpers) or a
`publish_fact(_, Type { .. })` literal in `src/compiler/mod.rs`. A type
reference graph is built from every `struct`, `enum`, `union` and `type`
declaration in `src/compiler` and `src/project`, excluding test code. A
breadth-first search from each root records every carrier type that directly
holds a mutable primitive, with one shortest witness path.

Recorded 2026-10-19: 11 fact roots reach 6 carriers, 62 entries (72 before
construction-final declared reductions were frozen, 77 before the name-versus-type
parser fix below):

| Carrier | Mutable field | Owner of removal |
|---|---|---|
| `PlanCtx` | `state: Arc<Mutex<PlanState>>` | llama (cr-graphunify) |
| `ValidatedTraitWorld` | `memo`, `projection_memo` | llama |
| `PlanBudget` | `usage: Arc<Mutex<PlanUsage>>` | budget lane |
| `PlanReservation` | `usage` (shared ledger) | budget lane |
| `ExpansionBudget` | `state: Arc<Mutex<State>>` | budget lane |
| `InvocationBudget` | `state: Arc<Mutex<InvocationState>>` | budget lane |

The budget carriers are reached because facts retain the shared `PlanBudget`
or expansion budget handle (for example `Plan`, `ClosedCfg`, `ModuleArena`).
Removing them means facts stop holding the ledger: charge budgets to the
producing node and drop the handle before publication.

Only type positions are references: field types, tuple and variant payload
types and alias right-hand sides. Field names, enum variant names,
discriminants, attributes, visibility, generic parameters and bounds are not.
An earlier version read every identifier in a body, so the enum variant
`TokenError::Budget(BudgetFailure)` was mistaken for `struct Budget` and the
variant `SourceErrorKind::Module` for `struct Module`. Fixing the parser removed
5 false `UnitResult` entries and corrected two `ParsedProgram` witnesses; their
carriers are still reached through real `Program` fields.

Limits, stated rather than claimed away:

- Types are matched by identifier, not resolved path. Same-named types
  (`State`, `Function`, `Program`) merge conservatively, which can add false
  positives but not hide a real one with the same name.
- `dyn Trait` fields reach every non-test `impl Trait for Type` in scope
  (pending approval, branch `graph-integrity-dyn-pending-approval`). This
  exposed `ValidatedTraitWorld.schema: Arc<dyn SchemaCx>` -> `SchemaAdapter.state`,
  the same `PlanState` mutex as `PlanCtx`, under 10 more fact roots.
- Generic parameters are not substituted. `Wrapper<T>` with `T = Mutex<_>` at
  a use site is caught only when the use site names `Mutex`, which a field type
  does. A `fn`/closure capture, or a type from another crate or `std` that wraps a lock
  internally (for example `std::sync::mpsc`) is not followed.
- Roots come from `src/compiler/mod.rs` call shapes. A fact read through a new
  helper with an unrecognized shape is not a root; the test fails if fewer
  than 8 roots are found, so the API change is noticed. A recorded root that
  stops being discovered while its carriers still hold mutable state fails
  the test even under `GRAPH_INTEGRITY_SHRINK`: lost coverage is never
  recorded as progress. Only removing a carrier's mutable field shrinks the
  inventory.
- `Cell`/`RefCell` are omitted because `publish_fact` requires `Send + Sync`.

Treat it as a structural tripwire with explicit, inspectable witnesses, not a
proof of immutability.
