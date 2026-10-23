# Projected required methods

Status: bounded required-method vertical implemented on October 17, 2026; generic methods and local source-recipe integration extended October 23. Full Rust and general associated-type support remain open.

## Admitted vertical

Extend ASSOCIATED-TYPES.md to value-oriented required trait method signatures containing associated projections, their exact owning-impl conformance and bodies, and closed method/UFCS calls. Admit existing Sized reference-free associated normal forms, including non-Copy records, tuples, wide scalars and generic impl RHS parameters. Do not implement a scalar whitelist or invent Copy permission.

The first measured core is stock sources 01 through 06 in the projected-method stock corpus: the existing associated-only baseline, a required get returning Self::Out, distinct applications of one trait on one Self, a moved non-Copy result, tuple/i128 results and a generic Wrap<T> implementation. Every implementation body is checked universally under its actual impl binders, even when unused.

The first native gate found that the original core programs also use `panic!`, which the existing source parser does not admit. Keep all 18 original sources unchanged and report that earlier boundary honestly. Separately measured appended programs use positive conditional checks and print the success literal only after every original payload condition holds. Their executable evidence tests this milestone without pretending to implement panic expansion or replacing the original witnesses.

The October 17 boundary excluded rigid caller projections, projected defaults and explicit associated bounds. Those original witnesses remain unchanged and are now exercised as native acceptance cases. GATs and reference-bearing projected method forms remain separate boundaries. A favorable closed callsite does not authorize an invalid universal body.

## Generic method extension (2026-10-23)

Generic trait methods now substitute the selected owner inside projected inference
terms before comparing argument and expected-result constraints. Raw method
signatures are independently checked against the simultaneous owner/method
substitution before closed projection normalization, rather than comparing a
normalized ABI to an unreduced source projection. Generic required-versus-provided
conformance likewise reduces only the exact owning implementation's associated
assignments after alpha-renaming. This does not treat membership in a foreign
application as an assignment or relax the residual executable-projection guard.

`generic_method_owner_projections_match_stock` in `tests/iterator_fold.rs` covers
projected arguments/results, distinct generic owner and method binders, an invalid
unused implementation and an invalid unused result assignment at workers 1/8 in
CHECK/BUILD/run. The existing iterator consumer matrix exercises `find`, including
non-fused stopping and invalid predicate results. Unsupported residual projected
ABIs, general associated-type support and full Rust compatibility remain open.

Local projected annotations now enter the nominal demand system through exact
retained-occurrence authentication. Receiver and argument demands must be read
before normalization under the validated caller. Normalized nominal results
still demand their graph realization instead of bypassing it. A same-owner
member-transplant test rejects forged occurrences through this ingress too.
When projection normalization introduces return-reference layers without source
lifetime authority, it reports Unsupported rather than inventing an origin.

## Native collection extension (2026-10-23)

`Iterator::collect` and `FromIterator` are ordinary prelude source methods.
`Vec<A>`, `String` from `char`, and `String` from `&char` use source loops;
there is no compiler-recognized collection spelling. A source `CollectInput`
adapter supplies `IntoIterator` without an overlapping blanket implementation.
Selected generic method results accept their contextual type, owner arguments
are substituted on both sides of associated equalities, and explicit caller
equalities retain their exact projection identity.

`tests/iterator_fold.rs` compares Unicode, empty, user-defined and mapped-range
collection with stock at workers 1/8 in CHECK/BUILD/run, plus incompatible element
rejection. Mapped collection required eliminating repeated canonical signature
and normalization proof reconstruction. Proof scratch stays local to one
validation computation, keyed by its existing world, facts and exact subject.
Every returned normalization dependency is still authenticated, and receipt
replay still checks the selected callee, source occurrence, conversion inventory,
current engine and original/mapped signature. No resource ceiling is increased.
The graph regression also exposed nominal declaration demands being issued only
by BUILD. CHECK now requests and awaits the same per-declaration facts, including
unused nominal membership checks, before publishing success.
The unchanged full-program corpus remains 9/20, with matrix ownership work,
additional iterator/library APIs and recursive layouts still open.

Validation: 1,781 library tests pass with one ignored; the combined collection,
projection, nominal-authority and graph suites pass 67 tests with one ignored.
All-target checking and lint succeed with existing warnings. Selected redacted
receipts are in `data/runs/xps14/2026-10-23-native-collection.jsonl`. The broader
`body_inference` suite still exposes separate underconstrained-generic diagnostics
and autoref-on-owned-call-temporary limitations; this is not full compatibility.

## Separate authority paths

A private owning-impl normalization context may be issued only after complete metadata, assignment-map and coherence checks. It binds the current world/stage, implementation DeclId, exact full impl TraitRef, real impl GenericEnv and original assignment pair/RHS. It is not public ValidatedFacts, an Implements proof or closed normalization evidence.

