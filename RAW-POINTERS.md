# Raw pointers and unsafe blocks

Status: first slice implemented 2026-10-18 (heap lane, agent t-rex). Full Rust
unsafe semantics remain open (IMPLEMENTATION.md G3).

## Supported

* Types `*const T` and `*mut T` (`ir::Ty::RawPtr { kind, pointee }`), including
  generic pointees, struct fields of raw pointer type and raw pointer
  parameters/results. Raw pointers are Copy, Sized, one machine word and carry
  no loan or provenance.
* `unsafe { ... }` block expressions (`Expr::Unsafe`), lowered as ordinary blocks.
  The checker tracks the unsafe nesting depth.
* Dereference `*p` as a read, as an assignment destination and under field
  projections (`(*p).f = ...`). Outside `unsafe` it is rejected with E0133,
  including in unused functions. Writing through `*const T` is rejected.
* `as` casts (`ExprKind::PtrCast`, `Rvalue::PtrCast`, `ir::ptr_cast_valid`):
  `&T`/`&mut T` to `*const T`, `&mut T` to `*mut T`, raw pointer to raw pointer
  (any pointees), raw pointer to `usize` and `usize` to raw pointer. Other
  pointer casts are rejected (E0606).
* `==`/`!=` on identical raw pointer types (address comparison).

## Analysis contract

Storage reached through a raw pointer dereference is outside every tracked
canonical root (`ir::raw_deref_index`). Initialization, move and loan analyses
check the pointer prefix as an ordinary read and leave the pointee untracked,
which is what Rust's `unsafe` contract delegates to the programmer. Values read
through a raw pointer carry no reference origins.

## Not yet supported

The later NATIVE-HEAP.md foundation adds ordinary Rust-source `null<T>`,
`is_null<T>` and checked byte-address arithmetic helpers under the explicit
`std::native_heap` bootstrap module, plus actual scalar-width dereferenced
memory transport. These are not raw-pointer inherent methods or public
aggregate-layout support. The method/API exclusions below remain open.

Raw pointer methods (`add`, `offset`, `is_null`, `read`, `write`,
`std::ptr::null`), `&raw const`/`&raw mut`, unsafe functions/traits, fat raw
pointers (`*const [T]`, `*const str`, `*const dyn T`), pointer ordering and
arithmetic operators, and casts between pointers and non-usize integers.

## Evidence

`tests/heap.rs` with stock-rustc-frozen fixtures in `tests/fixtures/heap`
(1 positive with exact stdout, 4 stock-rejected sources). The former
`integer_casts` reference-to-pointer legal-unsupported case is now positive.
