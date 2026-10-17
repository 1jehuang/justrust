# Positional rest pattern contract

Status: **implementation in progress, not accepted**. Full Rust remains open.

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
