# Stored closure parameter patterns

Status: accepted bounded closure parameter-pattern slice. Full Rust, Iterator and
reference-result compatibility remain open.

## Observed evidence

Frozen parent `9cc255298c0742dfd63cc4b8526295fa815028ba`, tree
`439a9f52e7ac691bc08576602fbd6001df8a8273`: hosted task `5933909x3t`
passed `check --tests`, an explicit fresh CLI build and the actual
`let f=|&&x|x+1` stock/native consumer. Receipt
`20261019-161409844-1001443` exercises CHECK and BUILD at workers 1 and 8,
including CHECK no-object/no-image assertions and successful BUILD execution.

The single semantic CHECK of unchanged corpus 11 with that fresh binary now
reaches `unsupported: associated equalities are supported only in function and
impl bounds`. The old `&&` parameter parser barrier is removed, but the program
has not passed. The source fixture hash is
`8300a2b141cc286d8d48926f758a7efcd6b3c792a3ef6eccf032375df2e9bc19`.
Source-pattern publication, checked binder/local identity and two-Deref path
forgeries reject in hosted receipt `20261019-162802320-1053045`.
During the verified hosted disk outage, coordinator-authorized LOCAL receipt
`20261019-163548850-1079249`, pinned to `nightly-2026-10-08`, passed six
invalid sources through CHECK and BUILD at workers 1 and 8: non-Copy shared
extraction, immutable binding writes, reference-kind mutability, refutable
parameters, wrong arity and wrong reference depth. Its duplicate-parameter test
stopped at the stock oracle because the expected error was E0416 rather than the
actual E0415. Only that diagnostic expectation was corrected, with source and
native rejection assertions unchanged. The corrected duplicate case subsequently
passed in LOCAL receipt `20261019-164853955-1126013`. The two additional positives
(tuple/wildcard shadowing with argument evaluation order, and mutable binding)
passed in `20261019-164855630-1126131`, with explicit CHECK/BUILD/run at workers
1 and 8. All 14 unchanged graph-integrity gates passed in
`20261019-165102539-1132582`. These final runs used the coordinator-authorized
LOCAL outage fallback with the same pinned `nightly-2026-10-08` toolchain.
Transport-interrupted runs are not counted as successful evidence.

Current-parent integration on `553f1adeab4c965acd1c01cc499ee08c06493048`
passed all-test compilation, the exact source/binder/projection authority unit
(`20261019-165833453-1162956`) and all 14 graph gates
(`20261019-165932521-1169837`). This final integration used the same authorized
LOCAL pinned-nightly fallback. Parent constant inference, generated metadata and
opaque specialization paths are preserved, not replaced by the feature patch.

## Frozen contract

Each stored closure parameter retains its original source `Pattern` and type
annotation. Bindings come from that pattern's authenticated source identifiers
and lexical scopes. No textual token rewrite, invented source name or flattened
identifier-only parameter list may stand in for the original pattern.

The first executable target is an explicit shared-reference pattern `|&&x|`:
entry destructuring must read the real argument tuple field, dereference exactly
the two retained shared reference layers and copy an eligible scalar. Moving a
non-Copy value through a shared reference must reject under ordinary move/borrow
analysis, even in an unused closure. Parameter bindings are installed together,
with duplicate names rejected and source shadowing retained.

Existing irrefutable binding, wildcard and tuple patterns may reuse the same
ordinary pattern checker. Refutable patterns must reject, not become unchecked
entry assumptions. Match ergonomics, `ref`/`ref mut`, mutability and dereference
semantics remain those of the existing checked pattern implementation.
Unsupported existing pattern domains remain explicit boundaries.

## IR and graph shape

There is no new IR variant or graph producer. The parent checker extracts each
real parameter from the adapter argument tuple and emits ordinary `TypedStmt::Let`
for simple bindings or `TypedStmt::LetPattern` for checked destructuring. The
canonical local IDs and exact Deref/Field projections pass the existing IR, CFG,
move, borrow and immutable closure-shape publication validators.

Signature inference constrains the parameter's complete type through the retained
pattern, rather than assigning the argument type directly to each bound leaf.
Source resolution and macro lexical registration visit every pattern binder.

## Acceptance gates

Stock/native explicit CHECK and BUILD at workers 1 and 8, with successful BUILD
execution and CHECK no-object/no-image assertions: nested shared references,
mutability, tuple/wildcard binding, shadowing and evaluation order. Invalid unused
closures cover non-Copy extraction through a shared reference, mismatched
reference depth, duplicate bindings and refutable parameters. Forged retained
source patterns/binders or final checked projections must reject through original
source and final closure-shape correspondence. Mandatory graph-integrity gates
remain unchanged.

Once the focused `|&&x|` consumer passes, probe unchanged
`tests/fixtures/full_programs/11_closures_iterators.rs` CHECK once and record its
actual next boundary. This slice does not claim complete Iterator support,
reference-result transport, early closure layout, or full Rust compatibility.
