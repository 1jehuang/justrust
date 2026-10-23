# Slice descriptor returns

Status: root shared and mutable slice returns, plus shared descriptors in physical aggregate carriers. Raw constructors and mutable subslices have separate contracts. This is not full Rust support.

## Frozen contract

Functions may return a root `&[T]` or `&mut [T]` for reference-free, sized element `T`, including generic non-Copy elements. Returning a slice transports its descriptor, never copies or moves an element. Shared subslice results preserve the input storage lifetime. Mutable results preserve exclusive access, and subsequent reads or writes obey the same authority checks as mutable parameters.

Returning references to local arrays is invalid. Returned slice provenance must satisfy the declared input lifetime relation. An unsafe raw constructor is a separate stage and is not implied by admitting descriptor results. Shared descriptors may occur in tuples, arrays and nominal/enum fields; one existing shared-reference element layer retains its separate typed-slice contract. Nested mutable slice carriers and deeper slice element carriers remain unsupported.

Source `chunks` supports forward and reverse consumption, partial final chunks,
empty and zero-sized inputs, and a panic on size zero. Chunk results preserve
the owner's shared borrow. Heap collections containing borrowed elements are
not covered: `Vec<&[T]>` needs general type-lifetime retention across raw storage,
not invented physical loan origins. Raw storage containing slices is explicitly
rejected as unsupported in CHECK and BUILD, including invalid escaped collections.
Its positive and invalid-input acceptance cases remain an explicitly ignored,
unfinished full-support gate in `slice_methods`; the conservative rejection has
separate active stock-differential tests at workers 1 and 8.

## IR and CFG

Existing reference contracts and issued loan origins carry returned storage authority. The result type is validated recursively in checked IR and CFG. Code generation uses the existing indirect result ABI: the caller owns the two-word descriptor destination, including descriptor fields in aggregates. No pointer to callee-owned descriptor storage escapes. A mutable identity return may require an implicit whole-slice reborrow; the referent is validated without treating the unsized slice as an independently sized value.

Graph-realized body carrier coordinates are replayed from the exact canonical
body contract and validated lowering proof, just as call coordinates are.
Changing the result signature, subject, compilation authority or lowering
proof cannot authorize a transported result. Equivalent-looking nominal names
are not interchangeable identities.

No new whole-program node, mutable fact, side result, global budget or graph ratchet is introduced. CHECK must not demand code generation.

## Acceptance gates

- Stock differential CHECK, BUILD and native run at workers 1 and 8 for shared, mutable, generic, non-Copy, empty and shared-subslice result transport.
- Invalid local-storage returns, unconstrained lifetime returns, shared-to-mutable strengthening and live alias violations rejected in CHECK and BUILD, including unused bad functions.
- Forged result element, borrow kind and source/contract metadata rejected.
- Actual graph confirms no CHECK codegen, and all graph-integrity ratchets pass unchanged.
- Shared aggregate, generic identity and chunk cases use stock CHECK/BUILD/run at workers 1 and 8. Root mutable coverage remains separate from unsupported nested mutable carriers.
