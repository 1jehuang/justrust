# Checked heterogeneous integer shifts

## Scope and external contract

This is the next bounded native milestone after eager bitwise implementation
`e42ad72ef` and independently verified836case23corpus evidence. It is not full
Rust, general const evaluation, release-mode wrapping, or a performance claim.

Add exact `BinaryOp::Shl` and `BinaryOp::Shr` to the existing Binary node shape.
Both semantic operands must be integer primitives among the existing ten IntTy
variants, independently typed. Result type is exactly the LHS IntTy. Bool,
references, aggregates and semantic Never endpoints remain unsupported or invalid
according to their existing source boundaries. Typed integer values with Never
completion preserve that completion, not fabricated values. Both operand visits
and evaluation are left-to-right exactly once. No new origins, paths or ABI.

## Typing and syntax

The LHS uses its own concrete hints and any hard integer result context. The RHS
uses only its own constraints and ordinary default integer inference. Never use
RHS hints to infer the LHS, nor LHS/result type to coerce the RHS. All100 ordered
integer type pairs are admissible. Primitive identity remains exact elsewhere.
Cast soft expectations still drop at Binary, including shifts.

Lex `<<` and `>>` as longest tokens and admit Pratt precedence7, between bitwise
AND6 and addition8. Other binding powers remain unchanged through cast10 and all
four prefix operands11. Compound `<<=` and `>>=` remain explicitly unsupported.
Quoted tokens are not syntax. Extend the existing bare-cast generic ambiguity
guard to immediate `<<`, while `(x as T) << n` remains admissible. A bare cast
followed by `>>` is an ordinary shift. Preserve cast postfix/statement guards.

## Count semantics and native code

Check the FULL normalized RHS value before hardware count masking or truncation:
signed negative counts and all counts >= LHS bit width fail with the existing
checked runtime status101. This includes u64MAX, high32 counts and exact64.
Preserve the LHS value while checking RHS and moving an accepted count into CL.
Left shift discards high value bits and normalizes to the LHS width. It MUST NOT
use arithmetic overflow/range traps for lost bits. Signed right shift is SAR,
unsigned right shift SHR, from already normalized source operands. Conversion
and private eight-byte scalar/aggregate transport remain unchanged.

## Bounded static-count diagnostics

Reject directly known invalid counts during native CHECK and BUILD. Stock
rustc metadata-only CHECK accepts the two frozen static-overflow fixtures and
its codegen rejects them under arithmetic_overflow. Earlier native CHECK
rejection is an explicitly documented diagnostic-phase difference, never a
successful invalid BUILD or a hidden runtime-only promotion.

Use a bounded evaluator of CHECKED typed integer expression subtrees, not source
text guessing, runtime execution or general Rust consteval. Supported pure forms:
canonical IntValue literals, empty-statement Block tails, integer Unary Neg/Not,
IntCast, integer arithmetic/bitwise and nested shifts. Preserve exact declared
width, source-signed cast extension, checked arithmetic and count semantics.
Parentheses are already transparent. Effectful blocks are DYNAMIC opaque
barriers: retain their normal runtime effects and count guard, never fold or
execute them in the evaluator. Known negative or
too-large RHS values reject. Known arithmetic overflow, zero division or MIN/-1
inside the evaluated count must diagnose or explicitly report unsupported,
never silently wrap or become a dynamic value. Genuine Local/Call/index/deref
inputs are dynamic, retain ordinary evaluation and runtime count guards. Do not
propagate locals or inspect callee bodies. Unhandled static-shaped forms are an
explicit precision boundary, not an inferred constant. This does not claim full
Rust lint/constant-propagation equivalence for arbitrary control flow.

Bound evaluator depth128 and total work1M SHARED across all count queries in a
function, not a fresh allowance per shift. Memoize ExprIds to avoid exponential
shared-DAG work and reject visiting cycles. Charge/check arena-sized memo memory
before allocation. Check completion first: Int-typed Never-completing expressions
are never Known, including casts and empty blocks. No unbounded
bigints. Bounded host i128/u128 may implement mathematical checks, not target
128-bit support. Validate canonical constants and endpoints even dead. CFG
validation additionally rejects directly constant invalid counts and malformed
operands in dead blocks; it does not infer values from mutable CFG locals.

## Acceptance freeze

Authoritative scratch32 SHA256
`10c83f1d5cb347684655cae77ee175f74d16444c8fa0e00b2f4c9bb405a403de`:
15positive,9declared runtime101,6invalid,2legal-unsupported. Two invalid static
counts explicitly record stock_check_status0, while BUILD rejects with
arithmetic_overflow. Other invalids reject in both phases. All10 LHS types,
heterogeneous counts, width-1, sign-right, left truncation, huge counts, LTR
once, precedence/casts, references and aggregates are covered. Exact sources
must not change to conceal phase differences. Independent root stock32 compile,
metadata and runtime verification passes, including the two phase distinctions.
Final native gates are still required. No corpus growth without approval.

Owners after explicit GO: butterfly frontend enum/parser/typing and private
bounded count evaluator, frog IR/CFG validation, badger native count/kernel,
deer unchanged analysis unless a concrete need and <=2 grouped regressions,
bear NEW tests/shifts.rs, duck NEW experiments/shifts_correctness.py. Root owns
four graph tests, real example, docs/profiles and tracked24corpus native receipts.
Reviewers remain strictly read-only. Every owner publishes production+fixture
COMPILE-READY before bounded focused tests. No implementation GO is implied by
this document alone.

## Implemented validation checkpoint

The exact32 stock/native CLI cases and four graph tests pass. All1145 hosted
tests, strict all-target Clippy and formatting pass on the frozen implementation.
Final check/test/lint IDs are20261016-035156006-3713726,
20261016-035158008-3714039 and20261016-035200936-3714315, retained as exact
redacted summaries in data/runs/prototype-bootstrap/2026-10-shifts.jsonl.
Both actual-code reviews are clear after correcting effectful blocks to Dynamic
opaque barriers without changing the original snapshot regression. The real
high-bit/reference/once-evaluation example executes with stock/native stdout
parity. Independent tracked24corpus native follow-through on53908ec49 passes
all868cases:526accepted,244invalid rejected,27declared trap101 and71unsupported
boundaries. Run compiler-shifts-20261016-1053 retains24raw corpus receipts plus
the actually executed96worker example under experiments/results/20261016-m8a-shifts-*.
Independent parsing verifies796normal worker comparisons,54declared trap runs,
128generated compile/runtime exit checks, source/process hashes and complete
successful graphs. Both static-invalid stock metadata0/build-error phase receipts
are preserved. No speedup or full Rust claim is made.
