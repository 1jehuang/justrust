# Immutable source schemas (bounded private milestone)

## Status and frozen contract

Accepted bounded composition: frozen `7598bfa48` passed the complete 3,502-test
suite and forced-hosted strict lint. Central receipts are in
`data/runs/source-schemas/CENTRAL-ACCEPTANCE.md`. This is not full immutable-world
activation. Five
construction-final tables move once from the local builder into `SourceSchemas`:
raw callable schemas, callable region-mode projections, original source region
environments, associated assignment rows, and first raw assignment RHS values.
The seal occurs after callable positions and receiver/owner metadata finalize,
before the remaining `PlanState` mutex is constructed. Const declaration slots
remain noncallable and function positions retain their original order.

Existing graph-output reads and node identities are unchanged. No new whole-program
kind, mutable callback cache, numeric admission registry, or graph-gate exemption
is introduced. `PlanCtx`, nominal interning, source context and other mutable-world
debt remain. Wide SchemaAdapter/PlanCtx activation is held.

## Storage and work ownership

Declaration slots are bounded by the authoritative inventory, with full snapshot
and DeclId equality on hits. Independent fallible Vec replacements prepay capacity
before allocation, reconcile actual public capacity, and retain immutable payer
receipts before moving old entries. Failed replacement leaves the original
pointer, capacity and receipt untouched. Old plus new allocations are conservatively
charged simultaneously. No HashMap bucket-layout estimate is used.

`StoredSchema` owns an exact capacity receipt for its signature parameter Vec.
Construction and callback copies reserve before allocating and charge copied
entries. Existing broad construction charges remain, without refunds or mixed
nominal reservation transfer. Signature results and Ty wrappers clone immutable
Arcs or inline scalars. Generic environments, predicates and reference contracts
are shared immutable producer-owned payloads, not deep copies. This does not
claim to repair all upstream allocation ownership. Arc wrapper/empty-slice
accounting follows the explicit bootstrap two-counter aligned-header policy,
not allocator instrumentation or a portable standard-library layout guarantee.

The old region-mode value is replaced, not supplemented, by a compact private
SchemaDecl projection derived from the exact final registry entry. Its hit path
physically replaces three operations (registry lookup, visibility copy, mode
lookup) with two (immutable slot lookup and allocation-free two-Arc/inline copy).
Missing slots retain authoritative registry fallback, including Const and required
trait items. Sticky budget failure is checked before hit and miss paths.

## Authority boundary

Only the original frontend constructor builds production payloads. Production
`seal_from_registry` verifies projections against final registry fields and Arc
identities, callable kind/env ownership, slot identities, payer and capacity
receipts. Region-contract mode itself remains the trusted original source/catalog
producer argument, not registry-authenticated metadata. Source callables retain
`source_region_mode` over original regions, parameters, result and free-callable
classification; catalog entries retain their existing mode. Required trait items
retain Positioned fallback. Same-ID negatives exercise identity/kind/owner/module/
visibility/receiver/binder/env, not hostile mode substitution. Returned plain
GenericSchema copies do not carry their temporary params receipt, whose drop does
not refund the caller ledger admission. Unchecked `seal` is module-private and used by privileged storage tests.
Sibling modules cannot inject projection values or call unchecked seal.

Raw associated RHS sorting/first-row semantics and normalized region semantics
remain trusted existing constructor-producer invariants. Container receipts are
not proofs of those semantic payloads. This milestone does not claim arbitrary
forged payload admission, target admission, or a new canonical nominal identity.
Weak source/name-owner tests establish those owners are not retained, not ledger
or complete transitive-world destruction.

## Acceptance evidence

Exact tested base: `af5c06a69`, not an inferred later main full-suite result.
Hosted focused tests: 17/17, receipt `20261018-230044792-2378858`.
Unchanged graph gates: 7/7, receipt `20261018-230012381-2377184`.
Strict all-target lint: clean, receipt `20261018-230048854-2379138`.

The intermediate correctly metered extraction was red at required1208:
`20261018-224159102-2291608`, Work requested665, used31999645, limit32000000,
ANALYZE key109[1209], ownership.rs:3403. That result is preserved, not relabeled
green. The subsequent projection removes actual duplicate lookup work; no budget,
required-function floor, growth ratio, ownership charge or graph debt ceiling was
changed. No speedup or complete Rust compatibility claim follows.
