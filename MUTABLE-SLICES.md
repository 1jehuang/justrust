# Mutable slice parameters

Status: mutable slice parameters and local carriers. Root slice returns are covered by `SLICE-RETURNS.md`; raw constructors and mutable range borrows remain open. This is not full Rust support.

## Frozen contract

A root `&mut [T]` uses the private two-word data/length descriptor. Array references unsize only to a slice with the same borrow kind and exact element identity. Generic parameters preserve that identity. Reference-free non-Copy elements may be transported and their length inspected, but moving elements out or replacing non-Copy elements is explicitly unsupported in this stage.

Mutable carriers are not Copy. Calls may reborrow an existing carrier. Reads and length queries create shared reborrows, leaving the carrier available after the read. Mutable indexing writes require writable authority through every dereference. The right-hand value is evaluated before the index, followed by the bounds check and write. Empty and out-of-bounds accesses trap rather than write memory.

## IR and CFG

Array unsizing retains `ArrayToSlice` with matching source/result borrow kinds. Reads use the existing shared `SliceIndex` and `SliceLen` through a shared reborrow. Writes use `AssignIndex`/`AssignIndexOp` on the dereferenced carrier, lowering to CFG `IndexAssign`/`IndexAssignOp`. The backend reads the descriptor and checks the unsigned index against its length before computing the element address.

The same typed per-function facts validate source, universal IR, CFG and concrete layout. No new whole-program operation, mutable fact payload, global budget or side result is introduced. CHECK does not demand code generation.

## Acceptance gates

`tests/mutable_slices.rs` compares frozen rustc 1.98.1 edition-2024 stdout witnesses for generic replacements, wide elements, repeated reborrows, empty/non-Copy length inspection and assignment evaluation order at workers 1 and 8. Trap checks require native exit 101 and preserved stdout, not stock abort exit parity.

`tests/mutable_slices_authority.rs` rejects shared mutation, overlapping mutable borrows, use after move, dangling local storage and malformed types, including unused functions. `tests/mutable_slices_graph.rs` checks the real CHECK graph and dereferenced indexed writes at workers 1 and 8. The repository graph-integrity ratchets remain mandatory and unchanged.
