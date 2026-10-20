# Referenced-array and shared-slice for-loop loans

Status: bounded language milestone, not general Iterator or mutable-slice support.

## Admitted source

Statement-position `for item in &array` and `for item in &mut array` use real
shared/mutable element references for fixed arrays of integer, float, char,
bool, or unit elements. Existing array-length limits apply, including empty
arrays. There is no special 32-element ceiling. Explicit reference-valued
array sources, source calls, nested control flow and ordinary source-body
bindings/calls use the same machinery. The iterable expression is evaluated
once before the new binder and loop frame become available.

Shared scalar/unit slice sources (`for item in slice`, where `slice: &[T]`)
use the same single checked body and canonical element-place authority. The
descriptor is evaluated and saved once. The loop reads its genuine captured
length, and each borrow creation captures data/length together before checking
the index. An item may outlive a descriptor local when its actual owner still
lives. No mutable descriptor authority is inferred from shared slice support.

The source body is checked and emitted once. A discarded bounded-dispatch
experiment duplicated bodies and conflicted with original source call/binding
identity. Its local checkpoint is retained in history, but no dispatch or
source-capture exception remains in the implementation.

Every value iterable has an exact retained source length occurrence: a fixed
array bound or a checked shared-descriptor `SliceLen`. These are inventoried
as genuine for occurrences, not fabricated `.len()` calls. Replay rejects
missing/duplicate rows, changed receivers/operations, forged source kinds and
orphan lengths. Existing `.len()` source-call obligations remain unchanged.

### Immediate fixed-array `.iter()` (2026-10-20)

`for item in array.iter()` now uses the same shared element-place loop for an
exactly typed fixed array, including non-Copy nominal enum elements. Ordinary
method selection runs first: a real in-scope array `iter` method retains its
own return type and source-call evidence. The builtin candidate requires exactly
`.iter()` with no arguments or type arguments. Owned arrays, shared/mutable
array references, and the existing checked temporary-borrow transport retain
their normal initialization, authority and lifetime checks. A mutable array
reference still yields shared items, never mutable ones.

The immediate loop consumes this builtin operation directly rather than
inventing a first-class iterator type. It evaluates the receiver once, keeps
temporary storage alive through the loop, and cannot extend an escaping item
loan. Its original method occurrence remains inventoried and is bound to the
exact loop length receiver and retained shared-borrow shape. Replay rejects
omitted/duplicate rows, changed loop or source identity, and replaced borrows.
First-class array iterator values, `.iter_mut()`, slice `.iter()` and arbitrary
adapter chains are not added by this slice.

## Runtime place authority

`Projection::DynamicIndex(LocalId)` is a typed compiler Place projection. Its
index must be an existing `usize` local, checked in both checked IR and CFG.
The projection is not admitted in source pattern decisions. The projected
element type is explicitly restricted to the scalar/unit capability above.

The loop saves the next index before advancing its private cursor. Each item
uses the existing checked `Borrow`, CFG `LoanSite` and `Rvalue::Borrow`, with a
place rooted through the saved array reference. CFG lowering emits explicit
array-prefix and index inspection. Ownership independently checks the index
read at borrow creation, including when CFG is inspected without the lowering
helper. No pointer cast, element copy, synthetic constant index, side-slot
result, or out-of-graph fact grants the loan.

Native borrow creation reads the full unsigned index, checks bounds before
scaling or address formation, and stores the selected address in the reference.
The private array element stride is unchanged. Subsequent index-local mutation
or cursor advancement cannot retarget that already materialized reference.
Unit elements and unused reference bindings still perform bounds checks.
Runtime slice lengths are checked even for empty/unit slices. Dynamic address
scratch preserves store RHS values rather than clobbering the saved value.

## Conservative ownership

Any constant or runtime array index overlaps the entire array, as before for
ordinary indexed loans. Unequal index values or LocalIds never prove loan
disjointness. A runtime element's initialization footprint is the entire array.
Every read/borrow requires that footprint initialized. Writes require it already
initialized and do not establish initialization or variant proofs for a guessed
element. A retained element reference carries the ordinary canonical loan,
parent authority, lifetime and last-use obligations.

Runtime slice elements cannot be enumerated into fixed declaration input paths.
Their declared lifetime/kind-qualified input envelopes therefore supply result
origin authority, as for other abstract referents. Actual returned body loans
must still originate from that genuine parameter, never owned local storage.
This does not change fixed aggregate path derivation, mutable authority, or
source admission of slice descriptor return types.

Break, continue, labels and return use the existing checked loop/match CFG and
cleanup authority. Body-local destructors run on their actual control-flow exit,
not at every possible dispatch arm. Empty arrays do not fabricate a reference or
turn their source bodies into unchecked dead syntax.

## Explicit open boundaries

- Mutable slice descriptors and slice descriptor return types.
- General structured, reference-carrier and Drop-bearing referenced elements
  beyond the ordinary place/borrow paths exercised by the nominal enum slice.
- Existing SCC-born loan values retained across a loop backedge. Such Rust-valid
  cases remain explicitly Unsupported rather than relaxing the borrow solver.
  An item can escape via a terminating break when ordinary lifetime checks prove
  it valid, and last use can release the array for mutation afterward.
- Full source-library IntoIterator activation and general iterator adapters.
- Non-Copy by-value arrays and expression-position for.

## Validation

`tests/loops.rs` runs stock rustc first and native check/build plus actual ELF
execution with one and eight graph workers. Coverage includes real mutation,
once-only iterable effects, nested labeled continue, break, return, narrow/wide
scalar storage, source call/binding identity on 40 elements, reference escape via
break, last use, unit and empty arrays, and exact destructor output on control
exits. Invalid controls include shared mutation, mutable alias conflicts,
immutable sources, escaped frame references, consumed mutable reference sources,
body scope and empty-body mutability/type obligations. Rust-valid unsupported
boundaries have separate stock-positive rejection controls.

Internal kernel controls additionally check invalid/non-usize index locals,
uninitialized index and whole-array reads, conservative overlap of unequal
indices, runtime bounds at 0/length/usize::MAX for scalar and unit arrays, and
creation-time address capture before index mutation. These typed fixture
mutations test authority/backend behavior, not source-language compatibility.
Shared slice controls additionally cover parameter-origin returned elements,
descriptor-local scope ending before item use, root mutation conflicts, distinct
input lifetimes, owned-local escapes, temporary slice call arguments, once-only
descriptor evaluation, and bounds on empty/scalar/unit slice descriptors.
Graph-integrity ratchets are unchanged. No performance claim follows from this
milestone.

`tests/iterable_literals.rs` additionally compares unchanged
`08_enum_state_machine.rs` and its minimized nominal-array loop with stock at
workers 1 and 8, including CHECK without codegen, BUILD and actual execution.
Focused controls cover custom method precedence, shared/mutable array reference
receivers, once-only temporary evaluation, illegal shared mutation, conflicting
owner writes and escaping temporary element loans. Internal iterator-source
tests mutate original source correspondence and borrow receipts. All 14 graph
integrity gates remain mandatory and passed for this integration.
