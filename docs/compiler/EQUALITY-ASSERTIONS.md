# Bounded equality assertion macros

Status: bounded milestone delivered. Full Rust remains open.

The native comparison domain is Unit, Bool, integers and floats. Arrays, tuples,
nominal aggregates, references and general trait-based equality remain outside
this slice. Array comparison now rejects in source typing rather than reaching
invalid CFG construction.

Validation: 27 immutable stock-first sources (26 original plus one separately
measured lexical-shadow control), 108 CLI cells and 108 observed-graph cells
across workers 1/8 and CHECK/BUILD, with 42 native runtime executions per matrix.
The native partition is 18 successful programs, 3 aborting programs, 1 ordinary
invalid program and 5 earlier unsupported boundaries. Seven public authority
tests and optimized equality/logical controls passed. The complete local suite
passed 3,065 tests in 152 binaries. All-target check, strict lint and formatting
passed. Exact failed and successful receipts are retained in
`data/runs/xps14/2026-10-18-equality-assertions.jsonl`.

Full and stock-abort gates use the previously documented local execution
exception. No hosted full-suite, performance improvement or full Rust support
is claimed.

Add `assert_eq!(left, right)` and `assert_ne!(left, right)` for the engine's already admitted value-comparison domain. Evaluate left and right once, in source order, using existing equality typing and execution. Success yields Unit. Failure uses checked literal panic with the assertion invocation location. Support parentheses, brackets and braces, an optional trailing comma, and an optional literal message with a trailing comma. Preserve original statement and match-arm punctuation rules.

Select builtins only after checked lexical/module macro lookup misses. This milestone does not implement general PartialEq/Debug dispatch, reference comparison, aggregate operators, implicit assertion borrows, nonliteral formatting or value pretty-printing. Existing operator boundaries must not be widened to make a test pass. In particular, do not move a non-Copy aggregate merely to emulate Rust's borrowed assertion operation.

The default native diagnostics are `assertion failed: left == right` and `assertion failed: left != right`, plus the checked source location. They are a documented diagnostic subset, not byte-identical stock output. Custom literal messages reuse checked literal decoding.

Acceptance must preserve independently measured source bytes and cover left/right evaluation order, exactly-once effects, ordinary Unit success, genuine SIGABRT failure, all delimiters, shadowing, source/IR authority and unchanged earlier unsupported boundaries. Run workers 1/8 CHECK/BUILD, debug/release, full regression, formatting and strict lint. No performance or full Rust claim follows from this slice.
