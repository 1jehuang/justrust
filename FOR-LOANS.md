# Referenced-array for-loop loans

Status: bounded language milestone, not general Iterator or slice support.

## Admitted source

Statement-position `for item in &array` and `for item in &mut array` use real
shared/mutable element references for fixed arrays of integer, float, char,
bool, or unit elements. Existing array-length limits apply, including empty
arrays. There is no special 32-element ceiling. Explicit reference-valued
array sources, source calls, nested control flow and ordinary source-body
bindings/calls use the same machinery. The iterable expression is evaluated
once before the new binder and loop frame become available.

The source body is checked and emitted once. A discarded bounded-dispatch
experiment duplicated bodies and conflicted with original source call/binding
identity. Its local checkpoint is retained in history, but no dispatch or
source-capture exception remains in the implementation.

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

## Conservative ownership

Any constant or runtime array index overlaps the entire array, as before for
ordinary indexed loans. Unequal index values or LocalIds never prove loan
disjointness. A runtime element's initialization footprint is the entire array.
Every read/borrow requires that footprint initialized. Writes require it already
initialized and do not establish initialization or variant proofs for a guessed
element. A retained element reference carries the ordinary canonical loan,
parent authority, lifetime and last-use obligations.

Break, continue, labels and return use the existing checked loop/match CFG and
cleanup authority. Body-local destructors run on their actual control-flow exit,
not at every possible dispatch arm. Empty arrays do not fabricate a reference or
turn their source bodies into unchecked dead syntax.

## Explicit open boundaries

- Runtime slice iteration and slice element-place/loan authority.
- Structured, enum, reference-carrier and Drop-bearing referenced elements.
- Existing SCC-born loan values retained across a loop backedge. Such Rust-valid
  cases remain explicitly Unsupported rather than relaxing the borrow solver.
  An item can escape via a terminating break when ordinary lifetime checks prove
  it valid, and last use can release the array for mutation afterward.
- General source IntoIterator desugaring and iterator adapters.
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
Graph-integrity ratchets are unchanged. No performance claim follows from this
milestone.
