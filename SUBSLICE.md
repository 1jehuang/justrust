# Shared subslices

Status: shared subslices of arrays and shared slices. Full Rust remains open.

## Contract

`&s[a..b]`, `&s[a..]`, `&s[..b]`, `&s[..]`, `&s[a..=b]`, `&s[..=b]` where `s`
is a shared `&[T]`, a shared `&[T; N]`, or a local/field array (borrowed whole
and unsized). Also `s[a..b][k]` (constant or runtime k), chained ranges
`s[1..][..2]` and `s[a..b].len()`. The element type is any admitted shared
slice element.

Evaluation order matches rustc 1.98.1: the slice, then start, then end, once
each; then the checks in stock order: inclusive end overflow, end > len,
start > end. Each is a runtime trap (native exit 101; stock aborts with 134 and
a panic message, so this is trap/stdout parity, not exit parity).

## Lowering

AST `Expr::RangeIndex { base, start, end, inclusive }`. The checker
(`Checker::subslice`) types it to IR `ExprKind::Subslice { slice, start, end,
inclusive }` of the source's shared `&[T]` type. CFG `Rvalue::Subslice` carries
start (constant 0 when omitted) and an optional end (None = len). Ownership and
borrow treat it like `ArrayToSlice`: it copies the handle once and carries
exactly the source loan and origins; bounds are usize operands and no referent
storage is read. The backend computes the new descriptor
`(data + start * stride, end - start)` after the checks.

## Boundaries

Invalid: non-usize bounds (also in unused functions), `..=` without an end.
Legal but explicitly Unsupported: `&mut s[a..b]` (mutable slices are not yet
supported at all), `&str` slicing, an unborrowed `s[a..b]` value, range
indexing of a non-place array value, `usize::MAX` as a path value (separate
gap).

## Acceptance

`tests/subslice.rs` (two frozen stock fixtures, SHA256SUMS, plus every out of
range class), `tests/subslice_authority.rs`.
