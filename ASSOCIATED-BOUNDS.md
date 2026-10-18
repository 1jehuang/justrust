# Associated declaration bounds: bounded next milestone

Status: delivered as a bounded milestone, verified on 2026-10-18. Builds on `458253eb5` and PARAMETER-EQUALITY.md. Full Rust remains open.

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

## Verified delivery

The 40-source corpus retains all 29 originals and 11 separately measured append controls. The append receipt is `associated-bounds-stock-append/receipt.json`, SHA256 `e14960c7ce019ef53084fd257803f9627cbba99f623d2d7544b5bfb87d92b268`. Concatenated source bytes in case order hash to `b56b309099f0686cc337d0590dd7f5579c6b34a0c6ca0a47a4f92247234495c5`. Native outcomes are eight Runtime, 14 Invalid and 18 explicit earlier boundaries, not 40 semantic successes. Debug and release each pass 160 CLI and 160 graph cells at workers 1/8, including 16 real payload executions. Every earlier parser/resolver/inference boundary is asserted separately. Bare U duplication still fails without explicit U:Copy.

Final hosted validation: all-target check `20261017-174250756-3911775`; full 2,997 tests across 146 binaries `20261017-174252477-3912192`; warning-free Clippy `20261017-174317713-3913996`. Release corpus/public authority passes 13 tests in `20261017-174256867-3912493`, and release associated proof controls pass 57 in `20261017-174318086-3912492`. Formatting and diff checks pass. Nineteen failed and successful root run summaries are retained in `data/runs/xps14/2026-10-18-associated-bounds.jsonl`. Historical source bytes remain unchanged; only observed earlier-phase expectations were updated.
