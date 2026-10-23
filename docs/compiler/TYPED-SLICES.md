# Typed shared slices with checked element layout

Status: accepted bounded implementation on October 17, 2026. Full Rust
compatibility, general DSTs and the excluded borrow capabilities remain open.

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

## Shared-reference element prerequisite (October 20, 2026)

The element boundary above now also admits one shared reference to an otherwise
admitted reference-free referent, including `str`: `&[&str]` and `&[&i32]` are
examples. The new typed `CarrierStep::SliceElement` denotes the conservative
all-elements domain, never an alias-disjoint runtime index. Known backing arrays
expand to their actual element cells; abstract input slices retain one summary
cell. Loading an element copies its stored inner carrier, rather than replacing
its lifetime with the lifetime of the slice handle. Independent source regions
and the implied inner-outlives-outer relationship remain explicit declaration
facts. Loaded references survive destruction or rebinding of the handle only
when their original referents remain valid.

Checked IR, CFG, ownership and native layout validation share this boundary.
Exact shared-reference argument autoderef and explicit `*items[index]` operate
on the loaded carrier. Neither path invents a fresh borrow or drops the selected
reference's origins. Within one input parameter, nested occurrences of the
same explicit named lifetime permit that lifetime to supply an elided output.
This does not deduplicate separate input parameters: their existing output
elision ambiguity rules still apply.

Mutable reference-carrying slice cases, deeper or aggregate reference-bearing
elements, and new reference topology introduced through generic substitution
remain explicitly unsupported. Explicit `'static` annotations on slice element
references are also unsupported until static input authority is represented;
they are never treated as anonymous input lifetimes.

The existing slice suites contain stock differential CHECK/BUILD/run witnesses
at workers 1 and 8, including the unchanged `*s[0]` and `for it in items` cases,
loaded-reference lifetime and alias controls, forged result/element/carrier
facts, and CHECK graph coverage without code generation. Focused release gates
and unchanged graph-integrity ceilings pass. Broader slice-suite failures remain
reported separately: builtin `.len()` result constraints, typed empty-array
inference (`let a: [i32; 0] = []`, corpus `bounds_empty`), and subslice inference
(`&a[1..]`, corpus `slice_range`). Their existing success/trap expectations are
unchanged, not reclassified as unsupported to make the suite pass. This is a
prerequisite for error-handling programs, not a claim that unchanged
`full_programs/15_error_handling.rs` or full Rust now compiles.

## Length inference follow-up (October 20, 2026)

The later unchanged-source baseline isolated `noncopy_len` as an unresolved
nominal-constraint failure; `bounds_empty` and `slice_range` already passed CHECK
at workers 1 and 8, so their production paths were not changed. Slice and string
length now constrain the result to `usize` without waiting for unrelated nominal
layout. Array receivers retain their original reference type and complete ordinary
method selection before permitting array-to-slice length fallback. Selection
errors, generic-argument failures and real in-scope trait methods never authorize
that fallback. Checked lowering still validates the canonical builtin occurrence.

The complete existing `typed_slices` suite passed 10/10, including unchanged
128-cell/50-ELF native acceptance and the expanded 15-source shared-reference
stock CHECK/BUILD/run matrix at workers 1 and 8. Existing array native/authority
suites passed 6/6, including trait priority and invalid-selection controls. The
array graph suite passed 2/2 with CHECK proving no code generation and BUILD
retaining the exact loan/unsize assertions. Graph integrity passed 14/14 without
changing ceilings. These are bounded inference repairs, not full-program support.

## Declared generic element families (October 23, 2026)

The generic-substitution restriction above is narrowed for an original declared
slice element parameter. Its source signature retains a `ParamTail` at the exact
`Referent/SliceElement` path, tied to the original generic binder. Substitution
may expand that family to the already supported single shared-reference element
shape. It cannot reconstruct a missing family from a concrete type. Missing,
foreign, duplicated or displaced source rows reject; trait headers retain their
original root reference contract while adding only declared family rows.

A copied generic element returned as a reference keeps its original element
lifetime, not the lifetime of the array or slice descriptor holding it. The
existing authenticated result-flow check and complete target/dependency closure
must succeed before the old descriptor-region fallback is skipped. The selector
requires an exact root parameter family, canonical component mapping and unchanged
ordered source paths. Escape and outstanding-loan mutation still reject.

Two independently borrowed mutable slice results may retain conservative generic
family candidates only when closed, validated actual types prove those families
contain no carriers. This does not partition one input or admit reference-bearing
mutable element families. No lifetime, alias, graph or resource ceiling is relaxed.

Stock/native CHECK, BUILD and execution at workers 1 and 8 cover generic reference
length/read, a returned element surviving destruction of its containing descriptor,
and independent mutable inputs. The broader trait-argument inference, associated
reference equality and unchanged error-handling program remain open acceptance
work, not supported by these bounded proofs.

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

## Measured acceptance

The frozen 32-source stock-first corpus retains every source byte and its order:
18 normal runtime programs, three bounds traps, five ordinary invalid programs
and six stock-valid capability boundaries. Native workers 1/8 CHECK/BUILD execute
128 cells and 42 ELF images in each debug/release bootstrap profile with compiler
fallback disabled. Stock abort/panic diagnostics and the native exit-101 empty
stream trap contract are checked separately, not reported as identical behavior.

Original facts validate all typed functions, including nominal Copy permission
and rigid generic templates. The 28 public authority controls and six private IR
controls retain malformed element/result/index, foreign metadata, concrete
reference substitution, original resource latches and real Copy-proof checks.
Four existing private parameter witnesses are promoted without changing source;
byte-only helper predicates and the remaining old boundaries stay unchanged.
Two newly written temporary-array-borrow controls retain their exact source as
parser boundaries rather than pretending this milestone admits rvalue borrows.

The final frozen hosted gate passed all-target check, strict all-target Clippy
with `-D warnings`, 2,795 unfiltered tests across 136 binaries, release public
acceptance 34/34, formatting and diff checks. The earlier release private slice
filter passed 49 tests with 1,048 unrelated tests explicitly filtered out. The
retained receipt file is `data/runs/xps14/2026-10-17-typed-slices.jsonl`.

Real failed gates are retained alongside the passing runs: missing array-to-slice
inference evidence, missing canonical Slice element transport, exponential shared
nominal-DAG scanning, the scratch type annotation, precise earlier capability
diagnostics, legacy fact-free Copy validation and a mistaken release target name.
The DAG scan now charges checked postorder traversal and actual-capacity scratch,
retains subtree heights on reuse, and stages backing before admission. Allocation
failure is an error, not an invented byte-budget-denial record. Early hosted SSH
failures used the stable client's explicit local fallback; the final acceptance
ran on hosted. No compile-speed or broad Rust compatibility claim follows.
