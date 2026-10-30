# Real literal-cache eligibility, October 30, 2026

This is a mechanism diagnostic on the pinned `f97fcc2` justrust library, not a
performance benchmark. It uses the frozen compiler from native acceptance
`89f61304b`, with procedural macro caching and the other experimental query
retention flags disabled. Each invocation actually runs the same four selected
unit tests. The edit executes the unique `JR_RUNTIME_EXECUTED_39` marker.

| Workflow | Eligible / successful lookup | Provider executions | Ineligible |
|---|---:|---:|---:|
| Prepare | 106 | 106 | 1,856 |
| Body edit | 106 | 0 | 1,858 |
| Restore | 106 | 0 | 1,856 |

All counts are exact resolved GenericActivity events from the main justrust
crate's native compiler profiles. The selected profile command lines are retained
and checked. Unresolved labels are explicitly counted and never attributed to
macro kinds. All three processes succeeded, four tests passed in each process,
all 63 pinned source files were restored, and tool identities remained unchanged.
The main shared compiler driver's SHA256 is
`5e7f545149e25586183b72fa3e1f101012f412732123852ba9eaec8c3705e04a`.

This proves the 106 admitted expansions avoid provider execution after a real
body edit. It does **not** prove that those operations were expensive or that
lookup, key hashing, token copying and disk decoding are cheaper. The separate
prospective `br37` timing comparison did not show a latency improvement:
owned-off median 1.472484 seconds, declarative-only 1.580672 seconds. Those are
scoped descriptive observations, not a statistically established regression or
an overall speed claim. The flag remains off. Most invocations were rejected by
the deliberately narrow literal-only admission boundary, motivating a separately
audited token-tree-only hypothesis rather than enabling unsafe procedural caches.

Complete state, original runner, raw test journals and three losslessly compressed
main-crate profiles are in `bench/results/declarative-real-eligibility-20261030/`.
Run its `validate.py` to verify all file hashes, raw test markers and exact profile
counts. Dedicated exported run summaries are under
`data/runs/xps14-declarative-real-eligibility-20261030/`. The runner's hard-coded
private paths record original provenance, not a portable execution promise.
The earlier `dp38` setup attempt failed before compilation because the private
runner omitted an optional `wild=None` argument. It did not alter source bytes or
establish a native result. The corrected run is `dp39`.
