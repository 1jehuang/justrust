# Labeled control flow

Labeled control flow is a prerequisite toward full Rust compatibility, not
completion of the control-flow or pattern categories.

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
