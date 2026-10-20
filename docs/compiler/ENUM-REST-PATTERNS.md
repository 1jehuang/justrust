# Enum tuple-payload positional rest

Status: **bounded native slice accepted on October 17, 2026**. Full Rust remains open.

## Genuine enum authority

Reuse the actual `VariantRest { name, fields: PositionalRest }` source node and
borrowed checked prefix/suffix projection. Resolve the real canonical enum
constructor use with its original-use certificate, exact source Enum declaration,
actual variant and tuple-payload shape. Map against that variant's payload arity,
not the enum's variant count or the selected child count. Wrong payload shape or
excess children are ordinary source errors. No omitted pattern or binder is
manufactured.

Match output remains genuine `MatchPattern::EnumVariant` with exact enum type and
variant. Every selected binding retains its actual source occurrence and maps
through the actual variant Downcast followed by the physical Field index.
Variant keys remain variant keys, never aggregate catchalls. Existing tag tests,
variant coverage, exhaustive-match checks, dead-arm validation and refinement
invalidation remain authoritative. No new AST, IR format or private ABI is needed.

## Bounded admission

Admit enum tuple-payload positional rest in existing match patterns and existing
single-variant irrefutable let patterns. Multi-variant let remains refutable and
unsupported in this slice. Selected child patterns must satisfy existing
irrefutability rules; no new literal/refutable payload matrix, guards, or-patterns,
reference ergonomics, arrays/slices or tail-at-rest binding is admitted.
Preserve existing already-supported nested single-variant behavior rather than
silently regressing it.

Generic enum applications use only existing exact nominal instantiation and
expected-domain machinery. A Never domain cannot invent unknown generic actuals.
Tuple-shaped zero-payload variants are admitted only where the existing enum
schema supports them and stock-first evidence confirms the source. Unit and
record variants do not become tuple variants through parentheses.

Enum/type/variant visibility uses existing enum authority, not tuple-struct
all-field constructor restrictions. Every selected payload type is checked.
Omitted payload positions produce no binding, load or move. Rest-only matches
still inspect actual tags when required, but do not eagerly consume the payload
or whole value. Scrutinee evaluation remains once-only.

## Resource and acceptance gates

Charge checked explicit-child work and selected-plan storage before traversal or
allocation. Keep all count/depth/work/storage limits unchanged. Source walkers,
lexical binders, inventory, clone and replay preserve written prefix/suffix order
and existing source identities; physical payload indices belong to checked plans.

Require stock-first exact sources, workers1/8 native CHECK/BUILD with no compiler
fallback, exact executable output, bootstrap debug/release, real nominal
owner/variant/downcast authority controls, complete hosted tests, strict lint,
formatting and diff gates. Retain exact failed and passing redacted receipts.
No full-Rust, performance, public layout or ABI claim follows from this subset.

## Observed acceptance

The exact stock-first corpus has 24 sources: 19 stock-valid and five
stock-invalid. Native outcomes are 16 runtime, four ordinary-invalid, three
stock-valid Unsupported and one stock-invalid/native-Unsupported capability
boundary. The last is a refutable multi-variant let, stock E0005, not a claim of
ordinary native semantic rejection. Workers1/8 CHECK/BUILD exercise 96 cells and
32 actual native executable images per bootstrap profile.

Debug and release each passed 35 focused tests, including 27 public authority
controls built from actual parsed enum owners/variants. These validate genuine
downcasts, physical suffix fields, exact nominal and variant identity, dead-arm
metadata, local destinations and foreign source occurrences. The old positional
corpus retains all 24 exact sources and 96 cells, with only its newly supported
enum-rest outcome promoted. An older payload-enum source likewise retains its
exact bytes and now executes its existing stdout oracle.

The complete hosted suite passed 2,348 tests across 118 binaries, no failed,
ignored or filtered tests (`20261017-113206605-1891781`). Strict lint passed
`20261017-113222355-1892877`; final release35 passed
`20261017-113239745-1894112`. Formatting and diff checks passed.

Selected rest plans own an actual-capacity-charged Temporary BudgetVec through
both lowering consumers. The real denial control passed
`20261017-113202973-1891536`: the same production selection helper rejects its
first backing growth and releases temporary storage after poison/drop. An
earlier whole-entry test was correctly stopped by a transient canonical-path
metadata allocation before reaching that growth. Its failure is retained, not
relabelled as backing-allocation coverage. The adjacent whole-entry success
control verifies the selection lease survives borrowed consumption.

Seventeen exact failed/passing redacted receipts are retained in
`data/runs/xps14/2026-10-17-enum-rest-patterns.jsonl`. No source witness, resource
cap, stack setting or fallback requirement was weakened to obtain acceptance.
