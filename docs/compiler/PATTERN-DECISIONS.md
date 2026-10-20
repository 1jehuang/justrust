# Nested pattern decisions: ordered tests before moves

Status: accepted scoped native implementation on October 17, 2026. Full Rust
compatibility and full product-pattern usefulness/exhaustiveness remain open.

## Frozen scope

Reuse actual source Bool/Int and nested enum-variant patterns inside the already
supported tuple, record, tuple-struct, enum payload and nonempty fixed-array
shapes. Share one checked decision representation across match, if-let,
while-let and let-else. Ordinary let remains recursively irrefutable.
No source AST, synthetic binding, callable identity or source-body ID is added.
Guards, or/range/@ patterns, references, slices and implicit ergonomics remain
closed. Existing array selected-Copy and indexed non-Copy move fences remain.

## Checked IR and runtime authority

`MatchPattern::Decision { ty, tests, bindings }` retains an explicit actual domain.
Each `PatternTest { projection, kind }` has a Bool, IntValue or exact
`EnumVariant { ty, variant }` observation. Tests form a nonempty ordered
conjunction. Projection paths are unique and use only typed Field,
ConstantIndex and Downcast steps under existing count/depth/work limits.

Every Downcast in a test requires an EARLIER exact enum-tag test on its parent
prefix. Every binding downcast requires the corresponding successful tag test.
An enum owner/index is never inferred from field count, spelling or metadata
alone. All paths, locals and facts are checked even for unused/dead/Never rows.
Canonical identity, substitution and resource preflight include every test and
nominal owner, not just successful binding paths.

CFG evaluates the source once and tests in order. Any failed predicate selects
the real failure/next-row edge without consuming payloads. ALL tests finish
before ANY binding initialization or move. Scalar inspection temporaries are
live before branching so failure cleanup cannot kill not-yet-live storage.
Existing tag refinement, move validation and client-specific binding lifetimes
remain authoritative, including persistent let-else success bindings.

## Conservative source coverage

Coverage distinguishes catch-all, complete scalar/variant rows and predicates.
A refutable payload row does not cover its entire enum variant. Existing proven
irrefutable variant rows or a root catch-all are required for exhaustive match.
A known missing root variant remains invalid. Potentially exhaustive product
matrices that this slice cannot prove are explicitly Unsupported, not falsely
accepted or diagnosed nonexhaustive. Full usefulness/product-matrix analysis
remains open. Exact source scopes and real binding occurrence order are retained.

## Required acceptance

Stock-first exact sources must cover all four clients, nested tag prerequisites,
repeated variants, scalar widths, once-only sources, first/late predicate failure,
non-Copy no-move on failure, success-only moves and real loop cleanup.
Public controls must mutate real checked plans and reject missing/reordered or
foreign tag proofs, wrong typed paths/literals/locals, duplicate paths and dead
malformed rows at the actual IR/CFG/ownership boundary.
Native workers 1/8 CHECK/BUILD and executable output, debug/release bootstrap,
strict lint/formatting, unchanged full regressions and retained failed/passing
receipts are mandatory. No speedup, public ABI or full Rust claim follows.

## Observed acceptance

The exact 30-source stock corpus was measured with rustc 1.98.1, edition 2024,
panic=abort and no bootstrap escape. Stock accepts 23 and rejects seven. Native
classifies 18 runtime controls, five ordinary invalid controls, five valid
Unsupported controls and two stock-invalid/native-Unsupported capability
boundaries. The latter are refutable ordinary let and an unproven payload
coverage matrix, not false claims of semantic parity. Source bytes and order
match stock evidence SHA-256
`04860b9744e16be425b8c3ed626e90098cfbeb4a07aa765dcf0e10931b8070b2`.

Workers 1/8 CHECK/BUILD cover 120 cells and 36 actual ELF executions per bootstrap
profile. CHECK emits no executable; native BUILD runs with compiler fallback
disabled and exact stdout/stderr/status oracles. The corpus includes late
predicate failure after a syntactically earlier non-Copy binding, nested tag
proofs, scalar widths, all four clients and failure-edge source reuse.

Twenty-six public authority controls start from actual parsed/checked plans and
canonical facts. They test missing/reordered/foreign tag prerequisites, wrong
paths/scalars/owners/locals, duplicate/dead tests and real CFG ingress. Private
controls exercise canonical nested tag identity, substitution of every test and
binding owner, dead-test preflight, allocation denial and test-before-move CFG
ordering. Decision tests and binding checks share the existing cumulative work
cap; no limits were raised.

Nine exact retained witnesses in eight earlier corpora are promoted without
changing any source bytes, order or output oracle. Original shape-specific
controls remain intact, with separate Decision-positive controls. The nested
single-variant tag-only witness is genuinely irrefutable; this does not prove
product matrices. Combined targeted debug/release gates pass 76 tests.

Final hosted gates pass 1,033 library tests and the complete 2,636 tests across
130 binaries, with zero failures, ignored or filtered tests. All-target CHECK,
strict Clippy `-D warnings`, formatting and diff checks pass. Failed and passing
receipts are retained in
`data/runs/xps14/2026-10-17-pattern-decisions.jsonl`; the full-suite run is
`20261017-130739966-2329609`. Synthetic graph fixtures imply no compiler speedup.
