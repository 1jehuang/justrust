# Scalar source constants: accepted M1 slice

Status: integrated source slice validated on frozen `dd403c1cd`: 3,472 full
suite tests passed, plus strict hosted all-target lint and formatting. Exact
central receipts are in `data/runs/const-source/`. This is not full Rust const
support, and no performance or whole-compiler compatibility claim follows.

The durable isolated evaluator checkpoint is `2199b26f0`. It deliberately did
not admit source const items. The integrated source work connects the evaluator to
original-source declarations and graph consumers. Tests now use the normal
compiler module rather than a standalone `#[path]` copy.

## Exact accepted slice

- Original-source free immutable `const NAME: TYPE = EXPR;` items only.
- Explicit fixed-width signed/unsigned integers (8 through 128 bits) and bool.
- `usize` with exact u64 semantics, pinned to the frozen native target
  (`target_pointer_width = "64"`, `const_eval::NATIVE_TARGET_POINTER_BITS`,
  compile-time asserted against `IntTy::Usize.bits()`). Not derived from the
  compiler host. Stock oracle: `tests/fixtures/const_usize` (rustc 1.98.1
  48a229cea, SHA256SUMS) for all operators plus E0080 overflow/shift/division,
  E0308, E0600, out-of-range literal and E0391, CHECK and BUILD at workers 1/8.
- Literals, resolved constant references, unary negation/not, integer arithmetic,
  comparisons, bitwise operations, shifts and logical short circuiting.
- Contextual negative MIN literal admission is distinct from negating a value
  already equal to MIN. Signed MIN / -1 and MIN % -1 are E0080.
- Shift counts are checked, shifted-out bits truncate, signed right shifts
  extend the sign. Integer arithmetic overflow and division by zero are E0080.
- Module value lookup, imports and visibility remain ordinary source resolution.
  A constant is never classified as a local mutable place. An in-scope constant
  name in a binding pattern is not a shadowing local: constant patterns remain
  explicitly Unsupported, including imported aliases, tuples, function/closure
  parameters, for bindings and matches. Distinct local names remain supported.
- Runtime function calls preserve name/type/arity checking before E0015. Valid
  const-function execution remains explicitly Unsupported, not false E0015.

Still unsupported: static/static mut, `isize` constants, aggregate
constants, casts, reference-valued constants, const blocks, general const fn
execution, associated constants and const generics. Array lengths/repeats keep
existing literal-only boundaries. Generated const declarations are not admitted
through the function-only item macro product authority. In immediate closures,
unqualified nonlocal lookup remains fail-closed because parent-parameter
absence is not proof against a shadowing parent local. Explicit module/crate
constant paths do not bypass lexical capture precedence.

## Per-item graph contract

- **118 CONST_DECL** uses one canonical declaration index. Its factory checks
  the exact original resolved `Arc<Program>` and authoritative declaration
  occurrence. It projects only immutable scalar syntax, source-node identity,
  declaration/module/type/visibility primitives and an immutable storage receipt.
  No Program, NameStore, token AST, PlanCtx or mutable budget is retained.
  A private seal binds the exact initializer arena and declared metadata.
  Metadata reads no other initializer, qualification or evaluated value.
- **119 CONST_BODY** output0 is source-authenticated structurally typed scalar
  syntax. Output1 is qualification, requiring all syntactically direct referenced
  119 output1 handles, including skipped operands. Output0 alone cannot authorize
  evaluation. Body and qualification constructors are private/sealed.
- **120 CONST_VALUE** requires the exact own119 body0/qualification1 publication.
  Evaluation lazily reads only executed direct120 values. Dependency failures
  remain engine failures, never generic E0080 substitutions.
- **110 TEMPLATE_BODY** prepares the exact source occurrence inventory after
  existing closure/capture graph admission. It requests all direct120 handles
  before synchronous checking, seals complete per-body inputs, emits literals,
  and replays exact source-node/declaration/value/expression/type correspondence.
  The legacy no-input wrapper rejects any constant use. No values enter PlanCtx.
- **104 CHECK** demands every required constant120, including unused declarations.
  BUILD remains gated by required semantic checks. No global const-value map,
  runtime initializer helper, all-constant body barrier or side-slot result exists.

## Cycle diagnostics and resource boundary

Stock Rust rejects skipped self/mutual constant references as E0391 even when
CTFE would skip their values. Therefore qualification and evaluation use separate
edges. Arithmetic faults behind skipped logical operands do not execute.

The engine records a complete ordered cycle from actual unresolved live waits,
including exact keys/output slots, independently of event capture. Diagnostic
witness extraction uses input-scaled checked counts and fallible Vec allocation,
not a new global/compiler budget. Allocation failure yields no witness and keeps
the generic engine error. Compiler diagnostic interpretation charges its existing
budget and recognizes E0391 only for authenticated119 qualification1 edges whose
body0 source metadata contains every direct reference. Non-constant cycles,
resource faults, external waits and unrelated root failures are not relabeled.

## Body constraint propagation

A resolved constant occurrence contributes its scalar type through the same
sealed `canonical.constant_value` input used by expression lowering. The body
solver does not evaluate the initializer again or infer authority from a name.
This constraint precedes numeric fallback, so named and qualified `N: usize`
endpoints constrain `0..N` items to `usize` rather than `i32`.

`tests/const_constraints.rs` passes stock/native CHECK, BUILD and execution at
workers 1 and 8 for both endpoint forms, and rejects incompatible stores and
unused invalid bodies (receipt `20261019-155728070-934903`). The current-parent
mandatory graph gate passes all 14 tests (`20261019-161009565-990082`). The
unchanged matrix program advances past the prior integer mismatch but still
rejects with E0282 for a closure parameter (`20261019-160812150-983718`). This is
not matrix acceptance, Iterator activation or general const evaluation.

## Validation boundaries

`tests/const_eval_scalar.rs` checks the isolated canonical arithmetic contract and
stock-rustc oracles. `tests/const_source.rs` exercises actual source check/native
build/run with workers1/8, unused values, source lookup, call categories and
qualification cycles. Source authority tests reject swapped initializers and
missing runtime graph inputs. Engine tests cover capture-independent cycle
witnesses, mixed output slots, early publication and fallible allocation.
Graph-integrity ratchets may not increase. Complete source/regression/lint gates
and coordinator approval are still required before publication.

### Retained allocation audit

Body, qualification and value producers prepay their own payload and engine Arc
header before construction/publication, retaining immutable receipts without any
ledger back-reference. Body0 additionally prepays its shared body Arc and the
engine's outer Arc wrapper. Qualification/value readers share that body, not a
second payment for its payload. Source projection pays both frozen arena headers,
the empty root path header and the declaration publication header. Nonempty source
paths and owner identities use the existing prepaid source-replay constructors.
Checked scalar arenas pay incoming Vec capacity and overlapping frozen Arc storage.
Runtime occurrence/emission vectors use source-replay prepaid geometric growth;
direct dependency lists/handles/proofs remain paid provider-local scratch.

Scoped controls test checked wrapper-size overflow, one-byte-short admission for
each constructor before a fact can be returned for publication, exact qualification
wrapper payment without repaying the shared body, and receipt clone/drop without
refund. Diagnostic controls run with capture disabled: a real source119 cycle
still has a typed witness, coexisting independent resource/type errors are not
relabeled, and mixed119/120 cycles remain generic. Full regression acceptance is
still pending coordinator scheduling, not implied by these focused controls.
