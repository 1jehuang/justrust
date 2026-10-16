# Integer foundation through 64 bits

## Status and scope

This is a coordinated implementation contract, not a claim of full Rust support.
The full request remains open in IMPLEMENTATION.md. The preceding snapshot has
975 hosted tests and 752 native differential cases for bounded Linux x86-64
features. No speed improvement is claimed by adding integer semantics.

This milestone unifies i8, i16, i32, i64, isize, u8, u16, u32, u64 and usize.
It admits their literals, existing arithmetic/comparisons, signed negation,
integer bitwise unary !, and integer literal match patterns. Cast syntax,
binary bitwise/shift operators, compound assignments, i128/u128, floats,
associated constants, const evaluation and broader inference remain separate
follow-through work. In particular cast inference has conflicting planning
reports and must not be guessed into this milestone.

## Shared checked representation

- `ir::IntTy::{I8,I16,I32,I64,Isize,U8,U16,U32,U64,Usize}` is closed,
  Copy/Eq/Hash and retains primitive identity independent of equal bit widths.
- `IntTy::bits() -> u32`, `IntTy::is_signed() -> bool`,
  `IntTy::mask() -> u64`, and `IntTy::name() -> &'static str` are shared helpers.
  Isize/Usize are explicitly 64 bits for the supported x86-64 target, never
  derived from host usize. Width 64 never shifts a u64 by 64.
- `ir::IntValue { pub ty: IntTy, pub bits: u64 }` holds canonical raw low bits.
  Bits above the type width must be zero, including for negative values.
  `IntValue::validate() -> Result<(), String>` verifies this before use.
  Shared convenience constructors may be added, but no parallel constant model.
- Replace Ty::I32/Usize with `Ty::Int(IntTy)`, checked ExprKind::Int(i32)/Usize
  with `ExprKind::Int(IntValue)`, CFG constants with `Constant::Int(IntValue)`,
  and checked MatchPattern::Int(i32) with `MatchPattern::Int(IntValue)`.
  Internal enum discriminants remain private I32 typed tags, not inferred usize.
- Source Type may use `Type::Int(IntTy)` and source literals must preserve a
  nonnegative magnitude and optional suffix until contextual resolution.
  Frontend owns its exact source AST shape and publishes it before consumers
  migrate fixtures. No unchecked reference or pointer constant is introduced.

All scalar/type validators, dead CFG validation, equality, projection and
reference candidate matching must see exact integer identity. Runtime array
indices remain exactly Usize, not U64. Existing path/state/work limits remain.

## Literals and typing

Decimal, binary, octal, hexadecimal and permitted underscores use a bounded
checked u64 accumulator with exact suffix recognition. Oversized values fail
closed without bigint allocation. Unsuffixed literals use the existing concrete
expected or peer type, otherwise i32, with no implicit numeric promotions.
Backward inference limitations stay explicit unsupported, not invalid Rust.

Remove the token-specific -2147483648 shortcut. A unary negative whose operand
is a literal, optionally surrounded only by parentheses, resolves the magnitude
against the contextual signed limit, allowing exactly 2^(width-1). A block is
not a grouped literal. Other signed negations are runtime checked. Unsigned
negation is rejected. Positive signed literals stop at 2^(width-1)-1. u64::MAX
must not round-trip through i64. Source array lengths validate target Usize then
existing length/resource bounds before host allocation conversions.

Integer literal patterns resolve against exact scrutinee type, with duplicate,
reachability and exhaustiveness checks migrated. Narrow domains may be covered
exhaustively without a wildcard. Never enumerate a 64-bit domain or compute a
host-sized shift by 64. Range/or patterns and new pattern forms stay separate.

## Execution semantics and private transport

The current native policy remains checked arithmetic with status 101 on admitted
runtime overflow/division failures. No release wrapping, panic wording, unwinding
or FFI/layout compatibility is implied. Add/sub/mul and division/remainder require
identical integer types. Comparisons select signedness from IntTy. Neg is signed
only, ! complements the declared integer width and remains logical for bool.

Every private scalar slot and aggregate scalar leaf remains 8 bytes. Signed
transport is sign-extended to 64 bits, unsigned transport zero-extended. Constants,
loads, arithmetic, calls, returns, arrays/repeats, enum payloads, dereference and
aggregate copies must preserve this invariant. References remain thin64 values
of a separate class. This is not repr(C) or packed Rust layout support.

For widths below 64, extended arithmetic must range-check before truncation.
Signed64 uses correct overflow checks, unsigned64 carry/borrow and multiplication
high-half checks. Division guards zero and signed MIN/-1 for BOTH div and rem at
every width before hardware division. Signed quotient truncates toward zero,
remainder follows dividend sign. Normalize results without silently wrapping.

## Ownership, graph and trust

Integers remain Copy scalar leaves. No numerical value lattice or new origin
kind is required. Preserve all-body semantic gates, exact CFG analysis before
object generation, CHECK without OBJECT, deterministic malformed-dead-CFG checks,
and existing byte/work caps. No permanent compatibility shim or stock compiler
execution is added to the native input path. Engine facts are scoped to their
immutable source snapshot, so no old compiled object is reusable across this
compiler revision. Any additional persistent artifact identity must include the
new typed scalar representation before reuse.

## File ownership and acceptance

- frog: ir.rs/cfg.rs, shared descriptors, constants, patterns, validators and
  owned fixture migrations. Publish shared API first.
- butterfly: frontend.rs, literal parsing/context/minimum semantics and source
  AST migrations, exact pattern typing and focused regressions.
- badger: backend.rs, one width/signedness-driven checked kernel and every
  transport path, focused native regressions.
- deer: ownership.rs/borrow.rs required scalar/constant migrations and focused
  verification, no speculative ownership redesign.
- bear: NEW tests/integer_types.rs only, bounded approved stock corpus.
- duck: NEW experiments/integer_types_correctness.py, same source corpus.
- coordinator: graph tests, example, docs, profiles and receipts.
- shrimp/buffalo: read-only actual-code correctness/resource review.

Freeze a bounded corpus before native acceptance. Include all ten type bounds,
radices/underscores, grouped minima versus block negatives, exact type mismatch,
transport through calls/aggregates/references, signed division/remainder and
narrow/64-bit overflow. Runtime trap fixtures use nonconstant inputs and exact
stock/native exit 101, not hardware signals. Forge malformed constants/dead CFG,
exercise workers1/8 and all-body failure retention. Preserve all existing source
fixtures unless independently executed exact-source behavior justifies a named
classification promotion. Final format/check/all-tests/strict lint and the full
native differential corpus precede completion and evidence commits.
