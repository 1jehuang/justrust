# Rejected resident linear-adjacency experiment

**REJECTED for shipping. Production restored. No useful latency claim or default
promotion.** This is licensed Blaze-derived resident work, not clean-room work.
The portable evidence bundle is
`bench/results/resident-linear-adjacency-20261031/`.

## Mechanism and native correctness gates

v50 replaced `Entries::usable`'s repeated `entry.named().nth(visit.seen)` prefix
scans with a raw `funcs` cursor. It consumed each target once, skipped non-named
kinds, and retained ordered named edges, SCC settlement, links, native counters,
and fallback. It did not change prefetch's `Entry::named`. A full-key memo was
not proposed: `get` already derives its key inside the `OnceLock` initializer.

v51 added a default-OFF compile-time `native-adjacency-diagnostic` feature. It
actually ran the original nth selector and candidate raw cursor on the same
reached DFS visit, with separate cursors, counted raw inspections before
filtering, and asserted exact `Option<&str>` equality before native traversal.
It emitted one bounded aggregate per traversing `usable` call, with raw fanout,
visit, yield, exhaustion, abort and inspection counts. It did not perform a
named-edge census or use per-edge atomics/events. Every diagnostic report
explicitly marked timings invalid. This diagnostic is also rejected for shipping.

Root executed the actual native gates, retained as original journals:

- v50 original-workspace check/build passed, **54 codegen / 360 resident** tests
  passed, no failed/ignored/filtered tests.
- v51 feature-enabled check/build passed, **55 codegen / 360 resident** tests
  passed, no failed/ignored/filtered tests.
- Full 3757-file source manifests and production Cargo.lock checks passed after
  tests. Whole-source deltas were only cached.rs for v43 to v50, then cached.rs
  and the feature declaration in Cargo.toml for v50 to v51.
- The native differential fixture used independent real sealed native stores,
  132-node graphs, mixed 128-way fanout, cycles, missing dependencies, checked
  imports and copy symbols. It compared results, all 19 counters, settlement
  maps and ordered symbol callbacks against the full original DFS.
- The diagnostic native fixture asserted actual original/candidate scan counts:
  49792/768 on mixed high-fanout exhaustion, 12610/386 on early missing dependency,
  plus exact visits/calls/yields/exhaustion/aborts and no report for settled hits.

These are tested fixtures and gates, not a general semantic proof.

## Runtime decision and attribution limit

All **28** uninstrumented strict calls passed the actual semantic oracle. Most
large-state timing changes varied across the two counterordered pairs. Assertion
recovery was lower in both pairs: -15.4684% and -19.5029%, using
`100 * (candidate / baseline - 1)`. Root rejected the change because there was
no repeatable useful overall latency win. Noise remains **unknown**, including
daemon/background CPU and OS caches. Waited child CPU is not daemon CPU. These
scoped pairs are not the comprehensive session-weighted performance gate.

All **7** strict diagnostic calls passed. Visible worker-log snapshots show
actual reduced scan work on identical reached visits:

| Snapshot captured after | Candidate inspections | Original inspections | Original/candidate |
| --- | ---: | ---: | ---: |
| manifest-feature-add | 109830 | 416910 | 3.79596 |
| type-error | 109692 | 417574 | 3.80679 |
| type-recovery | 147623 | 671534 | 4.54898 |

Maximum observed raw fanout was 151. This is scan-work attribution only, **not
elapsed speed proof**. The worker logs reset or were replaced, and snapshots
overlap. Do not sum snapshot counts or describe them as fresh per-phase work.
The assertion-failure and assertion-recovery snapshots repeated the type-recovery
visible report counts unchanged, so they do not attribute the recovery timing
change to fresh cache DFS work. Zero visible reports do not independently prove
zero traversal work.

The initial delta-validator failed because it required monotonic cumulative
worker report snapshots. Its unchanged script, empty output and failure evidence
are retained. A separate corrected snapshot-only validator passed. Portable
replay reproduces both that original failure and the corrected analysis against
the exact bundled raw logs. It does not overwrite or silently repair the failed
analysis. No v51 diagnostic timing is used in the uninstrumented comparison.

## Restoration and evidence

Before restoring, all three exact live v51 paths were preserved. Only cached.rs
and its Cargo.toml were restored, from full originals rather than reset/checkout.
Both cmp and scoped `git diff --exit-code HEAD` passed at 2026-10-31T05:36:34Z,
and source ownership was released at 05:36:45Z. The restored cached.rs SHA-256 is
`11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f`.
The restored Cargo.toml SHA-256 is
`f46dec5b2cf06f30456f2379967d4bb1747c9e690ae35a1861d3775c95eab381`.
No experimental production code or diagnostic feature is shipped by this record.

The bundle retains complete v50/v51 original/candidate/doc/patch/hash/readiness
artifacts, actual native journals, binary/version/feature identity receipts,
source/lock before-after checks, all 35 raw calls and worker snapshots, exact
pinned63 workload and complete4020-file toolchain identity, failed and corrected
analyses, and post-measurement cleanup receipts preserving stores.

Its standalone Python-standard-library replay reconstructs input edits and
freshness challenges, reparses actual raw tests/diagnostics and invokes unchanged
copies of the original semantic oracle. Coherently re-sealed mutation controls
must fail after transport hashes pass. An isolated relocated run uses empty
HOME/PATH and forbids reads from the original checkout/scratch/toolchain plus
native/network calls. These checks are historical evidence replay, not native
re-execution or cryptographic provenance authentication. See the bundle README
for offline commands, identities and detailed scope.
