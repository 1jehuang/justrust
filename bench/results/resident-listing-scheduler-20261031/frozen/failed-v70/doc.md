# Resident listing scheduler v70 (source-only hypothesis)

## Status and evidence boundary

On 2026-10-31 root reported roughly four seconds in roots with many-crate listing. This is root's measurement, not a new measurement by this worker. The hypothesis is that nested OS thread creation and oversubscription contribute: `Listed::of_all` distributes crates across available CPUs, `ListedCrate::new` calls `declared_in` and `exported`, and `declared_in` repeatedly calls `lower`, which otherwise distributes bodies across available CPUs again. No speedup is claimed.

Accepted original `main.rs` SHA256:
`0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176`.
The original was copied before editing and its digest verified. Ownership is limited to `toolchain/resident/crates/ra-jit/src/main.rs` and this new document. Panda's evidence worker uses frozen v69, not live main.

## Routing

`RA_JIT_LISTING_FLAT_THREADS=1` is exact and default OFF. A process-global `OnceLock<bool>` samples the environment on the first production routing use. Missing, non-Unicode, empty, `0`, `true`, `01`, and whitespace-suffixed values do not enable it. Later environment changes do not change the production decision.

A scoped thread-local depth marks actual multi-crate worker closures in `Listed::of_all` and `resolve_all`, only when their effective worker count is greater than one. `resolve_all` can reach `declared_in` for language crates after finding impls. Its `warm_impls` and `warm_files` call `on_idle_threads`. These helpers may perform queries that reach listing, so the existing outer-worker context is captured explicitly at that actual helper spawn boundary. The parent executes its own work with its existing context. No general context is inherited by arbitrary spawned threads.

Only `lower` and `exported` branch on the combination of the flag and positive depth. The flattened lower branch calls the same `Body::of` for every input body and returns the original vector, unchanged in order. The flattened exported branch uses the existing sequential attribute predicate closure for every input function, in original order. There is no body count cap, pruning, body skipping, added memoization or declaration graph change. Direct calls and calls under only one outer worker retain original threading. Original OFF lower/exported bodies remain byte-for-byte after their inserted early branches.

`resolve_names`, `verify_impls`, compiler/codegen workers, and all unrelated pools are untouched. `on_idle_threads` retains its original scheduling, helper budgeting, zero-helper behavior, work distribution, and query closure. Only the listing context is propagated to its existing helper closures. `warm_impls` is not itself serialized. This matters because its helper work is intentional when dependencies leave other cores idle.

The guard restores the exact prior TLS depth on normal return and unwind. It does not catch or replace cancellation. No new global lock is held over a query. Existing outer joins and OFF-path panic handling remain in place. Flattened queries execute on the current attached database thread, with normal Salsa unwinding rather than new error suppression. The existing lower `PROBING` propagation is unchanged on the original path, while flattened calls naturally retain the current thread's value.

The first actual flattened branch writes `listing: v70 flat nested branch activated` once per process. Write failures are ignored. This marker proves activation only. It is not a worker count, body count, timing measurement, or proof that any particular invocation would otherwise have spawned workers.

## Correctness argument and tradeoff

The optimization changes scheduling, not input identity, query keys, query dependencies, outputs, or retained caches. Every original module/body traversal and every body query remains. `body_declarations` and `module_declarations` remain Salsa tracked with unchanged implementations, so edits still invalidate the same dependencies. No new memo or stale-cache invalidation rule is necessary. Existing cache locks in `Listed::of` and `of_all` remain released before construction and query work.

Serializing the inner loops can lose useful parallelism near the tail when only one expensive crate remains in a pool originally started for multiple crates. Other crates may be idle while that worker still lists. This is deliberately an opt-in hypothesis, not a promoted default. Root must judge total agent wait, cold/incremental regressions, actual activation and results before promotion.

## Native fixtures added, NOT EXECUTED

- `listing_v70_exact_flag_and_unwind_isolation`: exact flag helper values, nested depth, unwind restoration, thread-local isolation, and restoration back to direct routing.
- `listing_v70_real_database_differential_order_and_routes`: fresh real `RootDatabase` fixtures for OFF+nested, ON+nested, and ON+direct. It creates 600 exports plus nested function bodies, deeper bodies, trait-provided bodies, const bodies and a static. It compares ordered function names, provided function names, export bits, static and kept-static names, rooted/generic output, provided/generic output and main selection. Test-only per-thread counters in the actual production `lower`/`exported` routing function assert which branches the real listing reached. No environment mutation or production counter overhead is introduced.

These fixtures exercise the production routing with the production guard in a controlled context. They do not independently prove that the outer pool closures are reached in root's workload, count worker spawns, validate cancellation end-to-end, or measure performance. Root owns actual compiler builds, fixture execution, full suites, counterordered timings, restore and promotion. This worker ran no builds, tests, workload probes, installs, commits or timing experiments.

## Frozen handoff

Freeze root: `<PRIVATE_EXPERIMENT_ROOT>/listing-scheduler-v70`.
`originals/main.rs` is the full pre-edit source. The document did not previously exist, recorded by `originals/resident-listing-scheduler.md.ABSENT`. `candidates/` contains both full candidates. `candidate.patch` is the full source-and-document patch. `SHA256SUMS`, `checks.txt` and `RELEASE` record integrity, static checks and the handoff boundary. After RELEASE, this worker stops edits. No artifacts are built or installed.
