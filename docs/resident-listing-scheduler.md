# Rejected resident listing scheduler v70/v71

**Rejected for performance, original source restored, no promotion.** The corrected v71 passed the actual 53 codegen and 366 resident native tests, including both new scheduler fixtures, and all 28 strict runtime rows. Two counterordered OFF/ON pairs did not show a useful replicated scoped latency improvement. Noise is unknown. These observations establish neither a speedup nor a causal regression.

The portable retained evidence is `bench/results/resident-listing-scheduler-20261031/`. This document describes a historical rejected candidate, not current production behavior.

## Hypothesis and implementation

Root reported roughly four seconds in roots while listing many crates. `Listed::of_all` assigns crates across available CPUs. `ListedCrate::new` calls `declared_in` and `exported`, and `declared_in` repeatedly calls `lower`. Both inner operations ordinarily create additional OS threads. The hypothesis was that excessive nested thread creation and oversubscription increased total wait.

The candidate introduced exact, process-first-use `RA_JIT_LISTING_FLAT_THREADS=1`, default OFF, sampled through a process-global `OnceLock<bool>`. Missing, empty, `0`, `true`, `01`, whitespace-suffixed and non-Unicode values did not enable it. Later environment changes could not change the decision.

A scoped TLS depth guard marked the actual multi-crate worker closures in `Listed::of_all` and `resolve_all`, only when effective worker count exceeded one. `resolve_all` can call `declared_in` for language crates after impl resolution. Its `warm_impls` and `warm_files` use `on_idle_threads`, so outer listing context was propagated only at those actual helper spawn boundaries. Helpers retained their original work distribution, budgeting and zero-helper behavior. `warm_impls` itself was not serialized.

Only `lower` and `exported` used this context to flatten their inner loops onto the current thread. Lowering called `Body::of` for every input and returned the original vector in order. Export detection used the existing sequential predicate closure for every input function in order. There was no body/function cap, item pruning, skipped traversal, query-key change or new memo. Existing OFF lower/exported bodies were retained bytewise after their inserted early branches. Direct listing and single-worker cases retained original threading. `resolve_names`, `verify_impls` and unrelated compiler/codegen work were unchanged.

The TLS guard restored prior depth during normal return and unwind. It did not catch or suppress cancellation. No new global lock spanned a query, and existing cache locks remained released before listing. The unchanged Salsa declaration/body queries retained their original dependencies, so no additional cache invalidation or stale-cache mechanism was required. Lower's original `PROBING` propagation remained on its original path, while flattened work naturally used the current thread's value.

The tradeoff was deliberate: serial inner loops can waste otherwise idle cores near the tail of a multi-crate pool when only one large crate remains. This default-OFF candidate was a scheduling hypothesis, not an assumed improvement.

## Failure, bounded repair and actual gates

The v70 source-only freeze was not a successful compiler gate. Root's actual check exited 101 with E0308: `WithFixture::with_single_file` returns an `EditionedFileId`, while `relevant_crates` requires a `vfs::FileId`. The full original failed check transcript is retained, with SHA256 `f2655e2a19d65e57bc6dd79e1cc44eedb8b97e6045056495799c0497376b5fa8`.

After inspecting the vendored API, v71 added exactly one fixture line, `let file = file.file_id(&db);`. Production bytes and assertions were unchanged. Standalone rustfmt parsing of the exact v71 main succeeded. The style check returned 1 with formatting differences, retained without applying unrelated formatting. v71 was explicitly source-uncompiled at freeze. Root subsequently ran the actual native gates successfully:

- 53 named codegen tests, run `20261031-060411424-1373945`.
- 366 named resident tests, run `20261031-060411827-1374092`.
- `tests::listing_v70_exact_flag_and_unwind_isolation`: exact flag helper, depth nesting, unwind restoration and thread isolation.
- `tests::listing_v70_real_database_differential_order_and_routes`: fresh real `RootDatabase` per OFF+nested, ON+nested and ON+direct mode. The fixture lists 600 exports plus nested/deeper functions, trait-provided bodies, const bodies and a static. It compares ordered declaration/output snapshots and checks test-only counters in the actual production routing function. It uses manually scoped production guards, not actual outer pool scheduling, so the fixture alone does not establish workload pool reachability or worker-spawn counts.

