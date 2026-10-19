# Generated item metadata and derives

Status: bounded unit-struct implementation validated. This is not full derive
support or acceptance of unchanged time-core.

## Frozen supported slice

Genuine item-macro output may carry outer `doc = <string>`, `doc(hidden)`, and
string-valued built-in `concat!` / `stringify!` documentation expressions.
Built-ins are selected through the existing checked lexical and module macro
lookup. Metadata retains its original ordered token spans, expansion admission,
original module and exact item-owner occurrence. The frozen scope arena seals
that association. Same-expansion co-membership alone is not authority.

The first derive emitter is `Copy` for nongeneric unit structs without lifetime
parameters or bounds. It emits an ordinary checked `ImplDecl` directly from the
original parsed nominal declaration, without printing/relexing expanded input,
fabricating identifiers, or changing the nominal type. Canonical `Copy`/`Clone`
obligations, coherence and `Drop` conflicts remain normal universal compiler
checks, including unused implementations. A parser-local append-only record
seals the original derive occurrence, metadata owner and nominal declaration.
Published records are immutable arrays; emitted impl shape and original nominal
occurrences are replayed before semantic use.

The unit `Clone` emitter uses a real ordinary shared-receiver method returning
an actual unit constructor. Derived method and receiver identifiers are explicitly
synthetic, not attributed to unrelated original tokens. A closed immutable member
admission binds their metadata, derive occurrence and nominal declaration.

The unit `Debug` recipe remains immutable source data until canonical resolution
visits the genuine embedded `fmt::Debug` declaration. That exact resolved source
method supplies the formatter and result types. Materialization emits an ordinary
source-trait implementation whose real body calls the formatter's `write_str`
with the unit type's actual name. No empty implementation or placeholder signature
is published. Original pending callable coverage and final member/signature/body
recipes are replayed separately.

Generic, record, tuple and enum derive recipes remain explicitly unsupported until
their actual stock bound and body generation is implemented. Unknown semantic
attributes and shadowed/noncanonical derive selections are never discarded.

## Acceptance gates

- Compile all test source types with `justrust check --tests`.
- Exact owner, ordered-span, foreign-world, nominal and impl-transplant controls.
- Stock-differential original-source generated documentation and unit `Copy`.
- Native CHECK without image and BUILD/run at workers 1 and 8.
- Invalid unused `Copy` obligations, duplicate impls and `Drop` conflicts.
- Explicit unsupported generic/field/other-derive and active-attribute controls.
- Mandatory unchanged graph-integrity gate before publication.

The direct Copy plus attachment checkpoint `2ee460` passed hosted check and two
exact metadata units. Combined emitter checkpoint `66abe253` passed hosted
`check --tests`. Actual consumer feedback corrected three genuine emission
boundaries: separate receiver binding, canonical source-signature occurrence
scope, and restricted trait-implementation method visibility. No compiler
validation gate was loosened.

Checkpoint `f07dbf2b117ad63bb577d8401b163db5c92fcddd` passed the original generated
`#[doc = concat!(...)] #[derive(Debug, Clone, Copy)]` unit consumer against stock,
then native CHECK (no image) and BUILD/run at workers 1 and 8, with exact output
`Unit Unit Unit\n`. Hosted receipt: `20261019-162555761-1046299`.
The canonical source signature is replayed against the exact derived member before
visiting its type occurrences in the original source module. Generated parameter
bindings and executable body names retain their own strict expansion domain.
Current-parent checkpoint `0db6eeba893c51ec2650e01ea4a2d58e6a0e5e90` on
`e074bf8687576a9f2db35826199c8517b9c672f1` passes four metadata/provenance units,
the unchanged fourteen-test graph-integrity gate, and the same stock/native
combined consumer with explicit CHECK/BUILD/run at workers 1 and 8. During a
verified hosted outage these gates ran locally with the exact matching pinned
`nightly-2026-10-08`, rustc `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`.
Receipts: `20261019-163753566-1087386`, `20261019-163819596-1088937`, and
`20261019-164203474-1101014`. The two new units use genuine embedded source
assembly, reject same-expansion scope/member transplants and foreign snapshots,
and ensure source-signature authority cannot authorize executable body names.
No unchanged time-core acceptance is claimed.
