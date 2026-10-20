# Mutable range borrows

Status: implemented for the frozen local-rooted scope. Native stock, invalid-input, canonical publication and unchanged graph-integrity gates pass. Full Rust remains open.

## Language contract

Admit `&mut base[start..end]` and the other five existing range forms over local/field arrays and local-rooted `&mut [T]` carriers. Admit shared range borrows from those mutable carriers without moving them. The element family is the existing reference-free slice family, including generic and non-Copy elements for descriptor transport. Existing non-Copy element replacement restrictions remain unchanged.

Evaluate the base, start and end once in source order. Preserve the existing inclusive-end overflow, end-out-of-bounds and reversed-range checks and trap/stdout contract. Construction of an empty range must not dereference an element. No null-pointer or invented static-origin shortcut is permitted.

A mutable result derives from a checked mutable reborrow of the actual source place. A shared result derives from a checked shared reborrow. The original carrier remains available only when its outstanding loans permit use. Return/output lifetimes remain bounded by the original borrow and source storage. Mutable parameters and by-value inputs retain ordinary move semantics.

## Alias contract

Simultaneously live overlapping mutable ranges are rejected, as are conflicting shared/mutable accesses and escaping references to local arrays. Range syntax alone is not a proof of disjointness. Conservatively retaining the source loan is allowed where distinct range disjointness has not been proved. Do not change BorrowTarget or ProjectionTail correlation semantics without coordinating their owners. Do not use raw constructor origins to bypass safe borrow checks.

## IR and CFG shape

Reuse typed `Subslice` and its descriptor calculation only after creating an ordinary checked reborrow. Source/result slice borrow kinds and element identities must match. Canonical source occurrence, original span, base and exact range-bound identities must survive source replay and IR/CFG validation. A forged shared-to-mutable conversion, replaced operand or foreign origin cannot authorize a result. Transfer preserves the issued reborrow origin and exact source lifetime dependencies through the range descriptor.

All outputs remain immutable typed graph facts. No new whole-program node, shared mutable state, global budget, side-channel result or weakened ratchet is permitted. CHECK must not request code generation.

## Acceptance gates

- Frozen stock rustc 1.98.1 edition-2024 fixtures with SHA256SUMS, native CHECK/BUILD/run workers 1 and 8: all six range kinds, nested/chained ranges, arrays/fields, generic/non-Copy descriptor transport, empty ranges, repeated reborrows, shared views, writes, output lifetimes and exact evaluation order.
- Invalid source at workers 1 and 8: shared source to mutable result, overlapping mutable results, shared/mutable overlap, conflicting source use, use after move, escaping local arrays, ambiguous return lifetimes, invalid bound types even in unused functions.
- Trap/stdout parity for inclusive overflow, reversed bounds and end beyond length. Do not claim exit-code parity with stock abort.
- Forged source/IR/CFG cases: wrong borrow kind, element identity, foreign origin, altered base/start/end/inclusive marker, and missing checked reborrow.
- Graph-shape tests require actual per-function reborrow/range facts and CHECK without codegen. Run unchanged graph_integrity before publishing compiler changes.

## Files and ownership

This stage owns slice-specific checker helpers and validation hooks. Coordinate source replay changes with inference ownership and BorrowTarget/ProjectionTail changes with their active owners. Vec wrappers, canonical NonNull alignment and range Index implementations remain separate library work.

## Evidence and validation boundary

Native six-range transport passed workers 1/8 CHECK/BUILD/run in `20261019-105759252-3414881`. Alias, dangling, move, shared-strengthening and invalid-bound rejection passed in `20261019-105906051-3427230`. Bound evaluation order, reversed/out-of-range/inclusive-overflow trap prefixes, per-function CHECK shape and all 14 unchanged graph-integrity tests passed in `20261019-110444932-3474496`.

Exact source/operand identity is enforced at canonical `publish_callable_body` and `lower_source_closed` publication, not by arbitrary unsealed structural IR/templates. The canonical regression rejects swapped bounds, changed inclusion and removed source origins. Initial tests aimed at unsealed structural templates tested the wrong boundary and were corrected without adding authority metadata or changing production validation. The ordinary checked borrow retains conservative whole-source alias authority.
