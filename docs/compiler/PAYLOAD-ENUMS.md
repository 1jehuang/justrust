# Payload enums: native sum-value milestone

Status: implemented, with 533 hosted tests and strict all-target lint passing.
Independent native M8a differential validation is recorded in FINDINGS.md.
The full Rust roadmap remains open in `IMPLEMENTATION.md`.

## Source boundary

Extend nonempty nominal enums with unit, tuple and braced variants, including
zero-field tuple/braced shapes. Variant fields are implicitly public according to
Rust enum rules. Accept source-order constructor evaluation, qualified/simple
imported constructors, top-level variant match arms and existing irrefutable
payload patterns. Single-variant enum let patterns are irrefutable. Mixed
struct/enum by-value dependency cycles must be rejected before immutable nominal
handles are built. Unused declarations are validated too.

Nested refutable patterns, guards, or/range patterns, references, Drop, derives,
explicit discriminants, empty enums and mutation through payload projections
remain following work. Singleton nested patterns may reuse irrefutable typing;
any unsupported inference/pattern remains an explicit legal boundary.

## Typed identities and projections

`VariantShape::{Unit, Tuple, Struct}` and
`VariantDef { name, shape, fields: Arc<[FieldDef]> }` retain source shape.
`EnumDef.variants` is an immutable array of those definitions.
`VariantId { owner: DefId, index }` includes the nominal owner, not just an ordinal.
`Projection::{Field(usize), Downcast(VariantId)}` replaces untyped field vectors
on checked places and flattened pattern bindings. A downcast must be followed by
a field projection. The shared projected-type helper validates identities and
bounds, but never authorizes a variant payload read by itself.

Checked enum constructors retain field expression IDs in source order. Lowering
snapshots those expressions before ordering operands by declaration index.
`Rvalue::EnumVariant` owns those operands, enum type and selected ordinal.
`SwitchVariant { place, variant, then_block, else_block }` is a typed runtime tag
test and a semantic selection edge. Both check/build use the existing per-function
BODY, LOWER and ANALYZE facts without an enum-wide body barrier.

## Sum initialization and selected-variant proofs

Initialization paths include every variant's payload fields. Whole initialization
sets all variant paths logically initialized: inactive paths are vacuous facts,
not proof that their physical memory contains values. A selected payload move
clears only that subtree. Whole enum reads and multivariant tag tests require the
entire enum subtree initialized, preserving Rust's partial-move restrictions.
Singleton patterns do not add a tag read and permit independent field extraction.

A separate must-selected-variant fact authorizes every multivariant downcast.
A successful typed true edge establishes it, joins intersect facts, and whole
writes/moves/storage changes invalidate proofs at the affected enum and every
contained enum. Moving a selected child retains the enclosing enum proof but
invalidates proofs contained in the child. False edges establish no new fact.
Refinement NEVER restores initialization bits. Same-target true/false successors
must merge both edge states rather than accidentally retaining the true proof.
Proof storage, clones, joins, invalidations and edge copies are budgeted.

Wildcard-only matches still require no read. Existing constant-true semantic ghost
edges preserve dead-arm ownership obligations and normal continuation joins.
Later variant arms establish payload authorization only through their actual
semantic variant-test true edge, never a blanket assumption at arm entry.

## Native private representation

Fieldless declarations remain scalar words. A declaration with any non-unit
variant shape uses the aggregate calling convention for EVERY variant, including
unit constructors. Its private frame representation is an eight-byte tag plus
the largest variant payload, with checked offsets/frame limits. Constructors
zero the entire backing before storing tag and payload, preventing uninitialized
padding or inactive bytes from escaping through physical copies. This is not a
platform Rust layout/FFI guarantee.

Source constructors use owning `Rvalue::EnumVariant` snapshots. A scalar
`Constant::Enum` is valid only for a unit variant of a wholly fieldless enum.
Calls, parameters, returns, branches, loops and snapshots use the existing
by-value aggregate/sret rules. A downcast offsets past the tag only after the
analyzer proves selection.

## Acceptance gates

Keep independent stock-classified positive, invalid and legal-unsupported cases.
Run real native output at workers 1/8 and a tracked 96-worker host example. Cover
mixed variants, zero fields, source order, nominal/privacy/alias handling,
struct-enum cycles, recursive calls and large payload ABI; selected/partial moves,
singleton independent patterns, wildcard no-read, dead-arm joins and whole-value
reinitialization. Synthetic CFG tests must reject unproven/wrong-owner/stale
variant projections and proof leakage through joins/backedges. Preserve resource
caps and verify exactly one cached analysis per function. No performance claim or
full Rust compatibility follows from passing these gates.