Substitute all required trait binders through the complete owner application before reduction. Reduce only a projection whose item owner and complete instantiated TraitRef match that exact impl application. Substitute the authoritative RHS with the same checked impl mapping. A remaining T is the original impl ParamId with its existing Sized, Copy and carrier rules, not an opaque fabricated parameter. Nested reductions must retain bounded cycle/dependency checks. Other unresolved projections stay unsupported.

Use this context for required-versus-provided signature comparison and owning-impl callable signature/body annotations. It must not authorize projections from another impl, merely same Self, a reordered argument list, a foreign world or a different associated item. Keep raw source/header callbacks immutable and verify their correspondence before normalization.

Closed calls use normal implementation selection and finalized-world normalization evidence. Assumption-only membership is not an assignment. Required signature normalization, direct target selection and resulting signature/carrier evidence must refer to the same exact trait application. Never borrow the implementation target's signature or topology as the required trait declaration's authority.

Direct calls to owning implementation methods must replay the associated normal-form domain after substituting actual impl arguments, even when normalization erased projections from the callable signature. Check every original assignment to cover body-local-only projections. A closed call must select the same implementation and exact arguments; a rigid call retains the genuine caller parameters. This restriction does not reject reference actuals in unrelated ordinary generic functions.

## Required signature and carrier staging

Raw projected required signatures are nonexecuting schemas. Do not derive a global empty origin or carrier schema for them. Retain their original declaration, source regions, full schema and member identity, and defer executable shape derivation until exact owner substitution or closed normalization.

Derive required topology from the normalized required signature and its original source-region contract. Validate reference/carrier refinement against the implementation only after both sides have independently established required and provided contracts. Generic impl results retain real parameter-carrier behavior. No projection becomes an empty carrier, ParamTail, Copy value or ABI word merely because it is syntactically associated.

Closed projected calls need a context-bound per-application required carrier view. It must retain normalization/provenance evidence and be checked against the exact original method, full TraitRef, caller facts and result signature. Ordinary declaration-only carrier lookup is insufficient for projected methods. Existing call/evaluation/move ordering and closed IR/CFG/backend residual guards remain unchanged.

## Source, resources and acceptance

Preserve exact source occurrence and owner certificates when admitting required signatures, supported defaults and short-name projections. Structural lowering uses node-local scratch. Source shapes describe demands, not type or region permissions.

All metadata, including dead and unused method bodies and every supplied type argument, is validated before capability rejection. Use existing work/depth/bytes limits, actual-capacity accounting and poisoned failure behavior. No cap increase, fake declaration, fake body, public proof constructor or new external compiler delegation.

### October 23 acceptance

The unchanged 24-source corpus now partitions into 17 native executable cases,
five ordinary invalid cases, one stock-valid Unsupported case and one stock-invalid
Unsupported case. Both remaining boundaries concern reference-bearing projected
signatures. All 96 CLI CHECK/BUILD cells pass at workers 1/8, including 34 native
executions with exact `ok\n` output. The 96-cell observed graph matrix passes and
checks source admission for every body, universal checking for generic bodies,
and absence of instance/codegen demand during CHECK. An overridden default is
checked universally but is not emitted as a reachable executable instance.

The full library suite passes 1,781 tests with one ignored. All-target lint passes
with existing warnings. The broader full-program corpus remains 9/20, not a
full-Rust completion. Selected redacted receipts are retained in
`data/runs/xps14/2026-10-23-generic-projected-methods.jsonl`.

### Historical October 17 measurements

Preserve all 18 measured stock source bytes/order. Stock rustc 1.98.1, edition2024 and panic=abort accepts 12 and rejects 6; every accepted program prints exact ok followed by LF after semantic payload checks. The stock result is not native acceptance. Six separately measured positive-check equivalents are appended without replacing those originals. The combined 24-source native partition is 6 executable, 3 ordinary-invalid, 12 stock-valid boundaries and 3 stock-invalid boundaries. Original cases 01-07 stop at the local panic macro, 12/13 at earlier Self-where syntax, 10/11 at associated bounds, and 17/18 at lifetime syntax. These do not establish projected-default, rigid-caller or reference-result support.

Acceptance passed: 96 native CLI cells and 96 observed all-body graph cells per profile, workers1/8 CHECK/BUILD, with 12 real ELF executions per profile; 19 public authority tests; debug and release public matrices; 35 selected release internal projection controls; all-target check, strict all-target lint and formatting. The full suite passes 2,896 tests across 140 binaries. Generic Wrap<T> bodies retain their original universal binders, and unused invalid implementations still fail before object generation.

The historical associated-type corpus retains every source byte and order, with its wrong projected method signature now classified as ordinary Invalid rather than Unsupported. Its separate required-schema authority witness is admitted as a nonexecuting schema, while its rigid caller witness remains Unsupported. Failed and passing receipts are retained in data/runs/xps14/2026-10-17-projected-methods.jsonl. No speed claim or general associated-type, Iterator or full Rust claim follows.
