# Closed associated-type projections

Status: bounded first vertical implemented and validated on October 17, 2026. Full Rust compatibility and general associated-type support remain open.

## First vertical contract

Admit real trait-associated type declarations and impl assignments, with closed qualified projections in ordinary function signatures, local annotations and type aliases. Select the exact trait application, including Self and every trait argument, and normalize before executable IR or layout. Normal forms may be any existing supported Sized, reference-free type, including wide integers, tuples and non-Copy aggregates. No scalar whitelist or invented Copy permission.

Generic impl RHS parameters are substituted using the selected closed impl head. Nested closed projections require bounded dependency normalization. Validate every unused assignment, reject missing, duplicate and extra assignments before publication, and detect direct, mutual and growing cycles without raising resource caps.

Projection-bearing required trait method signatures/default bodies and rigid generic caller projections remain explicitly unsupported in this first vertical. Implements membership alone establishes neither an associated equality nor Copy/layout/carrier facts. Projected method conformance and symbolic caller/carrier semantics are a subsequent milestone, not completed by this one. GATs, associated defaults/bounds/equalities, ambiguous short-name projections, inherited associated lookup, projection-bearing impl heads/trait actuals and reference-bearing normal forms remain closed.

## Source and identity

Use `Type::Projection(Box<SourceProjection>)`, where `SourceProjection` is `Qualified { self_ty: Type, trait_path: TraitPath, item: AstIdent }` or `SelfItem { item: AstIdent }`. Add separate associated declaration/value vectors to TraitDecl and ImplDecl. Existing method vectors remain method-only. Never fabricate a source trait path for Self::Out.

Resolver owner context distinguishes None, the retained trait source index and the retained trait-impl source index. Restore it on errors and clear it for nested free functions. Source lowering receives a private owner context derived from the retained Program and exact module, not a caller-minted owner certificate. Validate the original associated member occurrence separately from the trait GenericPath certificate.

Allocate one genuine `DeclKind::AssociatedType` per trait member, owned by its trait with its inherited generic environment. Stable identity includes the owning trait, preventing same-named members in different traits from colliding. An assignment is identified by `(impl DeclId, associated DeclId)` under the existing impl environment. No assignment binder, callable, BodyId or synthetic ParamId is created. Preserve all source rows before map insertion so duplicates cannot disappear.

## Semantic and world API

`ir::AssociatedTypeProjection { trait_ref: TraitRef, item: DeclId }` is retained by `Ty::AssociatedType(Arc<AssociatedTypeProjection>)` in raw schemas. `AssociatedTypeHeader { declaration, trait_decl, env: Arc<GenericEnv> }` is a separate HeaderSet collection. TraitHeader has associated type IDs. ImplHeader has separate `AssociatedTypeValue { implementation, associated, rhs: Ty }` rows. Method header arrays remain unchanged.

`SchemaCx::associated_type_rhs(implementation, associated)` returns the authoritative immutable raw RHS. Validate pair ownership, environment and exact header/callback agreement. `ValidatedTraitWorld::normalize_associated_type(projection, facts)` returns an opaque result with a `ty()` getter and private evidence binding world, environment index, actual facts phase, full projection, selected impl and arguments, assignment pair, raw RHS, normal form and dependencies. A closed projection inside Universal facts retains Universal phase. Assumption-only membership is not an assignment witness.

Candidate matching shares one substitution across Self and all trait arguments, requires every impl parameter resolved, and discharges instantiated predicates. Normalization uses bounded active/completed dependency state, checked allocation and shared work/depth ledgers. Resource, cycle and ambiguity failures never become successful fallback. No mutex is held through recursive world/schema callbacks.

## Staged publication

1. Allocate identities and raw-lower every signature/RHS, retaining raw source authority. No normalization under the PlanState mutex.
2. Validate all identities, owner/env metadata, complete assignment maps, impl heads, full-argument coherence, supertraits and unused RHS structure. Reject unsupported projected trait method schemas before origin derivation, after metadata validation.
3. Build declared environments and issue a private checked-head builder context. It is not public ValidatedFacts or proof evidence.
4. Normalize closed projections in ordinary callable schemas and validate unused RHS dependencies using a selector independent of method signature validation. Keep a distinct staged normalized schema map. Raw callbacks remain immutable and are compared as raw metadata, never against rewritten headers.
5. Validate normalized signature/carrier/Sized/reference policy, build origin schemas, run existing impl method conformance/WF and complete environments, then publish the immutable world.
6. Body-local lowering constructs raw types under the state lock, releases it, normalizes with original body facts and applies existing typed/carrier admission.

Every structural type visitor explicitly handles projections. Substitution rebuilds full Self/args and retains the associated item. Executable IR, CFG, closed instances and backend reject residual projections, including dead and nested metadata, rather than guessing word layout or empty carriers. Identity, semantic equality and normalization evidence remain distinct operations.

## Verified first-vertical results

- Full hosted suite: 2,851 tests across 138 binaries. The 1,128-test library suite, all-target check, strict all-target lint and formatting gates pass.
- Both debug and release run the same 40-source stock-first corpus through 160 native workers1/8 CHECK/BUILD cells. Twelve executable sources produce 24 checked ELF images per profile, including distinct trait arguments, local annotations, generic impl substitution, tuples, non-Copy records, explicit Clone/Copy records, i128, aliases and nested chains.
- The measured partition is 12 executable, 10 ordinary-invalid, 16 stock-valid explicit boundaries and 2 stock-invalid explicit boundaries. Original formatting/derive/parser boundaries remain classified as such rather than claimed as associated-type acceptance. All original 39 source bytes/order remain unchanged. An independently stock-measured manual Clone/Copy source is appended as source 40.
- Public authority controls: 22 tests. Release private filters also pass 32 associated and 34 projection controls. Raw callback mismatches, exact source occurrence, foreign facts, phase/environment replay, residual dead metadata, sticky resources and shared DAG traversal are checked without creating public proof constructors.
- The historical 62-source alias corpus changes only the `associated` outcome to supported; all source bytes/order remain unchanged.
- Actual failed and passing build receipts are retained in `data/runs/xps14/2026-10-17-associated-types.jsonl`. No performance claim is made.

## Acceptance

Stock-first exact sources cover same Self with distinct trait arguments, generic impl RHS substitution, non-Copy records, Copy aggregates, wide values, aliases, locals, nested chains, missing/extra/duplicate assignments, wrong values, unused errors and cycles. Public authority controls cover foreign worlds/owners/items/parameters, argument permutations, raw callback mismatch, actual phase/ledger replay, residual executable projections and poisoned cache hits. Run stable justrust check, full tests, strict all-target lint, debug/release native CHECK/BUILD/runtime matrices and cargo fmt. Retain real failed and passing receipts before commit/push. No speed or full associated-type compatibility claim follows from synthetic fixtures.
