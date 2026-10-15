# Cold query-call provenance gap, 2026-10-15

## What the real capture establishes

The validated cold `itoa 1.0.18` capture in `bench/results/frontend-mapping-20261015/final-cold-large.json` has no read-record loss, but its 217,244 direct dependency reads are all `disabled`. This does not mean that Rust computations have no dependencies. It means that the compiler's incremental dependency graph is not recording them in this execution mode.

`rustc_incremental/src/persist/load.rs::setup_dep_graph` selects no incremental graph data when incremental compilation is absent. In `rustc_middle/src/dep_graph/graph.rs`, `read_index_with_request` initializes its diagnostic outcome to `disabled` and enters native `read_deps` only when graph data exists. The diagnostic observer reports that state honestly instead of changing compiler flags.

## Why counting existing dep reads is insufficient

`rustc_middle/src/query/calls.rs::query_get_at` creates an actual `QueryReadRequest` around the query call. A first-cache hit records the cached result's `DepNodeIndex` through `request.cache_hit`, even if the incremental graph subsequently reports a disabled read. A dispatch miss calls `QueryVTable::execute_query_fn` without forwarding the request token.

`rustc_query_impl/src/execution.rs::execute_query_non_incr_inner` receives the resolved `(value, index)` from `try_execute_query`, but currently returns only `.0`. A nonincremental provider has a virtual result index. Losing that index at the wrapper is a diagnostic protocol gap, not a language requirement.

However, not every dispatch is a fresh provider execution. It can second-hit the cache, wait for another producer, or encounter a cycle. Therefore it would be incorrect to label every wrapper return as an executed provider, or to reconstruct a request-to-job edge merely from admission-parent nesting.

## Correct next hook

A separate observed query-call graph should carry the actual request token through the private `QueryVTable::execute_query_fn` dispatch protocol, including macro-generated vtables. Record branch-specific result-resolution facts only after releasing existing compiler/cache locks:

- direct cache hit and race cache hit identify the actual cached result index;
- an obtained result associates its index with the actual publishing job before cache visibility, separately from proof that a provider ran;
- a waiter resolves to the value actually obtained, not the helper work it happened to execute;
- a green ensure describes a previous-session boundary, not a fresh local producer;
- cycle, feed, forced, interrupted and unobserved paths remain distinct;
- missing context or missing prior publication stays unknown.

Consumer correlation must use actual captured identities and ordered publication facts. Independent slice positions, nearby timestamps, and admission-parent relationships are not substitutes. If resolution facts and publication facts use different slices, add a common checked capture sequence or another explicit ordering authority. Never bind a later producer retrospectively.

Call-dependency coverage and rustc's incremental semantic dep-read coverage must remain separate metrics. A query-call graph is useful even when incremental tracking is disabled, but does not cover arbitrary non-query state, native libraries, backend optimization or the whole build pipeline.

Use actual call/return event or continuation nodes for the future execution DAG. Combining a whole-job parent-to-child edge with a child-producer-to-parent-consumer edge can falsely create a cycle for ordinary synchronous nesting. An admission job is not automatically an indivisible executable task.

## Acceptance requirements

Use real nonincremental and incremental source builds without adding `-C incremental` solely to improve the graph. Exercise direct and racing cache hits, true provider execution, waiting/helping, cycles, feeds, green promotion and unobserved boundaries. Verify one terminal resolution per actual request, independent allocator/loss accounting, no future-producer edges, unchanged diagnostics and exit status, and artifact-neutrality controls. A malformed or partial capture must not become a complete graph.

This document records the original broader implementation design and source inspection. The narrow nonincremental returned-index protocol is now implemented and correctness-validated in `dispatch-result-provenance-results-20261015.md`, without branch-specific provider/race/wait/cycle classification. Incremental result provenance, feeds, opaque work and complete execution prerequisites remain unresolved. CPU/clock instrumentation alone does not close those gaps.

The implemented consumer-only follow-up is documented in `cache-call-provenance-20261015.md`: actual cache-hit publication relations, retained cold/warm graphs, and exact replay gates. It does not implement the dispatch hooks above or establish a weighted execution DAG.
