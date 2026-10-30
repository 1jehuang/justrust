# Demand-query cache: real workload mechanism diagnostic, 2026-10-30

## Result and limits

The default-off `-Zjr-demand-query-cache` experiment skipped end-session
promotion on a real justrust library-test edit and still encoded demanded query
values. **This is mechanism evidence, not a total-wait speedup.** The separate
br42 five-edit timing diagnostic measured median demand-on **1.430772 s** versus
baseline **1.421465 s**, so it did not demonstrate a win. Preserve that negative
result rather than substituting compiler phase differences for end-to-end wait.
No default promotion is justified. The experiment is not the architectural
endpoint. Comprehensive session-weighted gates remain necessary for any general
performance conclusion.

## Workload and controls

Root's dp43 run executed `justrust test --lib -- summary::tests::
--test-threads=1 --nocapture` in a pinned real-source copy. Separate off and on
stores each ran prepare, executable edit, and restoration: six successful test
commands, each reporting exactly these four tests:

- `summary::tests::parses_libtest_results`
- `summary::tests::shares_overlapping_time`
- `summary::tests::splits_passes_without_double_counting`
- `summary::tests::test_failures_are_not_compile_errors`

The unique edit inside `shares_overlapping_time` evaluates
`std::hint::black_box(43u64)`, asserts its result, and prints
`JR_RUNTIME_EXECUTED_43`. The raw journal contains exactly one runtime marker
for each edit and none for preparation/restoration. This is an executable edit,
not a comment-only invalidation. Original and edited source bytes are retained.
The runner's before/after receipts match for all 63 source files and all recorded
tool identities. This is historical receipt consistency, not a claim that an
auditor can independently attest process provenance or recover missing binaries.

The off configuration omits the demand flag, exercising its false default.
On passes `-Zjr-demand-query-cache=yes`. Both explicitly disable the other three
experiments: `jr-empty-lint-level-fast-path`, `jr-cache-small-query-values`, and
`jr-cache-associated-items`. Self-profile captures generic activities and
function arguments. Remote/cache shortcuts are disabled for this controlled
local mechanism diagnostic, not as normal build policy. Frozen driver SHA-256:
`ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`.

## Observed main-crate intervals

Milliseconds below are per-label, per-thread interval unions, not sums of
nested events. All listed main-crate activities occur on thread 3. Persistence
contains serialization, which contains encoding. Never add those nested
columns. Green-provider recomputation can overlap other recorded activities and
is not an avoided-work estimate.

| Mode | Phase | Persist | Promotion | Serialize | Encode | Green recompute union (count) |
|---|---|---:|---:|---:|---:|---:|
| off | prepare | 97.818 | 0.001 | 97.763 | 90.957 | absent (0) |
| off | edit | 128.603 | 50.067 | 77.987 | 72.890 | 73.879 (47,683) |
| off | restore | 108.670 | 49.875 | 58.328 | 54.098 | 83.981 (47,594) |
| on | prepare | 96.410 | absent | 96.350 | 88.982 | absent (0) |
| on | edit | 55.100 | absent | 54.704 | 50.473 | 85.266 (47,683) |
| on | restore | 40.295 | absent | 40.027 | 36.254 | 84.154 (47,594) |

Every off profile has exactly one `incr_comp_query_cache_promotion` and no skip
marker. Every on profile has exactly one
`jr_demand_query_cache_promotion_skipped` and no promotion activity, including
cold preparation. The skip marker lasts 36, 29, and 36 ns respectively: positive
marker durations are not estimates of time saved. Every profile still has one
`encode_query_values`, one serialization activity, and one persistence activity.
All six retained main profiles decode with zero unresolved records. Equal green
recompute counts here do not prove equal work or rule out future-demand costs.

Preparation includes dependency construction and the profiled invocations have
unknown contamination. These six commands were deliberately a mechanism
diagnostic, not randomized or replicated latency evidence. Phase reductions do
not establish a total win. br42's independently retained raw timing rows and
summary preserve the no-win observation, including other stacks as contextual
receipts, not endorsements of their correctness or speed.

## Retained evidence and portable audit

All new evidence is under `bench/results/demand-query-real-20261030/`:

- Six main-crate `.mm_profdata.gz` files only, losslessly compressed with
  deterministic gzip headers. No dependency profiles or build outputs.
- Six unabridged `*.raw-output.jsonl` journals with original run IDs and hashes.
- `state.json`: flags, exact event counts, interval unions, source hashes,
  compiler/tool identities, journal receipts, and row status.
- `summary.rs.before`, `summary.rs.edited`, and historical `runner.py`. The
  runner contains local scratch paths and is retained for explanation, not
  executed by the portable auditor.
- Dedicated `exports/dp43/*.jsonl` produced with `justrust export` from only
  the two dp43 recording homes. Global monthly aggregates are untouched.
- `br42-records.json`, `br42-summary.json`, and `br42-metadata.json`, retaining
  the separate total-wait negative result.
- `retained-sha256.json` covers the originally copied receipts and compressed
  profiles. It is integrity bookkeeping, not a signed attestation.

From a repository checkout, with Python 3 and no native tools or scratch files:

```text
python3 bench/results/demand-query-real-20261030/audit.py
```

The auditor decompresses all six profiles, verifies raw hashes, decodes metadata
and every record, and exactly rederives recorded interval unions/counts. It
checks flags, crate identity, both branch-marker contracts, all four test names,
unique runtime edit execution, source restoration and tool identity receipts,
six dedicated exported run IDs, and the br42 five-edit medians. It does not run
native builds or tests. The retained `audit-result.json` reports PASS after
decoding all six main profiles. Eight fast receipt-mutation tests also pass:

```text
python3 bench/results/demand-query-real-20261030/test_audit.py
```

Existing native semantic acceptance is documented in
`docs/native-demand-query-cache.md`; dp43 adds real-workload mechanism evidence,
not a replacement semantic or comprehensive performance gate.
