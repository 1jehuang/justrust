# Integer leading-zero count and original core estimator

Status: native semantic and exact-source debug/release acceptance passed on
2026-10-18. Full regression and integrated-worktree validation are recorded below.
Full Rust remains open.

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

## Acceptance results (2026-10-18)

The stock-first frozen corpus contains 30 exact sources: 20 valid, eight invalid,
and two legal but unsupported (reference autoderef and UFCS). Both public CLI and
observed graph entrypoints pass CHECK/BUILD at workers 1 and 8, in debug and
release bootstrap profiles. Every valid binary's complete output matches stock.
Each profile covers 224 native check/build cells and 80 executable outputs.
The primitive cases include all u8/i8 bit patterns, all u128 single-bit positions,
nested calls, signed widths, nominal methods, once-only effects and divergence.

The original estimator provider is byte-identical, SHA256
`2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5`.
Its client checks nine fixed cases and 14,693 mantissa/exponent sweep cases per
execution against a separate integer oracle and exact stock output. It is an
external source module, not a prebuilt core artifact.

Validation receipts are retained in
`data/runs/xps14/2026-10-18-leading-zeros.jsonl`:

- Debug exact-source corpus: `20261018-025509828-485800`.
- Release exact-source corpus: `20261018-025519407-488098`.
- Release internal controls: 24 passed, `20261018-025543486-490981`.
- Full isolated regression: 3,204 passed, `20261018-025621281-492123`.
- Integrated worktree hosted check: `20261018-025706102-516719`.
- Integrated hosted library tests: 1,316 passed, `20261018-025715568-514599`.
- Integrated native acceptance and pending character-cast controls: 11 passed,
  `20261018-025820398-614301`.
- Strict integrated all-target Clippy: `20261018-025849142-614225`.
- `cargo fmt --check` and `git diff --check` passed.

The hosted exact-source test initially failed because `/usr/bin/rustc` was not
available there (`20261018-025323196-474305`), so stock-oracle/native-process tests
ran locally. Hosted check, library tests and lint still passed. Local acceptance
exposed a divergent block receiver inference gap (`20261018-025346734-475970`),
then an over-broad negative-test diagnostic exclusion
(`20261018-025437198-482227`). Both were fixed and rerun. Initial Clippy rejected
an err/expect test idiom (`20261018-025354931-477116`), also fixed. These failed
receipts remain alongside success records.

The public immutable publication seal rejects same-expression-ID payload edits,
including receiver swaps. Helper replay checks metadata and graph consistency,
but does not independently reconstruct every typed expression from the AST.
No full-Rust percentage, compiler speedup, optimization quality, whole-core,
CTFE or general Cargo-project compatibility claim follows from these tests.
