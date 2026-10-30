# Resident demand-driven declaration lowering candidate

Status: REJECTED after coordinator-run native v6 mechanism/semantic gates and a fresh prospective suite. No useful latency benefit and no speedup claimed. Production main.rs is restored exactly to accepted commit c4a20a5f6. Only `toolchain/resident/crates/ra-jit/src/main.rs` and this new document belong to this change. The resident frontend is licensed Blaze-derived source, distinct from the separate clean-room compiler. Shipping defaults are unchanged.

## Hypothesis from the actual path

Coordinator-reported native v4 retained body-edit elapsed traces show roots 27.732 ms, apply 23.548 ms, check 12.490 ms, client planning 18.541 ms, and total 117.588 ms. These timings are nested elapsed intervals, not additive CPU costs or measured savings. The previous graph-reuse prospective comparison cannot establish a speedup because the matched reference improved too.

After tracked declaration nodes were introduced, `declared_in` still called `lower(db, bodies)` before asking `body_declarations` for each node. That helper unconditionally asked `Body::of` for every queued body, creating scoped lowering workers for batches of more than 16 bodies. This eagerly visits body queries even when tracked declaration nodes can validate without executing. The candidate removes that prewarming call and directly requests each tracked declaration node.

The initial caller search was incorrectly restricted to main.rs. Coordinator native v5 compilation rejected that candidate with E0425 at `outline.rs:356`, which calls `crate::lower(db, bodies)`. The failed receipt remains in `candidate-build-v5`. A resident-wide qualified-caller search confirmed this cross-module caller. The original `lower` helper has now been restored exactly from accepted commit `c4a20a5f6`, preserving outline prewarming. Only `declared_in` bypasses the helper. No outline files were edited. The separate `lower_until` path and all other lowering/compilation logic remain unchanged. Root must use fresh v6 receipt directories rather than overwrite the rejected build.

## Correctness contract

The same queued bodies are visited in the same order. `body_declarations(body, under_provided)` still calls `Body::of` and reads every block definition map and scope when the query executes. Salsa owns validation of those dependencies on later revisions. Macro expansion and structural changes remain database dependencies, with no outline approximation, manual epoch, or manual invalidation fast path. All function/static vectors, provided-method classification, and nested-body queues are unchanged.

This changes the scheduling of lowering, not which inputs or checks are required. It removes eager parallel warmup, which might regress cold loads or edits that genuinely invalidate many bodies. A cached query can still require dependency validation that lowers a changed body. Avoiding a direct `Body::of` call is not proof that its work disappeared. Reject the candidate if matched native measurements show a regression or no useful benefit.

## Test-only recomputation evidence

The exact original eight revision sources and uncached full-traversal comparison remain intact in `resident_graph_reuse_tracks_nested_items_and_macro_edits`. Added assertions do not replace or weaken any existing assertions:

- A test-only thread-local counter increments inside the actual native Salsa `body_declarations` implementation, not at its call boundary. It is absent from production builds.
- The initial cold walk must execute at least one tracked body declaration query.
- A second complete listing in the same revision must produce the same declaration graph and execute zero additional body declaration queries.
- The fixture prints the actual query-execution delta for every revision, exposing cross-revision behavior without assuming unchanged-body cache effectiveness that has not been measured.

Each counter is isolated to its test thread, and declaration queries execute synchronously there. The counter reports body-declaration query executions, not `Body::of` execution counts, skipped semantic checks, elapsed savings, or production reachability counts.

## Root acceptance gates

Native builds and workloads remained coordinator-owned. The coordinator subsequently authorized an evidence/documentation-only commit for the rejected result, not the candidate source. Coordinator must run the native graph fixture with captured test output, the existing listed-root fixture, telemetry filters and exact runtime/proc-macro/diagnostic gates. Compare a fresh prospective baseline and candidate with identical workload/edit sequence and execution mode, keeping telemetry identical. Inspect roots and end-to-end elapsed distributions together with the matched source-built reference. Include cold-start and structural-edit regressions because eager parallel warmup was removed. Source-only rustfmt and whitespace checks were performed before native gating.

## Actual rejection and preserved evidence

Native v6 build and all graph/listed-root/telemetry unit filters passed. The exact eight-revision fixture reported actual tracked body-declaration query execution deltas `7, 1, 3, 2, 3, 3, 2, 1`, with cold execution and zero additional same-revision executions asserted. This establishes a query reuse mechanism, not a latency benefit.

The actual fresh `rsa01 -> rsa02` five-edit suite passed runtime gates. Engine median changed `0.109617051 -> 0.110432806` seconds, descriptively 0.74% slower. The unchanged reference binary measured `0.110923157 -> 0.110535542` seconds. Reference-normalized engine/reference ratio changed `0.988225 -> 0.999071`, descriptively 1.10% worse. Noise is unknown. This scoped diagnostic establishes no useful benefit and no significance claim. The candidate was rejected rather than promoted.

The full candidate diff/source/hash, original failed v5 receipt, repaired v6 build/unit/query logs, scripts, actual before/after suite artifacts, and worker logs are preserved in `bench/results/resident-demand-lowering-rejected-20261030/`. Its portable `audit.py --self-test` checks content hashes, exact patch reconstruction, native gate receipts, actual query counts, suite artifact integrity, gate statuses, recomputed medians and unchanged reference identity without native builds or access to the original host paths.

Only the uncommitted main.rs delta was reversed through scoped edits, not reset/checkout. Exact restoration to `c4a20a5f6` was verified by an empty git diff and SHA-256 `471f0b5c10187ecbea879acfe3e5eae52925161e32bce6feeebe49b67af0368f`. The rejected candidate SHA-256 is `7792413378f3508b3d3692ec4ea9e2046ebb8adfd6464756be5c40131eda1855`. No candidate source change is committed as production code. Shipping defaults and unrelated work remain untouched.
