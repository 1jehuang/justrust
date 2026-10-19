# Closure callable foundation

## Acceptance coverage correction

The general `tests/support/oracle.rs` historically compared stock with one default
native BUILD/run. Earlier descriptions of its value-capture and no-call Drop
cases as worker-1/8 CHECK/BUILD coverage were incorrect. The dedicated
`tests/support/closure_oracle.rs` now explicitly drives CHECK and BUILD at workers
1 and 8, asserts CHECK demands no code generation and publishes no image, and
runs each successful BUILD. Invalid controls exercise both roots and worker
counts and may not substitute an unsupported boundary for rejection. Stage-one
explicit graph/worker tests and independently explicit callable-hint tests retain
their original coverage. The corrected capture matrix covers 31 retained stock
programs with 124 explicit native CHECK/BUILD cells. Its first successful
worker-1/8 execution is new evidence, not a retroactive description of the old
oracle runs.

## Canonical method source owners

Stored closures in implementation bodies retain their exact implementation/item
ordinal, and trait default bodies retain their trait/item ordinal. These resolve
to the registered callable declaration, not a free function with the same method
spelling. The resolver publishes immutable closure syntax only after all existing
free/default/implementation body rewrites finish. Placeholder adapter bodies cannot
overwrite already resolved default syntax. Ownership replay checks the original
publication, canonical ordinal and occurrence in that selected body's retained
syntax, including nested stored syntax. Foreign publications and wrong owner
ordinals reject.

`tests/closure_method_parents.rs` exercises ordinary associated functions, shared
self captures, equal-spelling methods in distinct implementations and unused bad
closure bodies with explicit worker-1/8 CHECK/BUILD/run. Generic implementation
and trait-default source ownership have metadata controls, but their executable
closure bodies still require the unfinished inherited provider environment.
This source ownership fix does not complete generic-parent closure support.

## Stored closure support

Stored closures can borrow shared and mutable scalar locals and owned aggregate
field places. The parent checker
infers the callable capability and final per-field capture layout, then publishes
only immutable, mode-specific checked bodies. Disjoint struct fields retain
distinct capture identities. Environment construction creates ordinary loans,
and rewritten bodies pass the same move, borrow, escape and Drop checks as other
functions. `tests/closure_borrows.rs` and `tests/closure_capture_places.rs` compare
CHECK/BUILD/run at workers 1 and 8, including unused invalid bodies, conflicting
loans, mutable callable admission, escaping locals and destructor order.

Inferred closure result hints do not create source-written coercion sites.
Reference-bearing closure results, including nested tuple/array carriers, remain
explicitly unsupported until their exact checked input-origin contracts can be
replayed through callable projection. This is not a successful reference-return
or opaque-return milestone. Generic parent bodies and opaque returns remain open.
Reference-rooted dereference precision, unique-immutable capture capability, and
raw/owning-pointer mutation classification are separate unfinished work. This
increment does not claim all reference-containing environment forms.

No-call stored closure bodies retain canonical CFG lowering authority even when
unused capture-check scratch locals have owning Drop obligations. The lowering
entry reads the exact checked parent-body allocation and validates owner, site,
provider, receiver mode and signature before capturing the actual CFG shape.
Foreign bodies, metadata, modes and modified shapes reject. This does not admit
new capture modes or relax destructor checks (`tests/closure_drop_proof.rs`).

Stored closures now retain immutable parent-body capture environments. Value and
`move` captures support direct calls and generic `Fn`/`FnOnce` bounds, with
mode-specific concrete receiver bodies and ordinary move/drop checking. Capture
source places, owner instantiation, site, provider and receiver capability are
part of executable authorization. `tests/closure_captures.rs` compares valid
execution and invalid move reuse against stock Rust at workers 1 and 8.
Closures in generic parents and opaque returns remain unsupported by this
increment. This is not complete closure support.

This is not full closure support or full Rust compatibility.
The mutable scalar increment below is a private checkpoint with scoped replay
passed on published `677ea96ab`, pending coordinator-composed full-suite
acceptance. Its scoped and infrastructure receipts are in
`data/runs/closure-mutable/README.md`.

## Historical immediate-closure foundation

The following foundation notes describe the earlier immediate-call subset. Its
listed unsupported stored-value forms are superseded by the current contract
above, not a claim that the complete closure program is finished.

The first genuine executable increment admits immediately invoked,
noncapturing closures with explicit parameter types, an explicit scalar/unit
return type and a braced body, for example:

```rust
fn main() {
    let answer = (|x: i32| -> i32 { x + 1 })(41);
    assert_eq!(answer, 42);
}
```

