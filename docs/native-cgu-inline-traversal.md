# Native CGU shared inline traversal hypothesis

## Scope and status

`-Zjr-cgu-shared-inline-traversal=yes` enables a default-off, **tracked**
partitioning experiment. `no` retains the original per-root traversal and insertion
code. The only production source changes are in `partitioning.rs` and the option
declaration in `options.rs`. This is not the resident frontend coordination work,
nor a claim of a 10x frontend improvement.

The supplied retained dp43 off-edit profile reports
`cgu_partitioning_place_items = 54.79 ms` **elapsed**. That is neither CPU time nor
time saved. There are no before/after performance results for this change yet.
No compiler bootstrap or native workload was run while preparing these sources.
The native correctness gate belongs to the coordinator and passed after source
freeze, as recorded below. This permits committing the default-off experiment,
not a performance claim or default promotion.

## Audited graph contract

`UsageMap::for_each_inlined_used_item` reads the immutable `used_map` adjacency
vector and calls `instantiation_mode(tcx)` on every destination. It reports a
destination if and only if its mode is `LocalCopy`. It does not report global
roots, even when an inline item's adjacency includes a call to another root.
The experiment uses this method unchanged, rather than duplicating its filter or
walking `used_map` directly.

The root loop independently accepts only `GloballyShared` items. Its historical
comment mentions a LocalCopy entrypoint exception, but the actual match has no
such exception. `MonoItem::instantiation_mode` handles the executable entrypoint
as `GloballyShared` before other function heuristics. Statics and global assembly
are also always global. No new mode heuristic or root exception is introduced.

For each initial CGU, retain an ordered set of inline items discovered from all
roots processed so far. Expand every root unconditionally. For each reported
inline destination, insert before recursing, and expand only on first insertion.
An active cyclic node is already marked, but its remaining outgoing edges are
still visited by its original stack frame. This is graph traversal, **not** reuse
of a partially computed transitive closure. There is no per-root closure cache,
no reuse across CGUs, and no state that survives this partitioning invocation.

## Preservation argument

* The visited-set invariant after each completed root is the union of the old
  per-root reachable sets. A previously discovered inline item has already had
  all its filtered outgoing edges traversed before the next root starts.
* DFS and adjacency order are unchanged. Skipping an already completed subtree
  cannot hide a newly reachable item: all its descendants were discovered on
  the earlier visit. The newly appended ordered-set suffix therefore gives the
  same order of first CGU insertion as before. Indexed suffix access avoids
  repeatedly scanning the retained prefix.
* Root insertion, linkage/visibility computation, internalization candidates,
  and root size estimation remain before traversal and unchanged. Roots never
  enter the inline set. Inline insertions retain `entry(...).or_insert_with`,
  `inlined: true`, internal linkage, default visibility, and the original size
  query. An existing entry is never overwritten by the experiment.
* Inline size estimation still occurs after the root's complete traversal and
  only at first insertion, in the same first-discovery order. Final CGU size
  computation, stable CGU sorting, merging, and internalization are unchanged.
* `instantiation_mode` reads immutable session/type state and query results.
  Skipped calls are repeat checks of adjacency already fully examined in this
  same partitioning query. Every distinct required query dependency is still
  read by that invocation on first expansion. This does not promise identical
  query-cache hit counts or profiling events: removing repeat traversal is the
  experiment. It must preserve query results, dependency coverage, and emitted
  diagnostics. Existing size-query laziness is preserved rather than querying
  all reachable sizes speculatively.
* The option is tracked, so enabled and disabled configurations cannot silently
  reuse the same tracked incremental configuration. Tests must cover persistent
  on/off transitions as well as fresh directories.

These are source-level arguments, not substitutes for native acceptance checks
of diagnostics, query fingerprints, symbols, and deterministic external artifacts.

## Work and memory bound

Within each initial CGU, each root adjacency is expanded once per root occurrence
and each reachable inline adjacency once in total. Filtering still examines every
outgoing edge of each expanded item, including destinations rejected as global.
Expected traversal work is linear in those expanded vertices and edges, with
expected constant-time hashing. The off path keeps its original work.

The on path retains one set entry per inline item per initial CGU, proportional
to inline entries already represented by the CGU output, plus one map entry per
CGU with roots. No all-pairs or per-root transitive-closure maps are built. The
sets are released with `place_mono_items`. Across CGUs the same item is deliberately
visited again. Worst-case output replication across many CGUs is not eliminated.
Recursion depth remains bounded by the longest simple inline path, as in the
original implementation. This is not a stack-depth optimization.

