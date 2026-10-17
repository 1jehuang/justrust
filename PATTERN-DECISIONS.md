# Nested pattern decisions: ordered tests before moves

Status: pending implementation acceptance. Full Rust compatibility remains open.

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
