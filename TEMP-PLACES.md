# Temporary-rooted assignment places

Status: private bounded slice, scoped hosted acceptance. Full Rust remains open.

## Contract

Admits assignment and builtin primitive compound assignment whose place is
rooted at a reference returned by a genuine call (function, associated,
trait or method call):

- `*call() = v`, `*call() OP= v`
- `call().f = v`, `call().f OP= v` (one implicit carrier Deref)
- `(*call()).f OP= v`, constant `call().a[K] OP= v`
- runtime `call().a[i] OP= v` (compound only)

Stock rustc 1.98.1 evaluation order is frozen and preserved: value, then
carrier call, then runtime index, then the destination read (compound) and
write. Every subexpression is evaluated exactly once. RHS reads of the
borrowed root happen before the carrier's `&mut` is created
(`*p(&mut x) += x`).

## Lowering (existing IR only)

`Stmt::AssignDeref { carrier, fields, index, op, value }` in the AST lowers in
the checker to:

1. `Let $rhs = value` (via `compound_value` for `op`)
2. `Let $carrier = carrier`
3. `AssignPlace` / `AssignOp` / `AssignIndexOp` on `$carrier` with the written
   projection (`Deref` + fields).

No new IR, CFG, borrow, backend or optimizer node. The carrier is an ordinary
hidden local, so existing loan, move, initialization and cleanup analyses
apply. The hidden binding's mutability is never treated as authority: a
shared-reference carrier is rejected as E0594, while an immutable binding
holding `&mut` (`let r = &mut y; *p(r) = 2`) is accepted, as in stock.

## Boundaries

Invalid (stock error, rejected at CHECK and BUILD, including unused bodies):
E0594 shared carrier, E0596 immutable root, E0308, E0368, E0614 scalar
deref, E0609, E0616, E0277, E0502 live shared loan, constant OOB.

Legal but explicitly Unsupported: by-value temporary field assignment
(`mk().f = 2`), plain (non-compound) runtime index through a call carrier,
non-reference carriers (raw pointers, user `Deref`), operator-trait
compound assignment, `**call()` chains.

## Acceptance

- `tests/temp_places.rs`: three frozen stock sources with oracle stdout
  (`tests/fixtures/temp_places`, `SHA256SUMS`, `ORACLE`), CHECK/BUILD/run at
  workers 1 and 8. The index trap prints identical stdout; native trap exit
  101 versus stock abort 134 follows the existing runtime trap convention.
- `tests/temp_places_authority.rs`: 15 invalid and 3 legal-unsupported
  sources, CHECK and BUILD, workers 1 and 8, no executable published.
- `tests/reference_fields.rs`: the unchanged historical
  `computed_call_field_assignment` source is promoted after exact stock
  stdout.
- Scoped hosted gate: 1,749 tests in 18 binaries including
  `graph_integrity` (no baseline change) and lib unit tests, strict
  all-target clippy. No performance claim.

The parser keeps assignment tails out of the recursive block frame
(`plain_assignment_statement`, `compound_statement` are `#[inline(never)]`),
preserving the 300-deep nesting stack test.
