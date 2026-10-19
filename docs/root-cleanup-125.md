# ROOT_CLEANUP_DEMAND 125

The historical isolated prototype below was integrated as `ea45cd17f`. Central full acceptance passed 3,424/3,424 tests, formatting, and strict hosted lint. See `data/runs/root-cleanup/README.md` for exact receipts and scope.

## Historical isolated prototype

Base: a4776b447df249ccd4ffd4e590a6fad173ec3ad7. The following notes describe its pre-integration state.

## Scope and contract

Closed nongeneric root only. Canonical Source publication and exact owner are validated before independently deriving Drop demand from original local/expression/result types. The output retains only owner, closed CallTargets and immutable retained-storage receipt. ANALYZE/OBJECT read this fact without awaiting MONO_PLAN. Generic instance admission remains unchanged. Final CHECK still checks both original sealed cleanup demand and executable inventory membership.

No semantic facts come from a caller-supplied cleanup descriptor. Swapped owner, foreign publication, changed source, type arguments and same-snapshot swapped demand are rejected. Arc-backed type arguments share already admitted canonical storage; outer target slots and capacity are prepaid.

## Validation

- 650664yw3x: local check, four initial authority tests, nine normal-Drop unit tests, eight graph tests, and 29 stock/native Drop/trait-object/heap tests passed. Strict all-target lint failed because the isolated archive omitted examples/compiler/{hello,arithmetic}.rs. Restored exact base examples, no test bypass.
- 836697005o: seven of eight extended unit tests passed. Generic value-field fixture rejected at the existing Source publication boundary, before this feature. Replaced its positive control with supported generic raw-pointer field storage, preserving distinct i32/u8 destructor arguments.
- 880260ahq6: all eight extended unit tests, existing poisoned-receipt promotion test, eight graph integration/integrity tests, and strict all-target lint passed. Runs: 20261018-214126494-380706, 20261018-214135284-385267, 20261018-214138369-386611, 20261018-214204461-399523.
- 956657v91b passed unchanged public CLI profile/depth control, sealed global cleanup-demand/inventory negative control, and fmt check. Runs: 20261018-214239718-427806 and 20261018-214248197-433276.
- Resource controls cover checked count multiplication overflow, exact outer capacity admission, empty and duplicate candidate sets, and exhausted work/bytes preventing publication. Poison is tested at the unchanged reservation.retain boundary, without adding a mutable ledger test hook.
- Public graph test runs check/build with workers 1 and 8, verifies successful analyses and expected artifacts, and follows explicit task-to-node events to prove each root proof/object depends on 125 but not 113. All existing graph ceilings remain unchanged.

Local execution was explicitly approved because hosted synchronization previously failed ENOSPC. Runs overlapped other approved correctness jobs and are not performance measurements. No speedup or full graph-unification claim. No full-suite result claimed for this isolated prototype. Main integration and composition validation remain pending coordinator approval.
