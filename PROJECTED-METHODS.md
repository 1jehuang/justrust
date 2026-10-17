# Projected required methods

Status: bounded next-stage design frozen; implementation and native acceptance pending. Full Rust and general associated-type support remain open.

## Admitted vertical

Extend ASSOCIATED-TYPES.md to value-oriented required trait method signatures containing associated projections, their exact owning-impl conformance and bodies, and closed method/UFCS calls. Admit existing Sized reference-free associated normal forms, including non-Copy records, tuples, wide scalars and generic impl RHS parameters. Do not implement a scalar whitelist or invent Copy permission.

The first measured core is stock sources 01 through 06 in the projected-method stock corpus: the existing associated-only baseline, a required get returning Self::Out, distinct applications of one trait on one Self, a moved non-Copy result, tuple/i128 results and a generic Wrap<T> implementation. Every implementation body is checked universally under its actual impl binders, even when unused.

Arbitrary rigid caller projections, projected defaults, explicit associated bounds/equalities, GATs and reference-bearing projected method forms remain separate, explicit boundaries. They are not solved by a favorable closed callsite. The measured generic-forward, Copy/Sized-bound, default and reference witnesses remain unchanged for later admission. Existing nonprojected trait methods keep their current contracts.

## Separate authority paths

A private owning-impl normalization context may be issued only after complete metadata, assignment-map and coherence checks. It binds the current world/stage, implementation DeclId, exact full impl TraitRef, real impl GenericEnv and original assignment pair/RHS. It is not public ValidatedFacts, an Implements proof or closed normalization evidence.

Substitute all required trait binders through the complete owner application before reduction. Reduce only a projection whose item owner and complete instantiated TraitRef match that exact impl application. Substitute the authoritative RHS with the same checked impl mapping. A remaining T is the original impl ParamId with its existing Sized, Copy and carrier rules, not an opaque fabricated parameter. Nested reductions must retain bounded cycle/dependency checks. Other unresolved projections stay unsupported.

Use this context for required-versus-provided signature comparison and owning-impl callable signature/body annotations. It must not authorize projections from another impl, merely same Self, a reordered argument list, a foreign world or a different associated item. Keep raw source/header callbacks immutable and verify their correspondence before normalization.

Closed calls use normal implementation selection and finalized-world normalization evidence. Assumption-only membership is not an assignment. Required signature normalization, direct target selection and resulting signature/carrier evidence must refer to the same exact trait application. Never borrow the implementation target's signature or topology as the required trait declaration's authority.

## Required signature and carrier staging

Raw projected required signatures are nonexecuting schemas. Do not derive a global empty origin or carrier schema for them. Retain their original declaration, source regions, full schema and member identity, and defer executable shape derivation until exact owner substitution or closed normalization.

Derive required topology from the normalized required signature and its original source-region contract. Validate reference/carrier refinement against the implementation only after both sides have independently established required and provided contracts. Generic impl results retain real parameter-carrier behavior. No projection becomes an empty carrier, ParamTail, Copy value or ABI word merely because it is syntactically associated.

Closed projected calls need a context-bound per-application required carrier view. It must retain normalization/provenance evidence and be checked against the exact original method, full TraitRef, caller facts and result signature. Ordinary declaration-only carrier lookup is insufficient for projected methods. Existing call/evaluation/move ordering and closed IR/CFG/backend residual guards remain unchanged.

## Source, resources and acceptance

Admit required method source projections without admitting projected default bodies or general short-name inference. Preserve exact source occurrence and owner certificates. PlanState lowering remains structural under its mutex; normalization and schema callbacks execute only after releasing that mutex.

All metadata, including dead and unused method bodies and every supplied type argument, is validated before capability rejection. Use existing work/depth/bytes limits, actual-capacity accounting and poisoned failure behavior. No cap increase, fake declaration, fake body, public proof constructor or new external compiler delegation.

Preserve all 18 measured stock source bytes/order. Stock rustc 1.98.1, edition2024 and panic=abort accepts 12 and rejects 6; every accepted program prints exact ok followed by LF after semantic payload checks. The stock result is not native acceptance. Gate exact native workers1/8 CHECK/BUILD and runtime results in debug/release, original authority/correspondence controls, full tests, all-target check, strict lint and formatting. Commit actual failure and passing receipts. Never report this stage as general associated types, Iterator or full Rust.
