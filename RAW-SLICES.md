# Raw slice construction

Status: implemented for the bounded contract below. This is not full Rust support.

## Contract

The bundled `std::slice` module exposes unsafe `from_raw_parts<'a, T>` and
`from_raw_parts_mut<'a, T>` functions. They construct shared and mutable slice
descriptors from a matching raw pointer and a `usize` element count. The
supported element family is the reference-free slice element family, including
non-Copy elements. Construction does not copy, move or read an element.

The caller must uphold Rust's allocation, alignment, initialization, range,
aliasing and lifetime requirements. Empty slices still require a non-null,
aligned pointer. These are unsafe preconditions, not dynamic checks or permission
to dereference arbitrary pointers in safe code.

The source API is an ordinary checked generic function call. Its body invokes
a hygienically resolved compiler intrinsic macro under an unsafe block. Pointer
and length are evaluated once in source order. Generic argument inference and
explicit `::<T>` instantiation preserve the same checked source signature.

## IR and CFG

`Expr::RawSlice` checks the unsafe context and exact pointer mutability and
length type. `ExprKind::RawSlice` and `Rvalue::RawSlice` retain the two operands
and shared/mutable kind. Metadata validation checks arity, operand identity,
result element/kind and completion. A canonical lowering snapshot preserves the
operation and source origin even for a constructor body with no calls.

The backend writes pointer and length to the existing two-word slice descriptor.
It does not manufacture a reference to an element to obtain the descriptor, so
valid aligned dangling pointers remain usable for empty slices.

Borrow analysis reserves a stable per-function origin for each canonical
constructor statement before solving. Transfer publishes that issued origin
with no storage targets, as required by the raw pointer's unsafe contract. It
never treats the absence of an origin as valid provenance. Ordinary wrapper
calls retain their declared reference dependencies, including `&self` and
`&mut self` lifetime authority.

## Gates

- Native stock-differential CHECK and BUILD/run at workers 1 and 8, including
  generic wrappers, mutation, non-Copy transport, zero lengths and operand order.
- Wrong pointer kind, wrong length type and calls outside unsafe are rejected.
- Forged typed operands, result kind, missing/changed constructor origins,
  cross-site pointer/length operands and missing or foreign canonical snapshots
  are rejected before borrow provenance is issued.
- Per-function graph outputs remain typed and CHECK does not demand codegen.
- Graph-integrity ceilings stay unchanged.

This does not add mutable range borrows, slice element reference families,
non-Copy element replacement, or full Rust support.
