# Fixed-array irrefutable pattern contract

Status: **implementation in progress, not accepted**. Full Rust remains open.

## Honest source and type authority

Add `Pattern::Array(Vec<Pattern>)` and `Pattern::ArrayRest(PositionalRest)`.
Preserve only actual written children and the explicit prefix/suffix around one
syntactic rest. No tuple/Field fiction, synthesized omitted wildcard or binder is
permitted. Existing tuple and enum pattern representations remain unchanged.

Admit existing nonempty fixed arrays in irrefutable let and match patterns.
Require exact actual `Ty::Array` length and element type under existing limits.
No-rest patterns require exact length; rest maps actual prefix/suffix children
through the borrowed checked positional projection. A pattern cannot invent an
unknown array length or element type. Empty arrays, slices, reference ergonomics,
refutable child leaves, guards/or-patterns, parameter patterns and tail-at-rest
bindings remain explicit Unsupported in this slice.

Source walkers, scoped binders, universal owner/proof validation, clone preflight,
canonical inventory and replay visit actual children in written order. Both rest
vectors are resource-admitted, with actual allocation capacity retained. Source
identities and existing structural inventory child paths remain stable.

## Copy-only selected elements

Every selected binding whose checked projection crosses an array element must
have actual Copy permission from validated facts. Valid Rust non-Copy array
extraction remains an explicit capability boundary, not an ordinary type error.
Bindingless patterns on non-Copy arrays may inspect without consuming elements.
Nested selected Copy fields may be admitted only through exact typed paths.

Keep the existing independent CFG and ownership prohibition on non-Copy moves
through ordinary ConstantIndex paths, including dead blocks. Do not weaken that
fence to implement pattern extraction. Future non-Copy array destructuring needs
separate checked extraction authority. Existing conservative indexed borrows
remain unchanged and no disjoint-reference or partial-move claim follows.

## Checked plans and gates

Array match plans use genuine Aggregate domains extended to nonempty fixed
arrays, with selected paths containing only Field and ConstantIndex. Existing
projection typing must prove each step's actual domain, bounds, destination type,
local freshness and nonoverlap. Never admit Deref, Downcast or dynamic indices as
aggregate paths. Existing array layout and backend index addressing remain the
private transport, with no new ABI or public layout claim.

Evaluate the initializer/scrutinee exactly once. Omitted elements create no
binding, read or move. Preserve count, depth, work, storage and array-length caps.
Require stock-first exact sources, native workers1/8 CHECK/BUILD without compiler
fallback, exact executable outputs, bootstrap debug/release, public owner/type/
index/Copy-boundary controls, full hosted tests, strict lint, formatting and diff
gates. Retain exact failed/passing receipts. This is not full Rust support or a
performance claim.
