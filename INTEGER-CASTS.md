# Integer-to-integer casts

Status: implemented and hosted-validated, 2026-10-16. All component owners
and both read-only reviewers cleared the actual patch. Exact32 stock/native
plus four graph gates, 1,067 hosted tests, strict all-target lint and formatting
pass. The tracked22corpus campaign verifies all816native cases, with23 raw
corpus/example receipts retained. Implementation4c66021f0. Integer
foundation source 4957a0e7d and evidence
5004e9bdd delivered 1,021 hosted tests and 784 tracked native differential cases.
Full Rust remains the requested goal, not a claim made by these bounded gates.

## Scope and exact shared API

Support `as` between all ten existing IntTy primitives, all 100 type pairs.
Preserve primitive identity, the x86-64 target descriptor and private normalized
eight-byte leaf ABI. No bool/char/float/enum/reference/raw-pointer conversions,
128-bit integers, trait coercions, arbitrary layout/FFI or additional operators
are included. Valid excluded conversions are legal unsupported boundaries, not
invalid Rust. A source expression whose semantic type is Never remains explicitly
unsupported in this first cast slice. An Int-typed expression with Never
completion is different and retains completion without invented runtime values.

Source AST adds `Expr::Cast { value: Box<Expr>, target: Type }`.
Checked IR adds `ExprKind::IntCast { value: ExprId, to: IntTy }`.
CFG adds `Rvalue::IntCast { value: Operand, to: IntTy }`.
No public reference-contract, origin, signature or native ABI changes.
Source target types resolve normally but must be IntTy. Checked IR and CFG require
an integer operand and exact integer result even in dead blocks. Malformed
constants remain invalid before conversion. A cast never manufactures reference
origins, copies mutable references or bypasses a source literal range diagnostic.

## Source typing: soft castable expectation, not hard equality

A cast checks its source with a private SOFT CastableTo(destination IntTy) hint,
independent of the outer ambient expected result type. The cast result is exactly
the destination and still participates in ordinary surrounding type checks.
Keep private None/HasType/CastableTo distinction, or an independently designed
equivalent, without duplicating the checker or importing upstream code.

The supported AST obeys these rules:

- Unsuffixed integer literals use the soft integer hint. Suffixed literals retain
  their suffix and do not acquire destination equality. Validate source range
  before constructing canonical low-bit IntValue.
- Unary integer `-` and `!` forward the soft hint. Existing grouped-literal minimum
  handling follows the same rule. Parentheses do not turn a block into a literal.
- Ordinary Block statements keep ordinary checking. Only its tail receives the
  soft hint, without demanding block-result equality with the cast destination.
- Binary expressions drop the soft hint and retain ordinary exact operand and
  peer inference. It must not force arithmetic to execute at destination width.
- If/Match arms and Loop result constraints drop the soft hint before their
  ordinary internal unification. Existing precision boundaries remain explicit.
- Local, field, index, dereference and call results retain their actual concrete
  types. Explicit Deref drops the cast hint. Index operands, call arguments and
  annotated locals keep their own constraints.
- A nested Cast establishes its own destination hint, replacing any incoming one.

Thus unsuffixed `300 as u8` and `({300}) as u8` reject a source u8 literal,
`(-1) as u8` rejects unsigned negation, and `300i32 as u8` is an explicit typed
conversion yielding 44. A known i32 local may narrow to u8. `(200+100) as u8`
must not be forced into u8 arithmetic. This distinction must be tested by actual
runtime, not inferred from metadata-only acceptance. Source `300 as u8 as u16`
still validates the inner source literal before either conversion.

The initial cast planning claim that all unsuffixed operands default to i32, and
the alternative ambient-result-hint hypothesis, were both retracted. Primary
compiler source is semantic evidence, not an architectural authority:
`rustc_hir_typeck/src/expr.rs` check_expr_cast passes ExpectCastableToType;
`expectation.rs` separates to_option from only_has_type and drops soft hints for
branches; literal, unary, block and binary checking use those distinct operations.
Exact external oracle artifacts preserve each status and diagnostic, including
infrastructure failures as failures rather than language classifications.

## Parsing and evaluation

