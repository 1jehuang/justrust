# Eager integer and Boolean bitwise operators

Status: frozen implementation contract, 2026-10-16. All owners and both
read-only reviewers confirm feasibility against current source. Implementation
requires a separate explicit GO. Frozen20stock cases pass compile, metadata and
runtime:14positive,4invalid,2legalunsupported. Artifact
`bitwise-oracle/corpus.json` SHA256
`1b3f06ed4dae9915ffb67625f18898193c12feb21f396cd1385b63c8dba8d7e9`.
Full Rust remains the
goal. Cast implementation4c66021f0 and evidencee9279d214 delivered1,067hosted
tests and816tracked native cases, not full compatibility or a speedup claim.

## Scope and shared representation

Add `BinaryOp::{BitAnd, BitOr, BitXor}` to the existing source-owned operator enum
reused by checked IR and CFG. Keep `And` and `Or` exclusively short-circuit logical
operations. No new expression/rvalue shape, origins, signatures or native ABI.

The new operators accept either two exactly identical `IntTy` operands or two
Boolean operands. The result has the same type. No mixed integer identity,
implicit promotion, reference/operator traits, unit/aggregate operands or
integer/Boolean conversion. Both sides are evaluated eagerly, left then right,
exactly once, even when a Boolean left value determines the result. Completion
requires both operands to return, unlike logical short-circuit conditional edges.

Integers operate on the declared low bits and use existing signed/unsigned
normalization for the result. These operators do not arithmetic-overflow trap.
Booleans use ordinary eager truth tables and retain canonical0/1 representation.
Existing malformed constants/dead CFG/type/completion checks remain mandatory.
No source reference authority, initialization or lifetime rule changes.

## Source syntax and precedence

Lex single `|` and `^` in addition to existing `&`, preserving longest-token `||`
and `&&`. Prefix `&` remains borrowing. Infix `&` is eager bitwise. Compound
assignments stay explicitly unsupported, not silently parsed as two operations.

Use Rust relative precedence throughout the Pratt parser, leaving a reserved
shift slot for a separate milestone:

| Level | Operation |
|---|---|
|1|logical `||`|
|2|logical `&&`|
|3|comparisons|
|4|bitwise `\|`|
|5|bitwise `^`|
|6|bitwise `&`|
|7|reserved shifts, unsupported here|
|8|addition/subtraction|
|9|multiplication/division/remainder|
|10|integer casts|
|11|prefix borrowing, dereference, negation, not|

Postfix remains above prefix. Audit every hardcoded parser binding-power use,
including all four prefix operators and cast admission. The chained comparison
guard must recognize only comparisons, never level4bitwise OR. Preserve existing
statement-expression and cast-postfix/less-than ambiguity guards. Grouped cast
targets remain unsupported. Quoted operator/type tokens must not become syntax.

Typing keeps existing scoped exact numeric inference, not full backwards numeric
inference. Concrete operand hints and hard expected result type may guide
unsuffixed operands as current arithmetic does. Cast soft expectations are dropped
at Binary, unchanged. A bitwise scalar hint returns operand identity rather than
unconditionally Bool. Logical/comparison results remain Bool.

## Lowering, native execution and analysis

Only logical And/Or take the CFG short-circuit path. BitAnd/BitOr/BitXor take the
existing eager binary lowering and owning scalar snapshots. Keep call/RHS effects
ordered and both operands' loan uses live where required. No rereading a mutable
carrier after RHS changes the original source storage.

The backend performs a fullword AND/OR/XOR on normalized source operands, then
normalizes the exact result type. Boolean transport remains0/1. No arithmetic
range helper or new runtime trap policy is introduced. Private8byte scalar leaves,
aggregate transport, references, parameter/result registers and sret are unchanged.

Existing Binary visitors already visit left then right once for preflight,
transfer and borrowing. Do not create reference origins from bitwise operations.
All existing depth/occurrence/work/state/path budgets apply. The previous cast
stack-frame and bounded checked-IR lowering regressions must continue passing.

## Acceptance and ownership

Freeze at most20source-hashed stock fixtures before project writes. Include all
ten integer types, Boolean eager effects, all three operators, precedence across
comparison/arithmetic/cast/borrow, source-evaluation-once, signed/high-bit values,
reference/aggregate scalar transport, exact type/range rejection and valid excluded
compound assignments/128bit operations. Preserve source/stdout bytes across harness
and tests. External stock is an oracle only, never a native user-input fallback.

Root owns graph tests, real native example, final full hosted check/test/strict
lint/fmt, exact redacted profiles, tracked23corpus raw native receipts and actual
96worker example. No performance/default promotion without representative gates.
Existing fixtures are promoted only after exact-source native runtime proof.

Owner scope after a separate explicit GO:
- butterfly: frontend enum/lexer/Pratt/typechecker/hints, <=3grouped regressions.
- frog: IR/CFG exact typing/completion/eager lowering, <=2grouped regressions.
- deer: ownership/borrow only if a concrete need, <=2grouped regressions.
- badger: native eager kernel/normalization, <=2grouped regressions.
- bear: NEW tests/bitwise.rs exact frozen corpus only.
- duck: NEW experiments/bitwise_correctness.py exact same corpus only.
- shrimp/buffalo: read-only actual patch review.

Publish production and fixture migration compile-ready before focused test work.
No worker commits, spawns, broad gates, historical promotions or corpus growth.

## Separate shift milestone, not enabled here

Shifts need distinct Shl/Shr variants, independently typed integer RHS of any
width, LHS-only result identity and full signed-negative/unsigned-width count
checks before x86 masking. Left shifts truncate lost value bits without arithmetic
overflow trapping; signed right shifts are arithmetic and unsigned right shifts
logical. Count inference, static overflow policy and traps require their own
frozen corpus and implementation gates. No shift support is implied here.
