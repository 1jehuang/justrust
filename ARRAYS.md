# Fixed arrays and literal indexing

Status: implemented bounded native slice, validated by 591 hosted tests and strict all-target lint.

The shared maximum literal length is 4096. Independent depth (128), expanded
move-path (131072), preflight-work and one-MiB backend-frame limits still apply.

This bounded data slice adds fixed array values, repeated initializers and
literal constant indexing. It does not pretend that the existing i32 type is
Rust usize. Runtime indices, slices, array patterns, general constant expressions
and library methods require later work.

Source types are `[T; DECIMAL_LENGTH]`. Nonempty literals evaluate elements from
left to right. `[expr; N]` evaluates expr exactly once, including N=0. Repeating
more than once requires Copy in this slice. N=0 and N=1 allow a non-Copy value,
but N=0 still consumes it. An array is Copy iff its element type is Copy, even
when its length is zero. Empty literal inference requires a concrete contextual
element type or an explicit unsupported diagnostic, never a fabricated Never
or Unit element type.

Checked Ty gains Array { element: Arc<Ty>, len }, and Projection gains
ConstantIndex(usize), distinct from numeric tuple Field projections. Indexing
accepts only in-bounds decimal literals. Any non-Copy Move through an indexed
place is rejected, even when the final projection is an inner non-Copy field or
the array has one element. Copy scalar-field reads can project without moving
the non-Copy containing element. Unlike a bare ignored local, an indexed wildcard
still requires the indexed array base to be live and initialized. CFG
`Statement::Inspect(Place)` records that non-consuming read before every index
projection, including ignored patterns. The backend emits no instruction for
Inspect. Only the exact indexed base is read, not unrelated enclosing siblings.

Array element initialization paths must be distinct so payload selection proofs
for one element cannot authorize another. Whole writes initialize every child;
indexed writes require initialized ancestors and a mutable root. Writes/moves
invalidate proofs only in affected subtrees. Zero-length arrays still retain an
initialization root marker and their own nominal element type. Type/declaration
validation traverses element types even for length zero, including recursive
by-value dependency rejection.

Every array uses owning aggregate snapshots and the private indirect call/result
convention, including length zero and one. Storage uses checked recursive element
stride, with an eight-byte backing sentinel for empty arrays. Layout, expanded
paths, proof states, comparisons, work and diagnostics retain fail-closed limits.
This representation is not a Rust platform ABI promise.

Independent stock-classified tests must cover constructor/repeat effects and
source ordering, non-Copy repeats and indexed move rejection, mutable initialized
writes, alias-safe snapshots, nested aggregates/payload enums, empty arrays,
whole-value calls/returns, initialization joins and resource bounds. The independent
30-case corpus separates 12 accepted programs, 13 invalid programs and five legal
unsupported boundaries. The exact historical array fixture is now accepted and
executed without changing its source. `tests/array_graph.rs` checks once-only
body/CFG/analysis facts and retained failure reports. Native backend tests cover
4096 repeats with bounded generated code, large by-value copies, zero backing
and nested empty-array strides. Tracked remote corpus evidence is retained in
`FINDINGS.md` when complete. This is not full Rust or a speed claim.
