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
`MONO_PLAN` -> per-instance `INSTANCE` (done 2026-10-22), `CHECK` fan-in via per-file item
facts, `parse`/`resolve`/`signatures` split, `PlanCtx`/`ValidatedTraitWorld`
memo state into keyed nodes, budgets per node. Coordinate with shark before
editing `src/engine.rs` or the dispatch in `src/compiler/mod.rs`.

## Dependency contracts and proof pilot (proposed 2026-10-28)

The intended property is **dependency-minimality**, not simply small nodes.
A graph records the dependencies its producers request; it does not establish
that those dependencies are necessary. A per-declaration signature node that
extracts one signature from a whole-program context still inherits that context's
dependencies. Renaming or wrapping the request does not remove the barrier.
Incremental reuse likewise does not make a broad producer independently
computable when its result is missing or invalidated.

### Define the guarantee before selecting a prover

Fix a node's semantic specification, supported input domain, fact granularity,
and observable output, including relevant failure distinctions. For a declared
input set D, distinguish:

- **Sufficiency:** any two valid input worlds that agree on D yield the same
  specified answer. No required semantic input is missing.
- **Set minimality:** no proper subset of D is sufficient across the valid
  domain. Necessity witnesses must respect the domain's consistency constraints;
  facts cannot be varied independently when their meanings constrain each other.
- **Execution-specific minimality:** the facts actually requested on one path
  are necessary for that case. This is a separate, stronger target than proving
  a fixed dependency set minimal across all cases.

These are relative guarantees, not proof of optimal compilation speed or the
globally best node vocabulary. A different fact representation or algorithm
can change the dependencies. Logical facts also need not correspond one-to-one
with scheduled tasks: batching may reduce scheduling overhead without making
unrelated facts prerequisites of an answer.

### Structural approach

Give each producer its own scoped source input and a role-specific typed lookup
interface, rather than access to a whole-program `PlanCtx`. Apply the restriction
recursively to supporting producers. A signature producer should consume its
declaration header and explicitly request referenced declarations or types,
not extract its answer from a completed global signature table. Recursive
declarations need explicit cycle or fixed-point semantics, not a hidden join.
Legitimate completeness requirements remain explicit keyed facts, and the check
root must still validate required items even when unused.

This can forbid known broad-access patterns by construction, but scoped access
alone does not prove minimality: a producer could still request an unrelated
fact through an allowed lookup. Existing graph-shape gates remain necessary.

### Bounded experiment and tool choice

Start with one signature-producing path and a precisely bounded supported
subset. Establish that its answer is correct and independent of unrelated
function bodies, and that its implementation cannot request those bodies or
the whole-program signature context. State allowed header and lookup inputs,
cycle behavior, and diagnostic obligations explicitly. Do not claim full
dependency-minimality from this narrower noninterference/access contract.

Evaluate tools on that same contract before choosing a project-wide approach:

- **Lean:** candidate for formalizing the dependency model and proving reusable
  composition rules for a restricted node-definition language.
- **Verus:** candidate for verifying contracts closer to the executable Rust
  producer, subject to the language features and libraries it can verify.

A mathematical model proof alone does not certify the Rust implementation.
Identify the correspondence proof, verified generation, or other implementation
connection, and record any unverified boundary and trusted assumptions. Choose
based on that connection, expressiveness for the contract, and proof maintenance
cost, not on a presumed universal best prover. Neither tool automatically
discovers minimal dependencies.

The pilot should deliver a checked specification and proof with its trust
boundary, plus executable regressions for unrelated-body independence and
forbidden broad requests. Any production integration must retain semantic,
invalid-input, forged-fact and graph-integrity gates. Compare the affected
workload before and after before claiming a speed improvement. This proposal
does not select a prover, add a dependency, certify existing nodes, or relax a
ratchet.

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
| Out-of-graph state primitives | `out_of_graph_state.baseline`: empty since 2026-10-22 (budget ledgers per node, ID counters issued, `PlanState` mutex removed) | none (reached) |
| Whole-program nodes | `resolve`, `signatures` (`parse` deleted 2026-10-26: per-file `ADMIT` chain renders file ids and limits, RESOLVE merges `PARSE_FILE` facts, probe inputs and splice-ordered syntax replay still assemble inside RESOLVE; `mono_plan` replaced by per-instance `INSTANCE` 2026-10-22, 32e3f6c70; `source` split per file and `concrete_proofs` removed 2026-10-18, `effects_ready` per item, debt root deleted 2026-10-20) | only `check`, `build` |
| Leaf closure growth 16 to 32 | 1.000 measured 2026-10-22 after 32e3f6c70 (ceiling constant unchanged) | 1.0 |
| Planning work growth 16 to 32 | 1.008 measured 2026-10-22 (optimizer 1.013; ceilings unchanged) | about 2.0 or better |
| Required flat program size | 2000 functions; the shared 32,000,000 work cap was removed with per-node budgets (61429d595). 3000 currently hits a subtract overflow at ownership.rs state_bytes | no fixed ceiling |

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

| Carrier | Mutable field | Status |
|---|---|---|
| `PlanCtx` | `state: Arc<Mutex<PlanState>>` | removed 2026-10-22 (00a747385) |
| `ValidatedTraitWorld` | `memo`, `projection_memo` | removed (e74630946, node-local ProofScratch) |

**2026-10-22: `fact_reachable_state.debt` and `out_of_graph_state.baseline`
are both empty.** `PlanCtx`/`SchemaAdapter` no longer hold `PlanState` behind
a mutex: canonical nominals and source schemas are sealed immutable fields,
and the interning memo, source context and reservations are per-call scratch.
Nominal identity is a canonical digest of (resolved declaration, ordered
actuals), not an interning-order ordinal (56bcecc5b). Every process-global ID
counter was replaced by owners issued from the producing node's
`OwnerIssuer` (a7cb0b967, 2b8954723, 9293e01f8, 03a46dc28, 3f7d7517d,
e1c447312). Any new entry in either file now needs explicit user approval.

Budget carriers removed 2026-10-22 (31 entries). `PlanBudget`,
`PlanReservation`, `ExpansionBudget`, `InvocationBudget`, `ByteLease` and
`StateLease` are now immutable `{id, limits}` tokens or scope-tagged leases.
Every charge lands in a typed ledger owned by the polled engine node
(`engine::with_ledger`, keyed by budget id inside `Meter::ledgers`), or in a
thread-local detached ledger outside polling. Leases refund only the scope
that paid them. Work, node and byte ceilings are per node and scale with the
node's declared input (`scale_task_work`); the 32,000,000 shared work cap is
gone. Compilation-wide usage is a post-run fold over node ledgers
(`PlanBudget::fold_node_ledgers`), for reporting only. Hot-path work/node
charges for default-limit budgets admit lock-free on the node meter and touch
the ledger only on failure.

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