The native build source inventory contains 3,757 files and the exact corrected candidate main identity. Native after-checks verified source and root Cargo.lock unchanged. Every runtime case retained the same 63-file workload and 4,020-file pinned toolchain inventories before/after, plus per-row source inputs.

## Runtime observations and decision

Root ran OFF, ON, ON-repeat, OFF-repeat, each with seven ordered strict phases. Every row passed the original named/count/status oracle and fresh runtime/type/assertion nonce checks. The two pairs are descriptive, not causal estimates:

| Phase | OFF seconds | ON seconds | ON vs OFF | OFF-repeat seconds | ON-repeat seconds | Repeat ON vs OFF |
|---|---:|---:|---:|---:|---:|---:|
| Cold prepare | 20.275661 | 24.374780 | +20.22% | 21.238114 | 21.457317 | +1.03% |
| Repeated no-op | 0.041166 | 0.056251 | +36.65% | 0.042438 | 0.042786 | +0.82% |
| Manifest feature add | 9.583476 | 11.188168 | +16.74% | 10.080657 | 11.437991 | +13.46% |
| Type error | 9.329817 | 11.462884 | +22.86% | 10.027376 | 11.264074 | +12.33% |
| Type recovery | 8.669899 | 9.985041 | +15.17% | 9.357852 | 10.575751 | +13.01% |
| Assertion failure | 2.008111 | 1.992984 | -0.75% | 2.072844 | 2.188401 | +5.57% |
| Assertion recovery | 0.494530 | 0.522182 | +5.59% | 0.468342 | 0.517620 | +10.52% |

The actual `listing: v70 flat nested branch activated` marker was present once in every ON worker-log snapshot and absent in every OFF snapshot. It was emitted once per process from an actual flattened branch, ignoring stderr write errors. These cumulative snapshots prove activation only, not fresh phase work, worker counts, avoided thread counts or latency attribution.

The result did not satisfy the useful replicated scoped latency gate, so the candidate was rejected. The session-weighted synthetic suite was not run for this candidate, and no broad default-promotion claim is made. All four completed private namespaces were stopped after measurement with exit 0, preserving their stores. The evidence worker ran no native jobs, builds, installations or timings.

## Restoration and retained evidence

Before restoration, exact live main/doc were preserved and compared equal to frozen v71. At `2026-10-31T13:11:54Z`, only main was restored via the write tool from the immutable original. Comparison to the original and scoped `git diff --exit-code HEAD -- main.rs` both passed. Original SHA256: `0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176`. Source ownership was then released to root. This is a point-in-time restoration receipt, not a claim about later independent compiler changes.

The bundle retains full baseline, failed v70, corrected v71 and live-before-restoration Rust source; selected sanitized historical documents and full patches; the exact one-line repair patch; root reanalysis and cleanup receipts; original pure parser/oracle sources; complete named native results; and safe raw runtime output/diagnostics with private-original SHA/byte provenance. Full private worker/state/environment data, credentials, executables and build targets are excluded by fixed allowlists.

Portable replay uses only packaged files and Python's standard library. It reparses raw selected output with hash-bound original pure oracles without weakening tuple equality, independently checks exact test/status contracts, inventories, patch context, original OFF bodies, repair bounds, timing arithmetic, rejection and restoration. Negative and relocated empty-HOME/PATH controls check evidence integrity only, not additional native correctness or performance.

```sh
python3 -B bench/results/resident-listing-scheduler-20261031/validate.py --manifest
python3 -B bench/results/resident-listing-scheduler-20261031/test_offline.py
```
