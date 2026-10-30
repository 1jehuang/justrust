# Native per-item dead-code scan retention

## Status and measurement scope

This is an opt-in experiment, not a default promotion or a speedup claim.
`-Zjr-cache-dead-code-items` defaults to `no` and is a tracked compiler option.
The retained body-edit profile
`bench/results/provider-cost-20261030/body-edit-provider-cost-unknown.json`
reported approximately 139 ms **inclusive elapsed** in
`live_symbols_and_ignored_derived_traits`. That includes nested queries. It is
neither exclusive scan cost nor time saved by this change.

The architectural lesson under test is to retain an item's usage edges while
recomputing the whole-crate reachability fixed point. The experiment uses the
native incremental query cache, not a daemon, private environment cache, or
an alternative lint implementation.

## Native boundary and dependency contract

The original `MarkSymbolVisitor` in `rustc_passes/src/dead.rs` remains the source
of scanning semantics. The new providers call exactly
`visit_node(tcx.hir_node_by_def_id(id))` with a fresh instance of that visitor.
The flag-off branch still calls the original visitor directly.

Two queries encode the logical `(LocalDefId, propagated allow/expect)` key:

- `jr_dead_code_item_usage(LocalDefId)` for ordinary context
- `jr_dead_code_item_usage_allow_expect(LocalDefId)` for propagated context

Separate query kinds preserve native LocalDefId dependency-node key recovery.
This source tree supports `cache_on_disk`, not a per-key cache predicate.
Both queries use `arena_cache` and `cache_on_disk`, and only the enabled
consumer invokes them. They use normal query dependency recording, stable
hashing, serialization, and diagnostic replay. No diagnostics are suppressed.

The summary preserves the produced worklist's exact order, including each
edge's propagated and own allow/expect booleans. `live_symbols` remains an
unordered membership set, never a sorted substitute for the worklist.
Ignored derived traits retain their original per-ADT `FxIndexSet` insertion
order. Independent ADT map keys are sorted only for merging, without changing
the order within any diagnostic's trait list. Existing `LocalDefIdSet` and
`LocalDefIdMap` are unordered collection wrappers with order-independent stable
hashes and codecs. `FxIndexSet` stable hashing and serialization preserve its
observable insertion order.

The global pass still owns and executes, without moving them into the query:

- constructor-to-item adjustment, root insertion and own allow/expect handling
- scanned-set suppression, including ordinary context subsuming propagated context
- the synthesized RPITIT special case
- unsolved trait/impl reachability, its fixed point, and deferred root phases
- both liveness snapshots and all dead-code diagnostics

## State-dependence inspection

Within the node scanner, `live_symbols` is only written. The reads which make
impl/trait liveness conditional on previously discovered symbols occur in
`check_impl_or_impl_item_live`, outside the node scan. Node scanning only
appends work and inserts ignored derived traits. It never reads existing
worklist, scanned, or unsolved-item contents. Therefore a scan's result is
additive and replayable, without a graph-state key or speculative graph reuse.

Each query starts with no typeck results, no pattern context, an empty ignored
variant stack, and both repr flags false, matching successful global scan
boundaries. The original visitor saves/restores typeck results around nested
bodies, saves/restores repr flags around nodes, truncates the variant stack
after arms, clears pattern context after patterns, and restores it around
inline/anonymous consts. A tainted typecheck still returns its original
`ErrorGuaranteed`. Query errors abort the global traversal as before, rather
than returning or caching an apparently successful partial scan.

## Differential acceptance

Run only after the coordinator has finished measurement/bootstrap and frozen
one compiler and its driver/sysroot:

```sh
python3 -m unittest bench/test_reuse_dead_code_accept.py -v
python3 bench/reuse-dead-code-accept.py --rustc /absolute/frozen/rustc --output /new/private/output
```

The native runner records the compiler and driver hashes, verifies incremental
stable hashes with `-Zincremental-verify-ich`, and executes 32 ordered revisions.
Each revision has seven runs: fresh no/yes, persistent no/yes, and no/yes/no
using one shared transition cache directory. The tracked option may correctly
invalidate that directory across transitions. Persistent yes exercises actual
same-option incremental reuse across unchanged and edited revisions.

All invocations use exactly the same source and binary paths. Status, stdout,
and stderr must match the fresh flag-off reference **byte for byte**, including
warning ordering, spans, expectation messages and line endings. There is no
diagnostic or path normalization. Successful binaries must additionally have
the expected exact runtime output. Expected compiler errors must exit with
status 1, never a crash, timeout, spawn failure, or silent success. Failures
write a failed/incomplete evidence report, not a success verdict. Base64 fields
retain arbitrary raw diagnostic bytes in JSON. Recorded elapsed times are
observations of this correctness matrix, not a performance comparison.

Coverage includes body/callgraph edits and reversions, unused roots and fields,
repr(C)/transparent types and unions, trait/inherent methods and RPITIT,
derived Debug/Clone ignored reads and derive order, nested closures and async,
macro expansion, owner churn, enum patterns, propagated allow/expect (including
expect(unused)), fulfilled/unfulfilled expectations, deny errors, type errors,
and recovery in the same persistent caches.

Python unit tests validate the harness machinery with mocks. They are not
native compiler acceptance. Native results, matched before/after measurements,
and the comprehensive session-weighted synthetic gate must be recorded before
considering any default promotion.

### Native acceptance on October 30, 2026

After the coordinator's bootstrap and frozen-compiler release, all 32 revisions
and all seven modes passed, totaling 224 compiler invocations. Every successful
program also produced its expected exact runtime result. The three intentional
error revisions produced matching ordinary status-1 failures in every mode.
All runs used incremental stable-hash verification. The ten Python harness
tests passed separately.

The private raw evidence is
`/home/jeremy/.jcode/scratch/dead-item-cache-20261030/native-acceptance/acceptance.json`.
Its SHA-256 is
`92f5e5b547dd1653145e266ac0b36afc8aa201ef3ca329bd30e911d1de5b9d10`.
The frozen compiler launcher SHA-256 is
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
The frozen driver SHA-256 is
`3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e`.
The coordinator owns preservation of raw measurement and acceptance artifacts.
This proves the covered differential behavior, not a performance improvement
or universal correctness. The option remains default-off.

## Attribution

The visitor, reachability algorithm, lint semantics, collection implementations,
and native query infrastructure are retained from this repository's owned Rust
compiler sources and their original Rust project attribution and licenses.
This experiment extracts and reuses the existing scanner rather than replacing
its semantics. The Python acceptance matrix follows the repository's native
retention-gate approach while intentionally avoiding diagnostic normalization.
