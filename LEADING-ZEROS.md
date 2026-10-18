# Integer leading-zero count and original core estimator

Status: contract frozen, implementation and acceptance pending. Full Rust remains open.

## Native semantic slice

Admit zero-argument `.leading_zeros()` on exactly typed by-value primitive integer
receivers (`i8/i16/i32/i64/i128/isize`, `u8/u16/u32/u64/u128/usize`). Return `u32`.
Interpret the exact declared-width bit pattern, including signed negative inputs.
Zero returns the declared bit width. Evaluate the receiver exactly once and
preserve divergent receiver completion. Generic, reference, float, Bool and char
receivers do not acquire integer methods through spelling alone. Ordinary user
methods named `leading_zeros` remain ordinary methods. Arguments and generic
method arguments are not silently ignored. Omitted integer receiver inference,
reference autoderef and UFCS are not claimed by this first slice.

Keep an explicit unary checked IR operation and explicit CFG operation. Validate
integer operand and exact u32 result, operand completion, child ownership and
source occurrence authority, including unused malformed rows and replayed source
bodies. All traversal, substitution, provenance, budget and ownership consumers
must cover the operation. No per-name compiler implementation is substituted for
an ordinary source function.

The Linux x86-64 emitter uses baseline instructions with a defined zero path,
not optional LZCNT or host CPU assumptions. Count within 8/16/32/64 bits and both
halves of 128-bit operands. Native arithmetic remains checked, so this is not
permission for integer wrapping or unrelated bit intrinsics.

## Original-library stepping stone

Retain the complete original `core/src/num/imp/flt2dec/estimator.rs` bytes under
`libraries/probes/`, with its SHA256, imported source revision and existing Rust
license attribution. An authored client calls its original
`estimate_scaling_factor` through a real external module. Test both stock and
native on identical provider/client files, not an edited approximation of the
provider. Cover mantissa 1 and width boundaries, positive/negative exponents,
and the provider's approximation semantics. This is whole estimator-module
acceptance, not whole core, its original numeric method provider, a native
library crate artifact, CTFE, alloc/std or general project support.

## Required evidence

Freeze a stock-derived primitive-method corpus before defining native expected
outcomes. Cover all integer widths, zero and top bit, signed negatives, u128 high
and low halves, once-only receiver effects, dead-invalid source, wrong receiver,
wrong arity/type arguments and unchanged ordinary user methods. Verify public
CLI CHECK/BUILD/execution at workers 1 and 8, complete actual output, and public
malformed-IR/CFG controls. Run focused debug and release gates, full regression,
strict lint and formatting. Retain failures and successes and separate stable
stock semantics from bounded legal-unsupported cases. Update this status only
after actual acceptance passes.
