# Root slice returns

Status: root shared and mutable slice returns with reference-free elements. Raw constructors and mutable subslices remain separate work. This is not full Rust support.

## Frozen contract

Functions may return a root `&[T]` or `&mut [T]` for reference-free, sized element `T`, including generic non-Copy elements. Returning a slice transports its descriptor, never copies or moves an element. Shared subslice results preserve the input storage lifetime. Mutable results preserve exclusive access, and subsequent reads or writes obey the same authority checks as mutable parameters.

Returning references to local arrays is invalid. Returned slice provenance must satisfy the declared input lifetime relation. An unsafe raw constructor is a separate stage and is not implied by admitting descriptor results. Nested reference or slice element carriers remain unsupported.

## IR and CFG

Existing reference contracts and issued loan origins carry returned storage authority. The result type is validated as a root slice carrier in checked IR and CFG. Code generation uses the existing indirect result ABI: the caller owns the two-word descriptor destination. No pointer to callee-owned descriptor storage escapes. A mutable identity return may require an implicit whole-slice reborrow; the referent is validated without treating the unsized slice as an independently sized value.

No new whole-program node, mutable fact, side result, global budget or graph ratchet is introduced. CHECK must not demand code generation.

## Acceptance gates

- Stock differential CHECK, BUILD and native run at workers 1 and 8 for shared, mutable, generic, non-Copy, empty and shared-subslice result transport.
- Invalid local-storage returns, unconstrained lifetime returns, shared-to-mutable strengthening and live alias violations rejected in CHECK and BUILD, including unused bad functions.
- Forged result element, borrow kind and source/contract metadata rejected.
- Actual graph confirms no CHECK codegen, and all graph-integrity ratchets pass unchanged.
- Existing slice-return Unsupported tests promoted only for supported root cases.