## Source tests and acceptance gate

Enabled graph work emits the fixed-name `jr_cgu_inline_shared_traversal`
GenericActivity once per root traversal. At most one fixed-name
`jr_cgu_inline_shared_revisit` GenericActivity is emitted per partitioning
placement invocation, and only if a LocalCopy expansion was suppressed because
an earlier root in the same CGU already reached that item. Same-root diamonds
and cycles alone do not produce that marker. Detection uses the ordered-set
insertion index with no extra hash lookup, root/item strings, or per-edge
profiler events. Both activities are absent on the disabled path. Traversal
timing includes enabled-only reuse detection, not inline insertion/size queries.
The mechanism fixture must have at least two global roots in the same initial
module/volatility CGU sharing LocalCopy descendants. Final `-Ccodegen-units=1`
merges CGUs only after placement, so roots in separate modules alone do not prove
shared traversal. At optimization level 0, mark the shared chain
`#[inline(always)]`, not merely its leaf, to avoid an intervening global root.

The pure traversal helper has source unit tests modeling the destination-mode
filter. They cover DFS order in a diamond, self and mutual cycles, multiple roots
sharing descendants, an inline edge to a different global root, repeat root
expansion, equality with the old ordered per-root union, and independent CGU
visited sets. A separate test ensures only cross-root reuse qualifies for the
revisit marker. They are source tests only, not executed in this source-only lane.

Preparation checks are rustfmt on the two edited Rust files and scoped
`git diff --check`. The native acceptance requirements given to the coordinator were:
tracked flag parsing, fresh/persistent no/yes/no transitions, CGU counts 1 and 16,
incremental on/off, optimization levels 0 and 2, shared inline diamonds,
bounded recursive runtime cases, generic/drop behavior, duplicate-symbol
diagnostics and recovery, incremental query verification, and exact artifacts
where reproducible. Do not promote the default based only on these source tests
or the 54.79 ms observation.

## Exercised correctness result, 2026-10-30

After source freeze, the coordinator built the candidate and ran the native
acceptance harness. This source-only worker did not launch builds or native
workloads. Native production sources remained unchanged throughout acceptance.
The frozen driver SHA-256 before and after was
`29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`.

The first acceptance completed all 672 semantic compiles successfully, but
**failed the positive mechanism gate**: enabled cold/warm profiles had traversal
23 and revisit 0. All three exported roots were in separate initial module CGUs.
Their later merge into final CGU count 1 did not exercise cross-root traversal
reuse. This was a fixture coverage failure, not positive optimization evidence.
The original failed result was retained, rather than overwritten or reclassified.

The harness repair moved `root_b` into module `a` alongside `root_a`, retaining
module `c`'s root for isolation. The entire shared chain stayed
`#[inline(always)]`, which selects LocalCopy at optimization level 0. Ordinary
`#[inline]` would select global mode there. The normal MIR inliner defaults off
at this level, and mandatory ForceInline matches `InlineAttr::Force`, not Always.
No native implementation or marker condition was changed for the repair.

The coordinator reran full acceptance with repaired harness revision
`20e3f2a5e`: **672 semantic compiles and four independent mechanism profiles
passed**, with incremental query verification enabled. The semantic matrix
contained 12 revisions across CGU counts 1/16, incremental yes/no, and optimization
levels 0/2, with seven fresh/persistent/transition modes per case. Exact diagnostic
bytes, status, runtime, and applicable deterministic artifacts were compared.

| Mechanism profile | Shared traversal events | Cross-root revisit events |
| --- | ---: | ---: |
| Cold disabled | 0 | 0 |
| Cold enabled | 23 | 1 |
| Warm disabled | 0 | 0 |
| Warm enabled | 23 | 1 |

The four mechanism executables had the same SHA-256
`9db9a8bbc9fd5ddf77d5dc6f254f88a3411f13dcb79af54466d597590d0d3b6a`.
The local raw evidence is
`/home/jeremy/.jcode/scratch/cgu-inline-cache-20261030/acceptance-v2/acceptance.json`,
with four retained profiles under its sibling `mechanism-profiles/` directory.
The failed first report is under the sibling `acceptance/` directory, and the
focused fixture-repair proof is under `mechanism-repair-v2/`.

The mechanism counts demonstrate actual duplicate LocalCopy DFS suppression,
not CPU time saved or a speedup. Performance measurements remain separate and
pending. The flag remains default-off, and broader resident frontend coordination
is still needed.
