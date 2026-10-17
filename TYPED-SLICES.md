# Typed shared slices with checked element layout

Status: pending implementation acceptance. Full Rust compatibility remains open.

## Scope and authority

Generalize the delivered shared byte-slice path to shared `&[T]` parameters and
locals using the exact existing structural element type. Admit existing sized,
reference-free scalar and aggregate elements, including wide integers, and rigid
generic elements under the original parameter environment. This is not an element
name whitelist or byte reinterpretation. Preserve the exact behavior of
`shared_byte_slice()` and `shared_byte_array_len()` as byte-only helpers.

Add `Ty::shared_slice_element() -> Option<&Ty>` and
`Ty::shared_array_element_len() -> Option<(&Ty, usize)>` for exact shared root
reference shapes. A shared checked admission helper
`validate_shared_slice_element(element, facts, budget) -> Result<(), String>`
validates all metadata and binder owners, Sized obligations and the bounded
reference-free element boundary. Reuse the same helper after concrete
substitution, in checked IR/CFG admission and before native publication. An
opaque universal parameter is not a permanent reference-free certificate:
concrete reference or reference-containing nominal elements remain unsupported.
Metadata, allocation and resource errors must not become that boundary.

`ArrayToSlice` preserves exact element identity and derives length in ELEMENTS
from the actual array type. It evaluates the reference once and preserves its
existing allocation, loan and origin authority. `SliceLen` reads only the handle
and returns usize, without requiring Copy on T. `SliceIndex` evaluates the receiver
and index once, requires genuine usize and authoritative Copy permission on T,
and produces a real independent value of exactly T. No move from borrowed indexed
storage or implicit Copy permission is introduced. Borrowed index projections of
non-Copy elements remain a separate unsupported place capability.

Mutable slices, ranges, slice patterns, nested reference-bearing elements,
reference-containing slice aggregates and slice returns remain outside this
milestone. Existing source names, builtin len occurrence certificates, facts
phases, ownership seals and canonical source replay remain authoritative.

## Native descriptor and memory contract

Keep the existing private two-word data/length descriptor and two-word argument
transport. No extra runtime stride word is added. Derive a checked concrete
stride from the SAME private element layout as arrays, not from the source name,
scalar width or the old u8 stride field. The private backend is not a Rust/FFI
layout claim, including its initialized dummy storage for empty values.

Snapshot the descriptor and index exactly once. Check unsigned index < len before
scaling, forming an element address or reading memory. Check extent/offset
multiplication and pointer addition before load; overflow traps rather than
wrapping. Empty slices cannot load an element. Scalar loads retain actual signed,
float and bool semantics; wide values and Copy aggregates use their complete
private representation and independent result storage, never a thin alias or
byte-normalized load. Both descriptor words survive calls, copies and generic
instantiation, and concrete closure rejects unresolved parameters or inadmissible
element layouts before emission.

## Resources and acceptance

Use existing work/depth/array/frame/argument limits without increasing caps.
Charge type/layout walks, vectors and copies before work/allocation, reconcile
actual capacities and preserve allocation/resource latches. Any cache requires
exact instantiated type/application and facts/world identity; a universal proof
cannot stand in for a concrete layout check.

Freeze stock-first signed/wide/float/bool/Copy aggregate reads, generic Copy
readers, non-Copy len-only inspection, element mismatch, non-Copy extraction,
live backing-array loan conflicts, receiver/index effect order and empty/equal/
max-index traps. Include original byte-slice regressions and actual malformed IR,
foreign nominal/parameter, substituted reference element, copy-proof and carrier
origin controls. Promote historical source outcomes only by explicit authorization
with source bytes unchanged.

Require native workers 1/8 CHECK/BUILD with compiler fallback disabled, real ELF
execution under debug/release bootstrap profiles, complete unfiltered regressions,
strict lint and formatting. Retain failed and passing measured receipts. Associated
type/projection normalization remains the next independent proof foundation, not
an Iterator or full Rust claim from this slice milestone.
