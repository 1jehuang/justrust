# Bounded equality assertion macros

Status: implementation in progress, not delivered. Full Rust remains open.

Add `assert_eq!(left, right)` and `assert_ne!(left, right)` for the engine's already admitted value-comparison domain. Evaluate left and right once, in source order, using existing equality typing and execution. Success yields Unit. Failure uses checked literal panic with the assertion invocation location. Support parentheses, brackets and braces, an optional trailing comma, and an optional literal message with a trailing comma. Preserve original statement and match-arm punctuation rules.

Select builtins only after checked lexical/module macro lookup misses. This milestone does not implement general PartialEq/Debug dispatch, reference comparison, aggregate operators, implicit assertion borrows, nonliteral formatting or value pretty-printing. Existing operator boundaries must not be widened to make a test pass. In particular, do not move a non-Copy aggregate merely to emulate Rust's borrowed assertion operation.

The default native diagnostics are `assertion failed: left == right` and `assertion failed: left != right`, plus the checked source location. They are a documented diagnostic subset, not byte-identical stock output. Custom literal messages reuse checked literal decoding.

Acceptance must preserve independently measured source bytes and cover left/right evaluation order, exactly-once effects, ordinary Unit success, genuine SIGABRT failure, all delimiters, shadowing, source/IR authority and unchanged earlier unsupported boundaries. Run workers 1/8 CHECK/BUILD, debug/release, full regression, formatting and strict lint. No performance or full Rust claim follows from this slice.