The parser retains each body as a separate anonymous `Function` declaration.
Its unnameable identity is deterministic in source traversal order and retains
its actual source-site span and module. Anonymous declarations are not inserted
into module or import lookup. Calls use the existing checked-call IR, including
ordered once-only argument evaluation. The normal per-declaration graph nodes
universally check body typing, CFG, moves and loans, even when the enclosing
source function is unused. There is no body substitution, shared result slot
or relaxed graph-integrity ratchet.

The next executable increment admits shared scalar captures of explicitly typed
parameters of the immediate parent free function (including an anonymous
parent's explicit parameters):

```rust
fn run(n: i32) -> i32 {
    (|x: i32| -> i32 { n + x })(2)
}
fn main() { assert_eq!(run(40), 42); }
```

Per-owner `BODY_LEXICAL` (116) and per-source-site `CAPTURE_ENV` (117) graph
facts retain original parameter binding occurrences, declaration/snapshot
identity, first-use field order and scalar layout. They contain immutable data,
not compiler contexts or mutable caches. Their dependencies are acyclic:
capture layout reads the owner's lexical fact and signatures, never its checked
body. Both the parent and anonymous checked bodies consume the layout handle.

The immediate-call ABI flattens the environment into private ordinary reference
parameters with the exact shared or mutable capture mode. Each real loan is
created once, before explicit call
arguments, and each capture read dereferences the checked field parameter.
This does not add general references-in-aggregates support. Normal ownership
checking rejects writes and mutable borrows of a captured original parameter
while arguments evaluate, and admits modification after the immediate call.
Repeated uses deduplicate the original binding. Unused captured bodies are
independently checked for type errors and non-Copy argument move reuse.

The next executable increment also admits mutable scalar parent-parameter
captures, using the existing noescape reference-call authority:

```rust
fn run(mut n: i32) -> i32 {
    (|| -> () { n += 2; })();
    n
}
fn main() { assert_eq!(run(40), 42); }
```

Each exact original parameter binding has one field, in source first-use order.
Reading first and writing later upgrades that same field to mutable without
reordering or duplication. Assignments, compound assignments and explicit
`&mut` uses demand mutable mode. Hidden parameter types and construction loans
use that final strongest mode. The hidden carrier binding stays immutable:
mutation is through its genuine `&mut` referent, never through copied scalar
storage. The parent independently checks construction against its original
binding mutability, even when the enclosing function is unused. Loans are
constructed before explicit argument evaluation, so conflicting reads, shared
borrows and assignments in arguments reject. Immediate-call last use releases
these loans normally. No reference-containing aggregate or region authority gate
is changed. Returned-reference capture lifetime propagation remains unsupported.

Immutable facts 116/117 additionally retain the exact loan mode and verify it
against the hidden parameter signature. Generated bodies still receive separate
110/112 checking, including their own move/reborrow and overlapping-loan errors.
`tests/closures.rs` freezes fifteen mutable-capture whole programs, checking
stock classification plus native CHECK/BUILD/run at workers 1/8. A separately
measured two-program nested probe additionally checks shared-to-mutable mode
upgrade of an anonymous parent's explicit scalar parameter, including when that
parent already carries a hidden shared capture field. Transitive hidden-field
captures and original reference-typed parameter captures remain explicit native
boundaries, with stock-valid executable controls at workers 1/8 CHECK/BUILD.
`tests/closures_authority.rs` adds unused-body move/alias/write controls and the
private graph-fact unit test rejects both forged mode upgrades and downgrades.
`tests/closures_graph.rs` requires per-owner/per-site immutable facts, separate
checked bodies, stable worker-independent keys and no codegen demand from CHECK.

Stored closure values, local/inferred binding captures, move captures,
parameter/result inference, non-scalar returns, generic enclosing bodies and
closures originating in macro expansions remain unsupported. Shadowed parent
parameters explicitly reject rather than being rebound by spelling. Unqualified calls
inside a closure body are deliberately unsupported until capture-aware callable
resolution exists. Use an explicit `crate::function` path. Unqualified free
value references reject before constructor lookup, so a captured local cannot
silently become a same-spelled module constructor.

`tests/closures.rs` compares whole programs against stock Rust for native calls,
multiple anonymous identities, nesting, early returns, argument order, mutable
reference parameters and non-Copy arguments. Negative controls cover invalid
unused bodies, move reuse and loan conflicts. These reference parameters are
not closure captures. Shared-capture programs additionally exercise field order,
deduplication, nesting, last use and call-argument loan conflicts. Additional tests inspect distinct per-callable graph
keys/source anchors and bound closure nesting to 16 live frames, independently
of the number of sibling closures. The ordinary parser depth gate is unchanged.

Remaining increments include general typed environments with local and
move fields, stored closure-local callable identity and per-call local storage,
and returned-reference captured-loan lifetime propagation. Generic `Fn`/`FnMut`/`FnOnce` bounds and the
full closures/iterators program remain open. This milestone makes no speed
claim and does not promote the frozen full-program closure fixture.
