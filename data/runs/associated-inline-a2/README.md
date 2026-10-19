# Associated-function inline A2: central acceptance

Frozen source: `ad00489617fece93096543a6f927897cd3a4d2c8`, on published `fd2712074`.
Production change is limited to `source_attrs.rs`; workflow tests are in
`associated_doc_attributes.rs`. Original associated functions with an unambiguous
body may carry one validated `inline`, `inline(always)`, or `inline(never)` hint.
These are optional hints, not optimization or ABI guarantees. Required methods,
duplicates, ambiguous generic/macro headers, generated owners and other attribute
families remain explicitly unsupported. Existing docs-only admission is preserved.

## Exact owned checks

All IDs have prefix `20261018-` and are preserved in `central.jsonl`.

| ID suffix | Command / result |
| --- | --- |
| 232249371-2822908 | Forced-hosted `justrust check`, exit 0 |
| 232255680-2823242 | Forced-hosted `justrust test --lib source_attrs::tests`, 26 passed |
| 232311397-2825757 | Forced-hosted associated-doc/config/associated-item/integrity workflows, 24 passed across 4 binaries, including all 10 integrity tests |
| 232326876-2822814 | Forced-hosted `justrust clippy --all-targets -- -D warnings`, clean |
| 232352542-2829670 | `JUSTRUST_TEST_JOBS=4 justrust test --local --no-fail-fast`, 3505 passed, 0 failed/ignored/filtered, 196 binaries |
| 232928605-3034709 | Final forced-hosted strict all-target lint, exit 0 (fresh artifacts) |

`cargo fmt --check` and `git diff --check` passed. Local full acceptance is needed
for the pinned stock-oracle subprocess paths used by these tests. The full run was
contended (foreign optimizer tests/lint and 84.569 CPU-stall seconds); it establishes
correctness, not a timing improvement. Source stayed frozen throughout the run.

The separately committed `experiments/results/associated-inline-a2-20261019.json`
preserves 26 stock probes and the owner's scoped evidence. Its four extra fixtures
are explicitly unexecuted, not claimed as stock successes. No stripped core files,
whole-core support, corpus promotion, full Rust support or speedup is claimed.
The unchanged real-program corpus remains 3/20.
