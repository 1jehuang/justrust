# Closed-type layout and pointee destruction proposal

Status: design only, 2026-10-19. No node ID reserved, no layout intrinsic or
Box support implemented by this document. Coordinate with canonical nominal
handle and resource-authority owners before changing shared IR interfaces.

## Why another fact is required

The current backend `value_size`, `field_offset` and array stride describe a
private carrier representation. Small scalars and unit occupy words, empty
structs have dummy storage, and wide integers have a private limb convention.
These are not Rust-addressed object layouts. Reusing those values as
`std::mem::size_of` / `align_of`, or allocating stock-sized objects while
accessing private field offsets, would be incorrect.

Generic pointer-field owner Drop is a separate completed prerequisite. It
resolves canonical concrete destructor instances, but does not establish
pointee layout, pointee cleanup, nested owner glue or Box ownership semantics.

## Fact identity and authority

Proposed logical operation: `LAYOUT(closed_type, target_contract)` with no
numeric node kind assigned yet. Its key must include a canonical, exact closed
type identity and target/layout-semantics version. Nominals use resolved
source declaration identities plus ordered arguments, never display names,
process pointer addresses, insertion-order interner indices or hash-only type
identity. Equivalent requests share one node; different source authority,
arguments, targets or layout versions cannot alias.

A `ClosedTypeKey` must use the immutable nominal authority being designed by
the nominal lane. An exact length-delimited encoding into engine key params
is one candidate, not permission to introduce a process-global type registry.
A caller-provided integer or deserialized record is not sufficient type
proof. The producer reads the canonical type/declaration fact through engine
handles and checks closure, supported representation and well-formedness.
Unknown parameters, associated projections and unsized operands reject
explicitly until a separate supported contract exists.

The immutable output contains:

- Exact closed type identity, target contract and layout version.
- Object byte size, power-of-two alignment and checked array stride.
- Representation class, scalar validity class and offsets when supported.
- Typed child layout handles for exactly the components inspected, not every
  declaration or callable in the program.
- Canonical source/declaration provenance for nominal representation choices.

No PlanCtx, SchemaAdapter, mutable budget, shared memo or backend closure is
retained. Work, bytes, depth and checked arithmetic are paid by the producing
task. Child requests are issued independently before awaits where possible.
By-value recursive layout must diagnose deterministically, never deadlock or
create an unbounded task chain. A raw/reference pointer's layout does not
request the layout of its pointee.

## First representation contract

First admitted addressed types: supported integers, floats, bool, char, unit,
thin references/raw pointers to sized admitted pointees, and fixed arrays of
admitted elements. Never, fat pointers, slices, trait objects, structs, tuples
and enums reject until their own precise contracts are implemented. Target
constants are explicit Linux x86-64 ABI data, not host `size_of` answers.

Unit has size zero and alignment one. Arrays use checked element-size times
length with the element alignment, including zero length and zero-sized
stride. Integer alignment, including 128-bit values, comes from the selected
target contract and stock/C executable controls, not the current carrier limb
alignment. All overflow, unsupported target and excessive-object cases have
distinct explicit diagnostics. Bool and char retain Rust validity requirements;
allocation does not initialize them or make arbitrary bit patterns safe.

Nominals are a subsequent increment. `repr(C)` needs target C field padding,
aggregate alignment and trailing padding with bidirectional C tests. Rust
representation needs an explicit internally consistent field-order/niche
contract, not an assertion that arbitrary Rust struct field order is a stable
external ABI. Enums require discriminant/payload/niche proofs. Do not publish
an approximate size for any rejected shape.

## Carrier versus addressed storage

Keep private call/register carriers separate from `ObjectLayout`. Every load,
store, field/index projection and address calculation carries a checked storage
representation. No codegen decision can silently infer it from the expression
spelling or local name. New layout facts cannot be promoted to public memory
intrinsics until the addressed consumers for admitted types use them.

Preferred integration: addressable local storage uses object layout, while
register/call transport retains a distinct carrier plan with explicit typed
pack/unpack conversions. Borrowed arrays must not use a detached compacted
mirror: that breaks alias identity and mutation visibility. Reference-derived
and raw-derived places use the same object offsets and element stride.
Conversions must preserve wide values, bool/char validity, initializedness,
move state, provenance and non-Copy aggregate identity. A unit/zero-sized
object address must satisfy non-null/alignment requirements without inventing
a byte load or store. No public FFI/layout claim follows from private carrier
transport.

## Source intrinsic and cleanup evidence

Source `size_of::<T>()` and `align_of::<T>()` require canonical source binding
and a typed operand in checked IR. Generic universal bodies retain an
unresolved typed operation, not a guessed constant. A demanded closed body
reads its exact layout fact, then seals the concrete operation with normal
source replay/generated-shape authority. CHECK reads layout facts but does
not demand machine code or linking.

`drop_in_place::<T>(*mut T)` must be unsafe and bind an exact canonical source
occurrence, pointer mutability/pointee type and closed cleanup plan. It is not
an allowed explicit `Drop::drop` call and must not bypass the existing E0040
rejection. For a trivial type the fact proves absence of owned cleanup. For
an owner it retains the canonical destructor target and all recursively
required field cleanup, with explicit source/instance analysis and object
demand dependencies. Universal unresolved is never a trivial closed proof.

Existing `ValidatedCleanupCall` is restricted to whole unprotected local
roots. Do not loosen that authority to make raw-pointer cleanup fit. Introduce
separate validated pointee-cleanup evidence, with canonical closed-type and
layout handles, through the agreed IR/CFG proof interface. Codegen consumes
that evidence and the existing symbol resolver; it cannot create an unchecked
call or find a destructor by method name. Pointee cleanup does not own or free
the allocation by itself. Panic=abort has no fabricated unwind cleanup.

## Source Box acceptance, not part of the first layout increment

A real source owner must allocate enough correctly aligned addressed storage,
initialize its T exactly once, run T's validated cleanup before deallocation,
and disarm allocation ownership exactly once on consuming transfer/release.
Allocation errors and zero-sized handling need explicit source policy. A
Copy-only container or manual-release demonstration is not Box completion.
Nested Allocation/Box payloads, early return, replacement, move and partial
move rules, projection loans, moved-out state, destructor borrows and abort
controls remain required. Deref/DerefMut remain separate until their actual
`Target: ?Sized` contract is supported. No compiler per-method container cases.

## Required acceptance and authority gates

1. Freeze stock edition-2024 sources before native classifications. Compare
   size/alignment/array stride and actual scalar-address reads/writes at
   opt-level 0/2, CHECK/BUILD workers 1/8. Include mapping boundaries, i128
   alignment, bool/char, unit, empty arrays and arrays of zero-sized values.
2. Bidirectional C executable controls for admitted scalar/array addressed
   memory. Do not claim aggregate ABI until actual repr controls pass.
3. Mutate type identity, target version, child layout, size/alignment/offsets,
   source occurrence, concrete type args and destructor demand. Every forged
   or stale artifact must fail validation. Test trivial versus owning cleanup
   instantiations of the same universal operation.
4. Prove CHECK has no codegen dependencies, unrelated functions do not enlarge
   the layout closure, duplicate requests reuse the same exact fact and
   source/type/target edits invalidate the appropriate dependents.
5. Preserve immutable reachable facts, task-local resource accounting and all
   unchanged graph integrity ratchets. No new whole-program join or limit
   increase. Unsupported recursive/unsized/nominal forms reject explicitly.
6. Only after those pass, implement source Box with exact-once nested-owner
   execution, stock semantic negatives and allocation failure/alignment tests.
