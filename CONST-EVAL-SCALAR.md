# Scalar constant evaluator building block

Status: standalone evaluator only, not compiler const-item acceptance.

`src/compiler/const_eval.rs` is intentionally not declared in `compiler/mod.rs`.
`tests/const_eval_scalar.rs` imports it through `#[path]` until coordinated graph
integration is approved. No parser admission or existing unsupported boundary
changes in this checkpoint. Full Rust support is not complete.

## Frozen isolated contract

- Immutable per-item body and value containers use canonical snapshot-local
  `DeclId` identity. These IDs are not persistent cache identities.
- Fixed-width signed and unsigned integers (8 through 128 bits) and bool.
  Target-sized integers, references, aggregates, statics and const execution
  of functions remain unsupported.
- Structurally typed topological expression arena, iterative evaluation, local
  memo/stack, caller-supplied task budget. No shared counters or side slots.
- Checked arithmetic, including signed MIN / -1 and MIN % -1, faults with
  evaluation category E0080. Shift counts must be nonnegative and below the
  operand width, but shifted-out bits are truncated. Signed right shift extends
  the sign. Unary negation overflow is checked.
- Logical `&&` and `||` skip value evaluation of the unused operand, not its
  structural/type/const-operation validation. Runtime calls receive E0015.
  Valid const-function calls are explicitly unsupported, not mislabeled E0015.
- Async dependency resolver receives the exact referenced declaration identity.
  Its returned declaration and scalar type must match. Only actual engine
  handle reads in the future resolver establish graph dependency edges.
- Body admission charges incoming Vec capacity plus the simultaneous frozen Arc
  allocation. The body holds an immutable `RetainedStorage` receipt, not a budget
  or mutable lease. Future producers must admit AST construction before allocation.

## Mandatory integration work still open

The public body constructor checks structural consistency only. It does not
prove that arbitrary expressions came from the authoritative declaration source.
Before publishing a CONST_BODY output the producer must authenticate the exact
initializer, declared type, resolved names and source publication against its
CONST_DECL input. A swapped body retaining the same DeclId must be rejected.

Reserved future operations are 118 CONST_DECL, 119 CONST_BODY and 120 CONST_VALUE,
subject to coordinator integration. No kinds are wired in this checkpoint.
Values must be read through exact typed graph handles, never inserted into
PlanCtx or substituted with runtime initializer functions. Existing CHECK must
demand every required constant, including unused invalid declarations. Runtime
body consumers demand only the values they actually reference. Genuine cycles
and dependency failures must retain graph diagnostics, not be relabeled E0080.
The async callback interface will need integration-specific graph error transport.

Source typing must preserve contextual admission of negative MIN literals:
`-128i8` is a canonical minimum literal, while negating a value already equal to
MIN is overflow. The standalone arena starts after literal elaboration and does
not implement that source rule.

## Validation scope

Standalone tests cover fixed-width boundaries, checked arithmetic against host
integer methods, shifts, comparisons, bitwise operations, type/identity forgery,
short circuiting, async suspension, deep iterative expressions and task budgets.
External stock-rustc cases validate CTFE semantic expectations independently.
They are not native compiler source-to-executable const-item acceptance tests.
Graph-integrity checks must remain green without ratchet changes.
