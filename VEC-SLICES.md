# Vec slice views

Status: implemented source-method slice with stock-differential native, alias/escape, raw-slice authority, and graph acceptance. This is not full Vec, Iterator, native-memory pointer, or Rust standard-library compatibility.

## Contract

`Vec<T>::as_slice(&self) -> &[T]` and `Vec<T>::as_mut_slice(&mut self) -> &mut [T]` are ordinary source methods around the canonical unsafe `std::slice::from_raw_parts` and `from_raw_parts_mut` APIs. They preserve the Vec allocation, element type, length, and receiver loan. They do not copy elements or reinterpret a thin pointer as a fat reference.

An empty Vec exposes an aligned non-null dangling element pointer using the actual canonical `align_of::<T>()`. Its zero-length slice does not access that address. Non-empty pointers continue to designate the existing private element storage. No public size, pointer-distance, allocation ABI, enum niche, or packed native-memory compatibility claim follows from these wrappers.

Shared views prevent conflicting mutable access while live. Mutable views are exclusive and permit element mutation under the supported element ownership rules. Generic non-Copy reference-free elements can be transported without extraction or copying. Returning a view of a local Vec is rejected, including in unused functions.

## Graph and representation

The wrappers use existing source checking, canonical call evidence, typed raw-slice descriptor IR, and receiver-region transport. No new compiler node, side result, mutable fact, or layout authorization is introduced. CHECK must not demand object generation. The build root retains the existing all-required-checks success gate.

## Acceptance

`tests/vec_slices.rs` compares stock and native CHECK/BUILD/run at workers 1 and 8 for empty and non-empty views, real empty-pointer alignment, shared and mutable effects, and generic non-Copy elements. Invalid controls cover overlapping mutable views, shared/mutable overlap, and escape from a local Vec. Existing raw-slice source authority and graph tests remain prerequisites.

Native stock-differential CHECK/BUILD/run at workers 1 and 8 passes, as do alias/escape controls, canonical raw-slice authority, and graph checks. Scalar observations in the positive fixture are bound before formatting, keeping the existing temporary/indexed-format ingress boundary separate. The full-program manifest retains its remaining Vec and Iterator requirements.
