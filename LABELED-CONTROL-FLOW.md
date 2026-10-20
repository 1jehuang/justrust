# Labeled control flow

Labeled control flow is a prerequisite toward full Rust compatibility, not
completion of the control-flow or pattern categories.

## Normal-stack break parsing (2026-10-19)

Break labels and values are parsed in a small dedicated prefix helper rather
than retaining the ordinary literal/borrow dispatch frame at every nested
operand. Root resolution likewise traverses break operands before its large
constructor dispatch frame. Existing syntax nesting checks are unchanged.
Regression coverage accepts 120 nested break operands, rejects 140 with a nesting
diagnostic, and validates original labeled targets while rejecting a forged
target identity.

## Body-local result constraints (2026-10-20)

Local scalar inference carries each loop or labeled block's result variable
through its own break operands, including nested tuple projections and later
width constraints. Labels use canonical source identities and closure-local
return depth, so an inner loop cannot capture an outer labeled result by
accident. A nested hint query without the enclosing target leaves target
authority to ordinary checking. These are local inference constraints only:
existing label, move, loan and CFG validators remain authoritative.
Stock-differential regressions live in `tests/local_literals.rs`.

## Checked representation

Labels are resolved lexically to private `LoopId` values during source checking.
Loops and while statements retain their ordinary target identities. A labeled
block has its own `ExprKind::LabeledBlock { id, body }` and is evaluated once.
Break values and the normal block tail must agree on the block's result type.
An explicit continue may target a loop or while condition, never a labeled block.
An unknown or out-of-scope label is a source error, including in unused bodies.

## Lowering and cleanup

The CFG lowerer gives a labeled block one exit and one result destination, but
no continue head. Normal completion stores the body tail and reaches that exit.
A matching break evaluates and snapshots its value, stores the same destination,
ends exited scopes and branches to the exit. There is no artificial back edge.

Outer-loop breaks and continues resolve the selected target rather than the
nearest target. Cleanup uses the selected target's saved scope depth. Borrow,
initialization and move analyses consume the resulting ordinary CFG, rather than
using a separate approximate labeled-flow checker. The native backend consumes
that validated CFG without an independent structured-flow lowering path.

The implementation tracker records actual validation outcomes. General `for`
loops, iterators, destructors and remaining expression/pattern features stay open.
