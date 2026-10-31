# Resident cache DFS linear adjacency, v50

## Status and provenance

FROZEN_READY for root-owned exact acceptance. This is source-only work on the
licensed Blaze-derived resident engine, not the clean-room compiler.
No native build, native test, workload, timing, commit, or push was run by this
lane. Formatting and whitespace checks are not native correctness acceptance.
No runtime speedup is claimed. Root owns the prospective strict runtime baseline,
native differential gates, and before/after workflow measurements.

Owned paths, released at freeze:

- `toolchain/resident/crates/hir-ty-codegen/cached.rs`
- `docs/resident-linear-adjacency.md` (new)

Accepted original `cached.rs` SHA-256:
`11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f`.
Full original, candidate, document, patch, and SHA manifest are preserved under
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/linear-adjacency-v50/`.
The manifest identifies the frozen candidate bytes. The patch is limited to the
two owned paths and can be applied to the accepted original checkout.

## Narrow production change

`Entries::usable` previously requested `entry.named().nth(visit.seen)` for each
outgoing named edge. `named()` starts a `filter_map` over `funcs` from zero each
time. For an all-named fanout of N, successful requests inspect
`1 + 2 + ... + N` raw targets, and the final unsuccessful request scans N again.
Mixed target kinds add repeated prefix work as well.

`Visit::seen` now means the next raw `funcs` index. Each request starts at that
index and increments it for every inspected raw target, returning the first
`Named`. The next request resumes after that target. Non-named targets are
consumed but never become DFS edges. At exhaustion, `seen == funcs.len()`, and
an empty suffix takes the existing settlement branch. Each target of each
visited entry is inspected at most once by this adjacency selector. This bound
is about scan work, not elapsed time or total cache traversal complexity.

The clone keeping the current entry alive is retained. Edge order, duplicate
named edges, early exits, the SCC stack and low-link updates, settlement,
native `links` checks, all miss counters, and lookup/fallback paths remain
unchanged. `Entry::named` remains unchanged for native prefetch use.
No key memo is proposed: `get` already derives its key inside the `OnceLock`
initializer, on a miss rather than each lookup.

## Native differential fixture supplied, not executed

`native_linear_adjacency_matches_original_dfs` calls the production
`Entries::usable` and a frozen copy of the original traversal, each against
independent real native `Store` instances populated through encode, decode,
insert, seal, and readback. This is not a standalone graph simulation.

Four 132-node fixture variants exercise:

- 128-way mixed fanout with all six `CachedTarget` kinds, duplicate named edges,
  unchecked imports and valid copy symbols, plus empty adjacency leaves;
- a two-node SCC, an outgoing settled leaf, and a late cycle back to the root;
- a late missing named dependency;
- a late unavailable checked native import;
- a late unavailable copy symbol.

Every node is queried in order twice, so positive/negative memo hits and partial
settlement after failure are observable. After every query the test compares
return values, the exact ordered native `defines` callbacks, every one of the
19 miss-counter values, and each fixture node's settlement map value.
Explicit final assertions require exactly one transitive failure in each
failing variant and exactly one dependency-native-import failure in each
native-symbol failure variant. Existing native direct-miss/import and
negative-cache tests remain present and unchanged.

## Source checks and next gate

Only standalone `rustfmt --edition 2024 --config skip_children=true` on the
owned Rust file and scoped `git diff --check` were run. Root must execute the
exact native acceptance against the SHA-frozen candidate before treating these
fixtures as passing. Root must preserve the accepted original for strict
before/after runtime comparison. A successful differential test would establish
fixture equivalence, not general semantic proof or a runtime improvement.
