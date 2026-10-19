# Construction-final declared equality reductions

## Status and frozen contract

Bounded private composition on `fd2712074`. Declared equality reductions are
construction dependencies, not implementation assignments or executable evidence.
The only producer is `normalize_declared_type`, reached by
`normalize_declared_callable_schema` from `stage_associated_types` inside mutable
`ValidatedTraitWorld::build`. All writes finish before final context replay and
before `Arc<ValidatedTraitWorld>` publication. Published contexts contain a plain
Vec and immutable storage receipt, with no reductions mutex or ledger backreference.

No node kind, cache, global key, index, registry, or new whole-program traversal is
introduced. The existing declaration index chooses the existing context row.
Duplicate reductions and repeated declaration-header contributions retain their
original encounter order. The normalized callable schema is returned with its
private local reductions, which move into the destination only after successful
normalization. Empty destinations steal the paid allocation without copying.

## API and producer ownership

`normalize_declared_callable_schema` returns private
`DeclaredCallableNormalization { schema, world, stage, declaration, reductions }`.
`normalize_declared_type` receives an explicit local reduction sink. The staging
caller resolves the destination through the existing `declared_callable_index`
and consumes the result through `finish`. Result and row world/stage/declaration
must match that destination. No result constructor or reduction fields are public.

`ReductionRows` contains only `Vec<DeclaredEqualityReduction>` and optional
`RetainedStorage`. Each reduction retains the existing world ID, private stage ID,
declaring DeclId, immutable projection and RHS Ty. No Program, NameStore, registry,
source handle decoder, PlanBudget or PlanReservation survives inside this owner.
The task-local caller supplies the existing budget for every operation.

Before growth, the complete replacement capacity is reserved and allocated with
fallible Vec allocation. Public actual capacity and payer authority are checked,
then retained admission and moved-row work are paid before moving any old row.
Old, incoming and replacement allocations remain admitted together. Capacity or
work failure leaves the destination pointer/capacity/receipt unchanged. Sticky
budget failure rejects empty/nonempty paths, and a foreign same-policy payer is
not accepted. Allocator surplus beyond the prepaid capacity fails closed rather
than being silently admitted. Receipts are immutable and dropping them does not
refund admission. Old and failed replacement admissions remain conservatively
charged until ledger end: transactions preserve data, not refund-neutral usage.
No exact minimal peak claim is made.

Projection/RHS clones in a reduction copy inline metadata and immutable Arc
children. Existing normalization remains responsible for those shared payloads.
The new exact container admission is not proof of semantic equality or source
origin, and it does not refund existing normalization charges.

## Authority and acceptance

The existing final predicate membership, world/stage/declaration and normalized
RHS replay is retained. Binder provenance remains checked by
`validate_final_equality_context` at evidence replay, not by pretending the
construction dependency comparison already authenticated every private context
field. A failed or finalized normalization cannot reopen published storage.

Eight focused tests cover ordered duplicate append, first-allocation move, actual
capacity/overlap receipts, underpaid replacement recovery, sticky growth/work
failure, foreign payer, source-backed stale/foreign/owner/RHS/membership/binder
rejection, private-result atomicity and immutable repeated publication reads.
Existing unused-invalid equality and forged-evidence suites remain acceptance
gates. These are not claims of arbitrary hostile private-builder admission.

The unchanged graph scanner first rejected the stale debt inventory with exactly
10 removed `(root, DeclaredCallableContext, Mutex)` entries and lexical equality
mutex tokens 2 to 0, receipt `20261018-232702344-2934699`. Only those inventory
entries are tightened. Required1208/1209, work growth, resource limits, dependency
ceilings and all other debt remain unchanged. No speedup claim follows.

## Remaining source/nominal audit, design only

`PlanState` still owns ambient `source_context`, raw nominal `schemas`, live
`nominals { applications, ordered, active }`, final `contexts`, and mixed live
`reservations`. Callable contexts are constructor-written and immutable in use,
but moving them would not remove the PlanCtx mutex carrier. No further table is
moved by this milestone.

`SchemaAdapter` still retains Program, catalog, DeclRegistry, PlanState,
SourceSchemas and budget. Removing its direct Program field would not sever
source/budget ownership: DeclRegistry owns Program, NameStore, ModuleArena and
PathKeys. `instantiate_schema` still uses raw_plan substitution; validate_nominal
still authenticates exact canonical def pointers through `nominals.ordered`.

Nominal source mint/seal is a hard prerequisite for removing that path. Current
mint validates declaration and argument authority before cache hits, uses active
by-value cycle detection and distinguishes structural transport. NominalAppId is
still ordered-position based, and SubstitutionCx requires original-application
authenticated decoding. Canonical key bytes, hashes, numeric registry shortcuts,
or copied raw StructDecls without checked source-owner decoding cannot replace
that authority. A future producer-owned immutable seal needs exact declaration,
kind, binder, module/visibility, member identities and typed template/dependency
authority without Program/NameStore/budget backreferences. No such seal is activated.

Ambient source_context could later become an explicit private scalar argument
through source_ty/with_transport, source_trait_application, metadata validation,
append_source_bounds and recursive mint calls. That removes a hidden authorization
slot but does not by itself remove the mutex or establish nominal authority.
World proof/projection memos, PlanCtx, nominal interner and budget debt remain.
