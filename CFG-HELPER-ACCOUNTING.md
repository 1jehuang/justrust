# Private bounded CFG helper admission follow-up

This follows OR source `bd912d003`, evidence `7a9dda14c`, and sticky/span repair
`20c00b60c`. It is not full CFG accounting or corpus09 promotion. Red checkpoint
`6c1e670cd` preserves the original exact panic receipt regression unchanged.

## Scope and failure protocol

Only `cfg.rs` producer helpers and their tests change. Comparison pairs use a
fixed two-slot stack array. Each retained decision operand projection is copied
through `decision_place`, independently paying its backing rather than cloning
an already-paid source place for free. Predicate order and cleanup are unchanged.

`admit_cfg_push` pays insertion Work1 even with spare capacity. Growth also pays
copy Work proportional to the existing length. The full requested replacement
backing is prepaid while previous admission remains charged. Allocator surplus
is admitted before element writes. This is conservative monotone overadmission,
not a physical allocator peak measurement or a refundable scratch lease. No new
budget, cap, mutable published receipt or storage backreference is added.

Block, statement and live signatures stay unchanged, including the existing
BlockId(0) error sentinel. The first resource error latches before indexing or
writes. Live pre-admits the inner scope backing before StorageLive, then checks
again before the LocalId push. Decision lowering checks immediately after block
and temporary calls before dependent indexing. Failed larger spare capacity
cannot be reused after the latch. The existing final lowering publication guard
remains mandatory.

Cleanup uses nested reverse indices, preserving inner-scope-before-outer-scope
and reverse-local order without the previous collected temporary Vec.

## All ten scope-call failure paths

Each replacement is directly `push_scope()?` in a Result-returning function:

1. Simple `pattern_test`, before comparison and scope reads.
2. `decision_pattern_test`, before temporary construction or indexing.
3. `match_expression` arm, before bindings/body/cleanup/pop.
4. `let_else` failure body, before expression/cleanup/pop.
5. `body_inner` entry, before statement traversal.
6. `body_inner` statement Drop scope, before statement matching.
7. While-condition scope, before condition expression and later pop.
8. Lazy LHS Drop scope, before operand expression and later pop.
9. Lazy RHS scope, before operand expression and later cleanup/pop.
10. If-condition Drop scope, before condition expression and later pop.

No caller catches a failed insertion and continues to a scope index or pop.
`body` and `materialize_expression` wrappers decrement only lowering depth and
return the same Err. `expression` propagates Err before consuming a place. The
outer lowering owner checks its first resource latch and the body result before
cleanup/final publication. Existing scopes can remain on error and drop with the
Lowerer, rather than incorrectly popping a scope that was not inserted.

## Controls and exact red-to-repair distinction

New controls cover full actual geometric backings for pending blocks,
statements, scope headers and LocalIds; spare-capacity insertion Work exhaustion;
explicit surplus-stage rejection; sticky first failure and no extra semantic
push even with invalid would-be indices; retained projection copy denial before
emission; decision failure with no returned CFG; and exact cleanup order.
The explicit surplus-stage control does not claim the system allocator rounded
a particular try_reserve_exact allocation.

Red receipt `20261019-010033657-88030` has 131 CFG passes and one legacy exact-byte
fixture failure: first panic occurrence 4948 bytes versus old element-only 4636.
The extra 312 consists of the new LocalId backing (8) plus one extra PendingBlock
(304), since the new producer pays the full two-block backing instead of one
block element. Charges were not removed or weakened.

The test-only repair independently sums full actual new capacities for both
successful panic materializations, retaining two separately allocated payload
clones. It pre-admits the real third occurrence's scope/statement backing without
pushing elements, asserts those exact receipts, then tightens the same source
publication ledger to payload_bytes minus one. StorageLive fits admitted spare
capacity, and the third full payload request fails before cloning with the exact
original first-failure and retry assertions. No allocator hardcode or allowance
fudge is used.

## Final pre-owner-replay scoped receipts

All three final commands explicitly report `where remote on hosted`:

- `20261019-010409060-103902`: 133 existing/focused CFG tests passed, including
  all eight new helper controls and the repaired exact panic receipt fixture.
- `20261019-010417384-104773`: 65 OR/source-authority/pattern tests and all ten
  unchanged graph-integrity gates passed, including the 1208 required-function
  boundary and its existing 1209 negative control.
- `20261019-010434619-105933`: strict all-target clippy exited 0.

The earlier red `20261019-010033657-88030` remains a red receipt, not replaced
or reclassified by these later runs. No full suite, local fallback or push ran.

## Explicit remaining inherited debt

This does not close all CFG allocation accounting. Initial lowerer containers,
other source-place clone paths, final block conversion and other helper families
remain outside this repair. In particular `cfg/drop_normal.rs::live_binding`
has a separate direct scope push and is not changed by the no-binder OR helper
repair. No fully paid CFG or end-to-end performance claim follows.
