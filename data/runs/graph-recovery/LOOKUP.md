# Immutable call-effect lookup checkpoint

This checkpoint removes a measured quadratic lookup, not a work-budget ceiling. It does not implement input-scaled `PlanLimits` or claim compile-time speedups.

## Measured attribution and work counts

On `dbcd2d350`, a temporary rejection backtrace in hosted run `20261018-195418816-1403990` identified `ANALYZE -> ownership::Transfer::block -> borrow::Plan::call_write_targets`. Each call charged and scanned the entire immutable effect binding list during ownership transfer, including the final diagnostic pass. A flat generated main has two bindings per generated leaf. The temporary instrumentation was removed.

The same generator, worker count 1, and source base were used before and after replacing that lookup. These are total metered planning work counts, not elapsed-time claims:

| Generated leaves | Before | After |
| --- | ---: | ---: |
| 128 | 3,644,518 | 3,517,305 |
| 256 | 8,329,659 | 7,814,096 |
| 512 | 21,363,860 | 19,286,187 |
| 996 | 59,274,638 | 51,380,253 |

Before: `20261018-195603130-1437967`. After: `20261018-195724476-1493292`, whose old known-failing-size assertion intentionally failed because 997 now compiled. Both used a temporary measurement loop that was removed. The new publication-order check adds one charged comparison per adjacent binding, while lookup pays each actual binary-search comparison before accessing the matching row. No approximate logarithmic charge replaces actual operations.

## Invariants and controls

The diagnostic pass constructs bindings in ascending block order. Publication now checks strict ordering, rejecting duplicates and reordered keys through the existing invalid-binding diagnostic path. The existing occurrence/source/canonical-function/facts authority conditions and contract validation remain required. No extra index allocation, mutable memo, or whole-program fact was added.

A real two-call generic source plan supplies binding fixtures. Tests cover first/last/missing/empty queries, duplicate and reordered rows, stale block and call identities, cloned rather than canonical function authority, missing facts authority, and sparse tables through 1024 entries. Each query succeeds within `floor(log2(n))+1` paid comparisons, and the worst absent-key probe fails at one less. Altered test rows are never published as compiler facts.

## Final integration and limits

After replay on `8434f8f05`, hosted check `20261018-200134827-1750667`, four borrow unit tests `20261018-200142775-1757109`, all five graph gates `20261018-200200106-1768689`, and strict all-target clippy `20261018-200217481-1783623` passed. Another 143 ownership unit tests (`20261018-200255264-1806577`) and 89 ownership/accounting/reference/loop integration tests (`20261018-200255876-1806838`) passed. Formatting and whitespace checks pass. Ten additional borrow/reference/carrier targets passed all 160 tests in `20261018-200319302-1824466`.

The exact adjacent size checkpoint is now 1165 compiling and 1166 failing at unchanged per-task Work 32,000,000, requested 1 at used 32,000,000. The binary-search probe `20261018-200013313-1653138` was followed by ordinary final graph acceptance on the integrated source. No resource ceiling or debt baseline was raised. Growth remains 1.684 within the unchanged 1.81 ceiling, leaf closure 1.000.

## Input-scaled limit design follow-up

A larger constant, or a quadratic allowance hiding this removed scan, is not a valid solution. Proposed admission is a checked sum of bounds derived only from the node's checked inputs: validation traversals, block transfers, edge merges, carrier relations, and cleanup dimensions. For ownership alone, retained lattice entry changes are bounded by the per-block bit dimensions `2*locals + 3*paths + enum_paths`, plus first reach. A complete work bound must also account for actual transfer costs, predecessor notifications, borrow reference/family/cell fixed points, type/projection traversal, and cleanup elaboration. This is not yet a proved complete bound and must not be used to raise admission until those terms and overflow/adversarial controls are enumerated. The shared/default 32-million limit remains unchanged in this checkpoint.
