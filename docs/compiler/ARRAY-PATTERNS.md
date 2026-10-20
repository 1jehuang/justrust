# Fixed-array irrefutable pattern contract

Status: **accepted bounded native slice, October 17, 2026**. Full Rust remains open.

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

## Accepted evidence

`tests/array_patterns.rs` preserves 27 stock-first sources: 22 stock-valid and
five stock-invalid cases. Native classification is 13 runtime positives, five
ordinary invalid cases and nine stock-valid Unsupported boundaries. Workers 1
and 8 each run CHECK and BUILD, giving 108 native cells and 26 executed ELF images
per bootstrap profile. Debug and release both pass without compiler fallback.
Typed controls inspect genuine ConstantIndex paths, nested Field paths, actual
Copy permissions and captured macro binders, not just rendered diagnostics.

`tests/array_pattern_authority.rs` has 33 public controls covering exact domains,
per-step typing/bounds, nested overlaps, local freshness, dead-arm validation,
actual source owners and the independent non-Copy move fence. The non-Copy
control retains the exact canonical source signature and facts: structural IR
validation succeeds, while validation of the lowered CFG rejects the indexed
non-Copy move. Lowering alone is not the CFG validation authority.

The original array-pattern source in `tests/arrays.rs` is retained verbatim and
promoted to a real runtime positive. Two existing aggregate-IR witnesses are
promoted with their exact domain/projection metadata unchanged. Deref remains
rejected. No old source corpus or limit is weakened.

Initial debug/release authority gates exposed a genuine Never-scrutinee CFG bug:
an unreachable Unit arm result was assigned to a Never match destination. The
repair guards only that result write with the existing scrutinee completion
flag. Every arm body is still lowered and validated. The unchanged public Never
witness and new tuple/array private controls pass, including malformed dead-arm
projection rejection. Later authority-fixture failures were corrected to use an
exact canonical signature and observe the actual CFG validation boundary, not by
widening production permission or weakening a limit.

Sixteen exact redacted receipts, including these failures, are retained in
`data/runs/xps14/2026-10-17-array-patterns.jsonl`:

- Final all-target check: `20261017-114711011-1959474`.
- Debug public matrix/authority/regressions: 95 tests,
  `20261017-114713525-1959812`.
- Release public matrix/authority/regressions: 95 tests,
  `20261017-114717074-1959946`.
- Complete hosted regression: 2,400 tests across 120 binaries, zero failed,
  ignored or filtered, `20261017-114732071-1960927`.
- Strict all-target lint: `20261017-114745204-1961921`; formatting and diff checks
  also pass. The full run's 12.30-second wide-integer oracle is a diagnostic,
  not an array performance measurement.
