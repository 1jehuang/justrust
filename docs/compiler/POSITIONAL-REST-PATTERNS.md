# Positional rest pattern contract

Status: **bounded native slice accepted on October 17, 2026**. Full Rust remains open.

## Honest source representation

Keep existing no-rest Tuple and Variant source patterns unchanged. Add
`TupleRest(PositionalRest)` and `VariantRest { name, fields: PositionalRest }`,
where PositionalRest owns only the actual written prefix and suffix patterns.
The representation contains one syntactic `..`, not manufactured wildcard
children or numeric field identifiers. Parse duplicate rest as syntax errors.
Every selected binder retains its actual checked occurrence and lexical identity.

A borrowed zero-allocation projection helper checks prefix+suffix against actual
arity with checked subtraction/comparison and yields prefix indices followed by
suffix indices in written order. It grants no nominal, visibility or source
publication authority. Consumers must charge work and validate exact expected
structural type or source schema first.

## Bounded admission

Admit existing nonempty structural tuples and nonempty nongeneric tuple structs
in irrefutable let and match patterns. Match trees remain recursively irrefutable.
Exact source declaration/shape, real field arity and nominal compatibility remain
mandatory. Tuple-struct constructor visibility includes all source field
restrictions, even omitted private fields, unlike named record-rest patterns.
Selected fields also retain their individual visibility/type checks.

No new enum tuple-payload rest, empty/unit tuple, array/slice patterns, tail-at-rest
bindings, reference ergonomics, guards/or-patterns or refutable aggregate leaves
are admitted. Existing enum record-rest behavior remains unchanged.

Only selected children produce binding projections, loads and moves. Omitted
positions produce none. Plain Field plans reuse checked Aggregate IR and existing
CFG/ownership semantics. Whole-value and moved-selected reuse must still reject;
omitted non-Copy siblings remain usable. Scrutinee effects occur exactly once.

## Resource and publication contract

All source walkers, name/provenance validation, scoped binding registration,
canonical inventory, clone preflight and replay visit actual prefix then suffix
children only. Both child vector allocations are admitted before cloning. No
omitted binding or constructor callable identity is minted. Existing count,
depth, work and storage caps stay unchanged. A Never scrutinee cannot invent an
unknown structural tuple arity from an omitted range.

## Required acceptance

Stock-first sources, native workers1/8 CHECK/BUILD with unavailable compiler
fallback and exact executable outputs, bootstrap debug/release gates, public
malformed source/owner/nominal controls and complete unchanged hosted regression,
strict lint, formatting and diff gates are required. Retain exact failed and
passing redacted run summaries. No full-Rust, ABI/layout or speed claim follows.

## Observed acceptance

The stock-first corpus contains 24 exact sources: 17 stock-valid and seven
stock-invalid. Native classification is 12 runtime, seven ordinary-invalid and
five explicit Unsupported cases, exercised in 96 workers1/8 CHECK/BUILD cells.
Bootstrap debug and release each passed all 26 focused tests, including 14 public
owner/nominal/source-shape controls. Each profile executed 24 native images.
Existing aggregate and record corpora retain their exact source bytes/order;
only three newly supported positional-rest outcomes were promoted. Three older
parser/native fixtures likewise retain their exact source while becoming
positive checks instead of obsolete rejection expectations.

The complete hosted suite passed 2,301 tests across 116 binaries with no failed,
ignored or filtered tests (`20261017-111852910-1831807`). Strict all-target lint
passed (`20261017-111906233-1832566`), as did formatting and diff checks. Debug
and release focused receipts are `20261017-111621737-1820635` and
`20261017-111627923-1821157`. Ten exact redacted failed/passing receipts live in
`data/runs/xps14/2026-10-17-positional-rest-patterns.jsonl`.

The failed gates are retained: incomplete integration, obsolete rejection
fixtures, actual resolver-versus-checker phase classification, and the legal
empty/unit tuple boundary. The latter remains typed Unsupported, not newly
admitted semantics. No cap, stack setting, fallback compiler or source witness
was weakened to obtain the final result.
