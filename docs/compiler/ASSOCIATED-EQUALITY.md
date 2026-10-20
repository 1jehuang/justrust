# Associated equality bounds for rigid callers

Status: bounded closed-RHS equality implemented and validated on October 17, 2026. Full Rust, unconstrained symbolic projections and general associated equality remain open.

## Bounded vertical

Admit genuine equality-constrained generic callers such as `fn read<T: Source<Out = i32>>(x: T) -> i32 { x.get() }`. The owner `T` remains universally quantified and checked, not replaced by a favorable concrete implementation. The equality RHS must already be closed and projection-free, Sized and recursively reference-free; this milestone does not normalize parameter-valued or projected equality RHSs. Support all existing admitted concrete values, including non-Copy records, tuples, arrays and wide numbers, rather than a scalar whitelist.

Use equality from the caller's actual predicates to normalize its signatures, local annotations and required trait-call results before ordinary executable shape checking. Required method source metadata remains immutable. Actual calls and closed instances must discharge the original equality against the exact selected implementation and associated assignment.

Parameter-valued RHSs, unconstrained rigid projection values, associated type bounds, projected defaults, GATs, general equality inference, trait-argument inference and reference-bearing associated normal forms remain explicit boundaries. Equality does not confer Copy unless the resulting real type has a genuine Copy proof.

## Source identity

Keep `TraitPath::Source(GenericPath)` and ordinary type arguments. Add `PathSegment.associated_types: Vec<AssociatedTypeBinding>` with `AssociatedTypeBinding { name: AstIdent, value: Type }`. The member occurrence and RHS are genuine source handles. They are not extra positional trait arguments, fabricated type parameters or string aliases. Only the final trait segment can contain these bindings.

Initially admit bindings in function and implementation type bounds, including existing supported where-bound syntax. Do not silently carry them through bare imports, nominal applications, impl trait heads or explicit UFCS trait applications. Trait-declaration/supertrait implication equality requires a real predicate implication carrier; until implemented it must report a typed boundary after validating every written handle and type, never drop the binding.

Resolve the exact trait declaration and associated member, retain original-use provenance and every binding row before deduplication, and validate all unused bindings and RHS children. Extend cloning, storage accounting, depth/inventory walkers, aliases, source replay and frozen-schema correspondence. Retain ordinary generic argument arity and existing resource limits.

## Logical equality and proof

Add a distinct `Predicate::ProjectionEq { projection: AssociatedTypeProjection, ty: Ty }`. The projection retains the full trait application, including Self and all ordered trait arguments, plus the real associated declaration ID. Source bounds publish both the membership predicate and equality predicate. Equality is not Implements with special arguments.

Raw validation checks the complete application, member owner, caller binder scope and RHS metadata before capability restrictions. A universally proved equality uses the exact current declared predicate environment. A closed equality selects the actual implementation, substitutes its authoritative assignment with the same implementation arguments, and compares the normalized result. Missing or different concrete equalities do not pass because membership alone is true.

Keep assumption-backed equality evidence distinct from assignment-backed closed normalization and private owning-impl normalization. Retain world, environment, actual facts phase, full projection, RHS and dependencies. No public proof constructor, synthetic Closed facts or reuse of a closed implementation certificate for a rigid caller.

All predicate equality, substitution, closure, memo keys, source/schema comparison, call evidence and instance validation include the full equality. Every concrete call/instance discharges the original predicates and repeats the associated normal-form domain check, including calls whose normalized signature no longer contains a projection.

## Declared-environment staging

Before `stage_associated_types` itself, and therefore before executable origin construction, form a private declared-callable context from structurally validated raw predicates and the exact callable GenericEnv. Preserve inherited implementation requirements and original row provenance. This is not a public ValidatedFacts value. Keep raw callbacks and normalized callable schemas separate.

Normalize equality-constrained callable schemas under that exact context, derive their executable shapes and source-region contracts, and publish finalized facts only after normal conformance and well-formedness validation. Do not use the empty environment for rigid equality normalization, call back into the world while holding PlanState's source mutex, or use normalized headers as the authority for raw callback comparison.

Public body-local normalization uses the finalized caller's actual facts. Construction-stage memo entries/evidence are isolated from finalized entries. Failure, cycles and resource exhaustion retain their typed classifications; no exhaustion-to-NotProven conversion or fresh unmetered budget.

## Required-call carrier evidence

For a rigid equality-constrained trait call, substitute the original required signature with the full caller application, normalize each projection using exact equality evidence, then derive a per-application required carrier view from that signature and the required declaration's original source regions. Keep the caller's real parameter carrier components. Never derive the required contract from an implementation target, invent an empty carrier set for Self, or forge an owning-impl context without an implementation.

Derive this signature's origins and carriers directly in `facts.env()`, preserving genuine caller `ParamTail` components. Use distinct equality-required application evidence, not `ProjectedMethodEvidence`, `projected_carrier_base`, an implementation environment, or a second synthetic identity substitution. Refactor only common final carrier-view validation/sealing as needed.

The view retains the original item/schema, full application, substitution and normalization evidence, source regions, caller world/environment/phase and resulting signature. Replay must recompute and compare this correspondence. Existing evaluation order, move/loan rules and closed IR/CFG/backend residual-projection guards remain unchanged.

## Repeated constraints and boundaries

Stock measurements accept repeated identical equality bounds, repeated bindings in one trait application, and unused conflicting equality bounds. Validate every occurrence before any semantic deduplication. Identical equalities may share a canonical predicate. Conflicting normal forms must not select a convenient RHS or be mislabeled as stock-invalid; the bounded implementation may explicitly reject such environments as unsupported until a proper inconsistency model is implemented.

## Acceptance

Freeze exact stock sources and native expectations before editing implementation. Preserve design-oracle originals, including earlier unsupported library calls, and use separately measured positive-check equivalents for native payload evidence. Cover closed scalar/aggregate/non-Copy/wide RHSs; genuine unused universal bodies; exact same-Self/different-trait-argument selection; wrong equality and bad body rejection; unchanged unconstrained/parameter-RHS/reference/default boundaries; repeated and conflicting constraints; source/world/environment/phase/evidence replay; and poisoned resource/cycle behavior.

Run hosted all-target check, full regressions, strict all-target lint, formatting, public authority controls and workers1/8 CHECK/BUILD plus real ELF runtime matrices under debug and release. Preserve failed and passing run receipts. No speed or full-Rust claim follows from this bounded implementation.

## Validated delivery

The frozen 46-source corpus passed debug and release: 22 runtime cases, six ordinary invalid cases, 17 stock-valid explicit boundaries and one stock-invalid explicit boundary. Each profile exercises 184 CLI cells and 184 all-body graph cells, with 44 CLI ELF executions. All source bytes remain frozen; earlier short-projection parsing boundaries are reported at their actual phase rather than attributed to equality proof.

Hosted final validation passed 2,936 tests across 142 binaries, all-target check and warning-free strict all-target lint. Release validation passed the 17 corpus/public-authority tests and 26 focused private equality tests. Thirteen public authority controls include exact-world/environment/phase replay, immutable source bindings, caller parameter carriers and poisoned original resource ledgers.

The 17 failed and passing receipts are retained in `data/runs/xps14/2026-10-17-associated-equality.jsonl`. Construction and final logical/normalization memo epochs are distinct. Original equality obligations survive normalized signatures and are discharged on actual calls and instances. Parameter-valued RHSs, unconstrained projected values, associated bounds, projected defaults and general equality inference are not delivered by this milestone.
