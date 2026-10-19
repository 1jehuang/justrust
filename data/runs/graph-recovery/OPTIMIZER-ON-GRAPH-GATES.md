# Optimizer-on graph gates, central validation

This test-only increment is based on `37f6ee20b`, whose compiler sources are
unchanged from accepted `af5c06a69`. It imports the graph tests and kind names
from private `11fc496ea`, not the unfinished accounting repair `67e190636`.

Review found that the proposed work test used
`WORK_GROWTH_CEILING.max(2.2)`. Central integration removed that loophole before
validation: the optimizer-on test uses the existing **1.81** ceiling directly.
No existing ceiling, size requirement, debt entry or allow-list was raised.

Central forced-hosted validation:

- `20261018-225627878-2362954`: all 10 graph-integrity tests passed, including
  the existing 1208/1209 size ratchet and three optimizer-on tests.
- `20261018-225646238-2363787`: strict all-target clippy passed.
- `cargo fmt --check` and `git diff --check` passed.

Observed graph/accounting metrics, not wall-time speed claims:

- Optimizer-on build work growth from 16 to 32 leaves: **1.603**, below 1.81.
- Leaf dependency-closure growth: **1.000** for flat and inlining-driver inputs.
- Driver IPO_OPT fan-in: **34 -> 66**, exactly 16 additional SUMMARY reads and
  16 additional LOCAL_OPT body reads. These fixtures inline every leaf.

These tests do not prove that every optimizer allocation or work operation is
accounted correctly. Independent review of the separate repair found blockers
recorded in `OPTIMIZER-ACCOUNTING-REPAIR-REVIEW.md`; that repair remains private.
The optimizer remains off by default. No compiler production code or inline
policy is changed, and no new complete-suite or performance claim is made.

Only the two owned central run summaries are copied to
`optimizer-on-graph-gates.jsonl`. The implementation owner's mutation-test
observations remain owner evidence, not independently rerun central results.
