# Nested runtime array reads and writes

Status: bounded slice, full local suite acceptance. Full Rust remains open.

## Contract

Reads through nested runtime array indexes rooted at a local, with fields
allowed along the path and no dereference: `m[i][j]`, `t[i][j][k]`,
`m[i][0]`, `g.cells[i][j]`. Each runtime index level is evaluated and
bounds-checked outermost first, matching rustc 1.98.1: an out-of-bounds outer
index traps before the inner index expression is evaluated, and an
out-of-bounds inner index traps after both are evaluated.

Rows are addressed in place (`Projection::DynamicIndex(index_local)`), never
copied out, so `m[i][j]` reads one element. A runtime array row is admitted as
a place step only when it is indexed again. A runtime element that is not
indexed again (`a[i].0`, `a[i]` whole) keeps the existing whole-value read.

## Lowering (existing IR only)

`cfg::pattern_source` walks the checked expression chain. For each interior
runtime row it materializes that level's index into its usize expression local,
emits `IndexInspect { array: prefix, index }` (the runtime bounds check, in
source order), and appends `Projection::DynamicIndex(local)`. The backend
`place_address` performs a per-level bounds check for every `DynamicIndex`
projection. `ir::projected_ty` admits an array runtime element only when the
next projection indexes it again; whole-row runtime loans stay scalar/unit
only. Ownership/borrow analyses already treat a runtime element's footprint as
the whole array. No new IR, CFG statement or backend operation.

## Boundaries

Invalid (stock error, rejected at CHECK and BUILD, including unused bodies):
non-usize index (E0277), uninitialized index (E0381), type mismatch in an
unused function (E0308), constant out-of-bounds under a runtime row (stock
deny lint `unconditional_panic`).

Legal but explicitly Unsupported: indexing through a reference to an array
(`r[i][0]` with `r: &[[T; N]; M]`, now classified Unsupported instead of
Invalid), and non-scalar leaves (`m[i][j].x` with struct elements).

## Writes

`m[i][j] = v` and `m[i][j] OP= v` (any depth, constant indexes and fields along
the path, rooted at a mutable local) with scalar/unit elements. Stock order is
value, then each runtime index outermost first with its bounds check before the
next index is evaluated, then the final index and the write. The parser routes
such targets to `Stmt::AssignNestedIndex { target, op, value }`; the checker
lowers it to existing IR: `Let $v = value`, then per interior row
`Let $iN = index` plus an ignored runtime row read (lowered to `IndexInspect`,
the bounds probe), then `AssignIndex`/`AssignIndexOp` (or `AssignPlace`/
`AssignOp` when the last step is constant) on a place with
`DynamicIndex($iN)` steps. `ir::indexed_array_ty` types the array operand of
runtime index statements, where a trailing runtime row is interior. borrow.rs
treats a runtime-row index write as carrier-free (scalar elements only).

Invalid write controls: immutable root (also in an unused function),
value type mismatch, non-usize index, float into int compound, live shared
loan (E0506), constant out-of-bounds under a runtime row. Legal but
Unsupported: writes through `&mut` (`r[i][0] = v`), non-scalar elements, and
field access after a runtime index (`m[i].0 = v`).

## Acceptance

- `tests/nested_index.rs`: four frozen stock sources (`tests/fixtures/nested_index`,
  `SHA256SUMS`, `ORACLE` rustc 1.98.1 48a229cea) at CHECK/BUILD/run, workers
  1 and 8. Both trap fixtures give trap/stdout parity only (native 101 versus
  stock abort 134), not exit-status parity.
- `tests/nested_index_authority.rs`: four invalid and two legal-unsupported
  sources, CHECK and BUILD, workers 1 and 8, no executable published.
- `typed_tests::usize_does_not_convert_i32_or_erase_runtime_index_pattern_boundaries`:
  the unchanged stock-valid `a[i][0]` source is promoted from rejected to
  accepted.
- `wide_integers` tuple-array field reads after a runtime index are unchanged
  (a regression found and fixed during this slice).
- Full local suite 20261019-021057859-1309484: 3665 tests, 204 binaries,
  including stock-oracle tests. Hosted strict clippy 20261019-021544885-1523175.
  No performance claim.
