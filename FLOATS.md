# Native floating-point primitives

This work extends the independent compiler toward full Rust support. It does not
close the full numeric, library, ABI, or project compatibility requirements.

## Representation and operations

`f32` and `f64` have distinct checked types and canonical IEEE bit constants.
An f32 carrier occupies the low 32 bits of the existing private eight-byte scalar
slot, with zero upper bits. This is a private compiler ABI, not platform FFI.
Aggregate layouts and argument transport continue to use that private ABI.

Native arithmetic uses scalar SSE operations for addition, subtraction,
multiplication and division. Remainder uses an iterated x87 FPREM followed by a
store at the declared width. Unary negation flips the sign bit, preserving signed
zero. Ordered comparisons reject unordered operands, equality is false for NaN,
and inequality is true for NaN. There is no integer-style overflow trap for
floating-point arithmetic. Subnormal values are not deliberately flushed to zero.

Numeric `as` conversions involving floats are represented explicitly in checked
IR and CFG. Float-to-integer conversion truncates toward zero, saturates at each
integer endpoint and maps NaN to zero. Float width conversion and integer-to-float
conversion round at the destination width. Unsigned 64-bit conversion uses a
sticky-bit halving path for values outside the signed range. Integer-to-integer
casts retain their separate operation and existing contract.

Float literals retain decimal source text until the resolved width is known, so
f32 literals are parsed directly to f32 instead of double rounding through f64.
Omitted generic arguments retain distinct float-domain inference evidence with
f64 fallback only after constraints are collected. Unknown evidence cannot force
numeric fallback or turn a float constraint into an integer constraint.

## Verification

`tests/floats.rs` compares native executable behavior with an external stock
compiler, independently of the native compilation path. The runner exercises
CHECK and BUILD with one and eight workers, bounds every subprocess, compares
exit status and output, and verifies that CHECK never publishes an executable.
`JUSTRUST_TEST_RUSTC` selects an externally pinned oracle executable.
`tests/floats_graph.rs` covers shared graph facts, unused invalid bodies, borrow
violations and typed noncompletion. Private inference tests exercise fallback,
tainted evidence and incompatible numeric domains.

These checks are acceptance gates, not a full Rust conformance claim. Recorded
outcomes must be read together with the implementation tracker.

## Still open

This does not implement float formatting, the complete primitive method library,
const evaluation, float patterns, target ABI interoperability, f16/f128, or a
standard-library build. General type inference, platform support and the other
remaining implementation categories stay open.
