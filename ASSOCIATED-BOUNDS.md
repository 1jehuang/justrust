# Associated declaration bounds: bounded next milestone

Status: frozen implementation contract, not delivered. Builds on `458253eb5` and PARAMETER-EQUALITY.md. Full Rust remains open.

## Scope

Admit associated declarations with the actual catalog `Copy` and `Sized` bounds, including repeated bounds and existing import/qualified paths. Keep real source occurrences, not spelling or guessed layout. All associated assignments, including unused ones, must satisfy the written bounds under their actual implementation environment. Closed calls and the existing equality-normalized paths retain these obligations after substitution.

This slice does not admit user-trait associated bounds, lifetimes, `?Sized`, associated defaults, GATs or general unconstrained rigid projection values. Unsupported forms must still validate every representable source handle before their capability error. An unrepresentable parser form remains an explicitly earlier syntax boundary.

## Stock compatibility constraint

The measured stock compiler is rustc 1.98.1, edition 2024, panic abort, without RUSTC_BOOTSTRAP. In the frozen receipt, `type Out: Copy` does **not** make the separate equality-bound parameter U implicitly Copy in `dup<T: Source<Out = U>, U>`. Duplicating U still needs its own genuine Copy evidence. Direct projection values can use the associated Copy bound in stock, but their unresolved executable representation is a separate future milestone here. Do not add U:Copy during predicate closure, infer U from an implementation, erase ParamTail, or grant Copy based on a name or byte layout.

Receipt: `associated-bounds-stock/results.json`, SHA256 `9b4a51ddbd5a7773e6d1f5992bb33083f1bd90ebb40c6362855542fb6a094355`. It retains 18 originals, eight separate controls and three supplemental witnesses, including wrong/missing bounds, unused invalid assignments, same-Self/different arguments, actual tuple/wide payloads, contradictory unused constraints and a user trait named Copy. These are stock-only observations until native gates pass.

## Source and semantic representation

Extend `AssociatedTypeDecl` with `bounds: Vec<TraitPath>`. Keep every ordered source row and member occurrence through parse, resolution, cloning, storage/depth/inventory scans and immutable source replay. Associated bounds must retain original Source(GenericPath) provenance even when they resolve to catalog traits; do not drop occurrence authority by converting them to a bare builtin enum.

Extend `AssociatedTypeHeader` with `bounds: Arc<[DeclId]>`, containing exact resolved catalog Copy/Sized declarations. Add an authoritative source/schema callback for the bound IDs and require header/callback agreement. Empty bounds keep the old implicit Sized rule and old compatibility behavior. Foreign, wrong-kind and noncatalog bound IDs cannot confer authority. Validate all IDs before any Unsupported result and preserve duplicate source validation before canonicalization.

## Proof and construction

Validate associated header identity, ownership, environment and all bound IDs before semantic use. After declared implementation environments exist, use the existing private owner normalization context to normalize every assignment and require its associated bounds under that implementation's real predicates. Do not insert the implementation's own head or assignment as an assumption to prove its bound. A generic RHS T must have genuine declared T:Copy evidence if Copy is required.

Repeat the exact bound requirement after closed selection/substitution using the same selected implementation arguments. Keep construction and finalized memo/evidence namespaces separate. Do not call world normalization while holding the source PlanState lock. Do not change general predicate closure to manufacture Copy for equality RHS parameters.

No new executable projection, carrier mapping or ABI rule is introduced. Existing exact reference-free associated normal-form validation, source-region transport, result-domain certificates, move/Copy checks and residual guards remain mandatory.

## Acceptance

Preserve frozen stock sources and add separately measured literal payload controls if earlier parser boundaries mask them. Native tests must cover valid scalar/tuple/wide assignments and calls, generic RHS with explicit Copy, non-Copy and missing-Copy assignments rejected even unused, wrong argument/member/world/source replay, catalog identity rather than user-trait spelling, duplicate bounds, and unchanged no-bound declarations. Bare U duplication without U:Copy remains ordinary invalid. Unconstrained rigid projected values remain an explicit boundary.

Run hosted all-target check, full regressions, strict lint and formatting; debug/release workers1/8 CHECK/BUILD and real native payloads; public metadata/proof tests and original resource guards. Retain both failed and passing receipts. Report declaration-bound support only, not full associated types, general bound elaboration, a speedup or full Rust.
