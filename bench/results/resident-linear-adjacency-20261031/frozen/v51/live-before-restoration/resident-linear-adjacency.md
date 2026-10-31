# Resident native adjacency attribution, v51

## Source-only freeze and provenance

FROZEN_READY for root-owned native acceptance. This is licensed Blaze-derived
resident code, not the clean-room compiler. v50 is not promoted: root reports
mixed runtime results. v51 is an attribution diagnostic, not a speed candidate.
No native builds, tests, workloads, timings, commits, or pushes were run by this
worker. Source formatting and diff checks do not establish native correctness.

Owned paths, released together at freeze:

- `toolchain/resident/crates/hir-ty-codegen/cached.rs`
- `toolchain/resident/crates/hir-ty-codegen/Cargo.toml`
- `docs/resident-linear-adjacency.md`

Immutable v50 full original, candidate, document, patch and SHA manifest remain
under `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/linear-adjacency-v50/`.
All v50 manifest hashes were verified before v51 edits. v50 candidate SHA-256 is
`3cba6e0b6868ca2f827a59d64e3a91bd962c8b8958d7a51ab4ebf1ad5e8b1fe7`.
The earlier accepted pre-cursor source SHA-256 is
`11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f`.

v51 complete original/candidate files, patch, SHA manifest and readiness record
are under `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/linear-adjacency-v51/`.
The v51 original directory is the exact v50 source and documentation plus the
pre-v51 Cargo manifest. The incremental patch is against those originals.

## Default-free compile-time control

`hir-ty-codegen/native-adjacency-diagnostic` is a default-OFF Cargo feature,
with no default-feature activation. Every diagnostic type, field, initialization,
scan, counter, report and feature-only test is behind this compile-time cfg.
There is no runtime environment check, shared counter, per-edge event, or atomic
in the disabled path. The disabled selector retains the v50 raw cursor algorithm,
with its result bound locally before the unchanged DFS settlement branch.

The v50 production change advances a raw `funcs` cursor, skipping every
non-`Named` target once instead of restarting `named().nth(seen)` from zero per
edge. Named order and duplicate edges remain intact. SCC settlement, lookup,
native links, miss counters and fallback remain unchanged. `Entry::named` still
serves native prefetch. No full-key memo is proposed: `get` already derives its
key inside the `OnceLock` initializer.

## Exact same-visit shadow measurement

When the diagnostic feature is compiled in, each reached DFS visit holds a
separate `named_seen` cursor. Every selector invocation actually runs both:

1. Original: `funcs.iter().inspect(count_original).filter_map(Named).nth(named_seen)`.
   The counter is before filtering, so it includes non-named targets, every
   repeatedly rescanned prefix, and the final failed exhaustion scan.
2. Candidate: the existing raw-cursor `find_map`, incrementing its raw inspection
   counter for each target consumed, including skipped non-named targets.

The code asserts exact `Option<&str>` equality before the existing DFS processes
that edge or exhaustion. Only the candidate raw cursor drives the traversal.
The shadow never calls `get`, `links`, or `settle`, and never updates native miss
counters. Both counts therefore describe the same reached graph and query,
without cross-run concurrency or cache-state differences. These are actual
instrumented iterator visits, not a mathematical estimate of original work.

## Bounded report schema

One fixed-size report is emitted per `usable` call that traversed entries, after
success or transitive failure. Direct unavailable and settled lookups emit none.
Reporting errors are ignored. Panic/mismatch is a diagnostic failure, not a valid
completed measurement. The prefix is `native-adjacency-dual-v51`, and every report
contains `timings_invalid=true`.

All counters are local to the `usable` call. The only shared reporting operation
is the one final stderr write. There is no named-edge census or extra adjacency
pass for fanout. A test-only thread-local slot captures the last report for exact
native-fixture assertions and is absent from non-test builds.

Fields:

- `selector_calls`: both-selector invocations, including exhaustion.
- `yielded`: successful named-edge yields, including duplicates and backedges.
- `visits`: reached DFS entries, counted on their first selector invocation.
- `raw_lengths`: sum of `entry.funcs.len()` across reached visits.
- `max_raw_length`: maximum reached raw adjacency length.
- `fanout`: fixed raw-length histogram buckets `0`, `1`, `2..7`, `8..31`,
  `32..127`, `128..511`, `512..2047`, `2048+`. This is raw fanout, not named fanout.
- `exhausted`: visits whose selector returned `None`.
- `aborted`: `visits - exhausted`, the active unfinished visits on early failure.
- `candidate_inspections`, `original_inspections`: actual raw targets examined
  by their respective selectors. Neither includes native import/link scans.

## Native fixtures supplied, not executed here

The existing v50 `native_linear_adjacency_matches_original_dfs` semantic test is
unchanged except for cfg-only initialization of the new visit field. It compares
production traversal with the full original DFS using independent real native
sealed stores. Four 132-node cases cover 128-way mixed fanout/all six target
kinds, duplicate edges, SCC/backedge behavior, missing dependencies, checked
imports and copy symbols. It compares results, ordered `defines` callbacks,
all 19 counters and settlement maps after each query, including repeated queries.

The feature-only
`native_adjacency_diagnostic_counts_actual_shadow_inspections` uses real native
stores and calls production `Entries::usable`, asserting:

| Path | Candidate inspections | Original inspections | Calls | Yields | Visits | Exhausted | Aborted |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Mixed 128-way root, complete exhaustion | 768 | 49792 | 130 | 128 | 2 | 2 | 0 |
| Same root, missing dependency at named edge 65 | 386 | 12610 | 66 | 65 | 2 | 1 | 1 |

Each root has 768 raw targets, all six target kinds, with repeated edges to an
empty leaf. Both paths assert sum/max raw lengths and histogram. A subsequent
settled lookup asserts that no fresh/stale report is captured. These counts are
deterministic fixture requirements, not observed passing results yet.

## Identity, controls and interpretation required of root

Build a separately named `native-adjacency-dual-v51` diagnostic artifact with
`hir-ty-codegen/native-adjacency-diagnostic` explicitly enabled. Record the full
build command, feature controls, toolchain version, complete source snapshot
identity, owned-file SHA manifest and final whole-binary SHA-256 in root evidence.
Use separate build/artifact directories and the root's versioned native cache
identity controls so the diagnostic cannot be mistaken for accepted stock/v50
or reuse a non-diagnostic compiler artifact. This lane cannot supply a binary
SHA without violating the prohibition on native builds, so that gate remains
explicitly root-owned and incomplete.

Replay the existing native workflow using that diagnostic identity, preserving
root's workload/state controls, and retain all report lines by phase. Aggregate
inspection counts, reached raw fanout, visits, yields, exhaustion and aborts.
Do not infer scan attribution from workflow timing alone. Absence of traversal
reports means no observed adjacency work, not a measured win.

Dual-selector execution, counting, assertions and stderr reporting invalidate
all diagnostic timings for speed claims. Any later performance decision still
requires separate uninstrumented strict original/candidate measurements and the
project's applicable native correctness gates. This diagnostic is attribution
only and does not promote v50.