`as` is left associative, below unary and postfix operations and above `* / %`.
With current Pratt levels, introduce cast level7 and raise unary operand minimum
to8, preserving postfix strength. `-x as T` means `(-x) as T`; `x as A as B`
means `(x as A) as B`. Parse a real type target, not an expression or integer
suffix. Preserve statement-context rules for naked block/if/match expressions,
including required parentheses where Rust grammar requires them. Bare casts may
not be followed directly by field/index/call postfix operations. `x as usize < y`
is the generic-argument ambiguity diagnostic, not an accepted comparison without
parenthesizing the whole cast. Parser-local post-target guards must distinguish
`(x as usize) < y` and `(x as T).field` from their unparenthesized spellings without
inventing semantic Group nodes. Parenthesized target-type syntax may remain
explicitly unsupported if its existing type parser cannot represent it correctly.
Casts are not
places, computed assignment targets or reference/rvalue-borrow extensions.
Evaluate the source exactly once into the ordinary scalar value path. Existing
Call temporary materialization, scalar Copy and borrow liveness rules apply.

## Value conversion and native transport

After source typing, all 100 admitted integer conversions are total and do not
panic on narrowing. Interpret canonical source low bits using SOURCE signedness,
extend that value to64, retain destination-width low bits, then normalize by
DESTINATION signedness for transport. Same-width signedness changes preserve
bits. Signed -1i8 as u16 is65535, while u8 value255 as i16 is255.

The existing native operand loader already normalizes source signedness and
width. Cast emission may evaluate that operand once and call the existing
normalize_integer(destination), without arithmetic overflow/range-check helpers.
Do not truncate before sign extension or reinterpret narrow signed source bits
as an unsigned host magnitude. No new constant folder is required. Private
aggregate strides, sret snapshots, reference pointers and scalar calling
conventions remain unchanged. This is not Rust platform layout support.

## Ownership, resource and trust invariants

Checked IR validates source edges, completion and exact result. CFG validates
integer source/destination and every nested operand, including unreachable code.
Ownership preflight meters the closed descriptor and visits the source once;
Transfer reads/consumes the ordinary scalar operand once. borrow::visit_rvalue
visits it once so a Deref source retains its carrier use and loan lifetime.
origin_step does not propagate reference origins through casts. No new paths,
origin rows, numeric lattice or growing allocation are introduced.

All current node/depth/work/state caps, complete-candidate contracts, all-body
semantic gate, CHECK-without-OBJECT behavior, exact ANALYZE-before-OBJECT trust
and failure retention remain authoritative. Native input compilation must not
invoke rustc, Cargo, LLVM, assemblers, linkers or interpreter fallbacks. External
stock tools are only bootstrap/oracles. Do not install this experiment over stable
justrust. No performance claim without the comprehensive measured gate.

## Frozen acceptance corpus

Scratch artifact `integer-casts-oracle/corpus.json` SHA256
`942778196cb187edd61937b08c38f1c30871af1dff2145dfcbc6857323a782b3`
contains exactly32 cases:26 positive, three invalid and three valid excluded
boundaries. Independent stock compile, metadata check and actual runtime passed
all32 under rustc1.98.1 edition2024. Ten matrix cases cover all100 type pairs with
runtime source parameters and mathematical low-bit expected values;16 focused
positive cases cover soft source hints, control flow, precedence and once-only
reference operand effects. There are no expected runtime-overflow cases because
integer casts themselves never overflow-panic. Exact native32 plus four graph
tests pass as well. Tracked run compiler-integer-casts-20261016-1028 verifies
all816cases across22corpora and executes the96worker example.

## Required acceptance and ownership

Freeze at most32 source-hashed stock cases before project tests. Include all100
source/destination pairs with independent mathematical low-bit expectations,
signed/unsigned edge extension, full64 transport, once-only operand effects,
left-associative chains and unary/multiplicative precedence, source soft-hint
contrasts, grouped minima versus blocks, projected reference operands and last
use, invalid source ranges/noninteger endpoints, and explicit excluded valid
boundaries. Do not accept timeouts, signals or compiler crashes as rejections.
All explicit int-to-int narrowing must complete, not be treated as overflow101.

Retain four all-body/graph tests, high-width native example, full hosted
check/test/strict all-target lint/fmt, malformed checked-IR/dead-CFG tests and
tracked full differential receipts before source promotions. Existing source
and stdout bytes must remain unchanged for any historical promotion.

Proposed file ownership after explicit GO:

- frog: ir.rs/cfg.rs typed cast variants, validation, lowering and scoped fixtures.
- butterfly: frontend.rs syntax, soft source hints and expected-type plumbing.
- deer: borrow.rs/ownership.rs exhaustive visitors, metering and liveness tests.
- badger: backend.rs normalized nontrapping cast emission and bounded tests.
- bear: NEW tests/integer_casts.rs from frozen corpus only.
- duck: NEW experiments/integer_casts_correctness.py from identical corpus only.
- coordinator: graph tests/example, docs, receipts, exact promotions and commits.
- buffalo/shrimp: read-only actual-patch correctness review, no corpus growth.

No owner edits before feasibility confirmation and an explicit implementation GO.
