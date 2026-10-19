# Immutable semantic graph facts

Status: migration in progress, 2026-10-19. This does not certify the compiler's
reachable fact graph as immutable. `GRAPH-INTEGRITY.md` remains authoritative.

## Reachable mutation inventory

The chicken audit's failures are real even when the lexical gate passes:

- `SIGNATURES` publishes `PlanCtx`, whose `state: Arc<Mutex<PlanState>>` is also
  retained by `SchemaAdapter` through `ValidatedTraitWorld.schema`.
- `PlanState` contains declaration schemas, callable contexts, source-region
  metadata, associated assignments, temporary source context, nominal application
  interning tables and retained reservations. Body checking changes the nominal
  tables and temporary source context after publication.
- `ValidatedTraitWorld.memo` and `projection_memo` formerly shared proof results,
  errors and active states across readers. These are now invocation-owned
  `ProofScratch` and `NormalizationStack` data, not world fields.
- `DeclaredCallableContext.reductions` formerly concealed another mutex inside
  the world. Reductions are now returned from normalization as ordinary local
  values and installed through exclusive `&mut ValidatedTraitWorld` access
  before publication. Final queries only read them.
- `PlanBudget` still has shared resource/failure bookkeeping. In particular the
  old shared 4096-obligation limit made independent readers depend on world-wide
  cache reuse. Graph execution now admits obligations through each node's engine
  meter with the same 4096 limit. Direct non-graph API calls retain ledger limits.
  Aggregate obligation telemetry includes those per-node counters. Obligations
  count recursive proof-search misses, not terminal rule decisions or exact
  declared Implements assumptions. Assumptions still require the exact predicate
  including binder identity in the admitted caller context. Projection equality
  never takes that shortcut. Fixed canonical leaf decisions use the
  primitive/reference/raw-pointer Sized/Copy/Clone rules, which do not search impls or
  recurse. They still admit full metadata, reject active ancestry, consider exact
  assumptions, meter work and evidence, and mint context-specific evidence. In
  particular `&mut T` is not Copy/Clone. Parameters, projections, aggregates and
  user traits do not take this leaf path. This is not a query-count metric.

No lock is hidden behind an alias and no placeholder graph output replaces a
semantic value. No whole-program kind or ratchet ceiling is added or raised.

## Proof scratch invariants

1. Every public proof/normalization operation starts fresh local scratch, or
   receives an explicitly borrowed invocation-local `ProofSession`. The instance
   planner owns one world-bound session for its synchronous admission loop so
   nested recursive instance prefixes reuse completed exact-context proofs.
   Scratch never belongs to a published world, facts object or static.
2. The complete memo key retains the exact predicate environment index, facts
   phase, owner-finalization stage and predicate/projection. Projection keys also
   retain staged status. A result for a different assumption context cannot hit.
3. An active DFS ancestor is rejected before completed memo lookup. Concurrent
   computations cannot introduce a false cycle or borrow another invocation's
   successful proof, failure, or resource allowance.
4. Completed local memo entries may retain immutable evidence. Evidence leaving
   the computation is included in the ordinary immutable output/proof object and
   retains the existing replay checks. The mutable memo container does not escape.
5. Declared equality reductions are complete before the world is published.
   Normalization queries cannot append dependencies afterward. Final replay
   still verifies world, stage, declaration, exact projection and finalized RHS.
6. Unused source bodies retain ordinary TEMPLATE_BODY and universal validation
   nodes. Memo locality must never turn demand of an executable root into
   permission to skip a malformed unused generic body.

## Why copying PlanState is not a migration

`NominalAppId` currently consists of a snapshot and interner position.
`StructTy`/`EnumTy` equality and hashing use this ID. `SubstitutionCx` resolves
bare IDs through `PlanState.nominals.ordered`; downcast projections also retain
bare IDs. `SchemaAdapter::validate_nominal` requires the exact canonical Arc
pointer at that interner position. Separate copies can assign the same ID to
different types, reject valid types produced by another task, or accidentally
accept forged metadata. Moving the mutex into a helper is the same violation.

## Next identity boundary and acceptance criteria

The remaining migration must make nominal application identity self-contained:

- Immutable declaration authority, source schemas and callable contexts remain
  graph facts read through the owning node's engine handle.
- An application carries a sealed immutable authority tying source declaration,
  exact arguments, source snapshot and validated definition together. A public
  or forged definition with matching numeric IDs does not acquire that seal.
- Semantic identity is the complete declaration-and-argument key under the exact
  source authority, never an allocation-order index or a collision-prone hash.
- Per-task nominal construction may memoize locally, but its immutable result
  and complete application authority must travel in typed body/application
  outputs. Later substitution must not consult the producer's scratch table.
- Bare-ID consumers, including nominal carrier mappings and enum downcasts,
  must be migrated together. Local integer labels, if retained for diagnostics,
  must not substitute for semantic authority or a graph dependency.
- A non-Copy application handle affects VariantId/Projection clone sites. Those
  changes require coordination with the ownership and DynamicIndex lanes. The
  ownership implementation and its size gate are not owned by this lane.

Before removing PlanCtx's lock, adversarial tests must distinguish independent
source sessions, argument order, equal-looking declarations, replay under wrong
facts, forged/copied definitions, nested substitutions, enum downcasts and
parallel readers. Real programs and invalid unused bodies must retain their
recorded graph dependencies under workers 1 and 8. A lexical ratchet shrink
alone is insufficient evidence.
