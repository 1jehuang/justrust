# Parallel native declaration query experiment (v72/v73)

**Not promoted. Performance acceptance rejected, with mixed scoped results and unknown noise.** V73 passed the actual native and strict-runtime correctness gates, but no broad or useful-roots improvement was established. Production `main.rs` was restored exactly to accepted SHA256 `0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176`. The experiment is retained as evidence, not enabled shipping behavior.

## Hypothesis and actual implementation

V69's native final tracked declaration query reused green results but lowered cold bodies sequentially. V72/V73 investigated retaining native green reuse while preserving the existing parallel cold-lowering scheduler. This was not the v70 flattened listing scheduler.

The historical exact first-use `RA_JIT_PARALLEL_DECLARED_QUERY=1` switch was default OFF. A local `#[salsa::db] trait DeclarationDatabase: HirDatabase` exposed `fn declaration_snapshot(&self) -> RootDatabase`, implemented safely by `RootDatabase::clone`. The native `#[salsa::tracked(returns(ref))]` query accepted `&dyn DeclarationDatabase` and actual `base_db::Crate`, returning the full existing `Declared` graph.

Actual Salsa 0.28.5 macro source requires `&dyn Db`, not concrete `&RootDatabase`, for a tracked provider. The vendored `RootDatabase::clone` shares storage/files/crate-map handles and nonce. Salsa `Storage::clone` shares the `StorageHandle`/`Arc<Zalsa>` but creates a fresh `ZalsaLocal`. Child query reads do not transfer to the parent's active query stack. Therefore each wave still reads every tracked `body_declarations(db, body, under_provided)` on the parent in original order after warmup. Name resolution and `module_declarations` are also read on the parent. Existing `Declared::extend` preserves all fields, ordinary/provided distinctions, nested items, consts, statics, macros, cfg and graph ordering.

There are no handwritten unsafe casts, global database pointers, fake snapshot suppression, private dependency keys or new dependencies. Both original OFF traversal and original `lower` remain bytewise unchanged in the candidate snapshots. Restoration independently checks the full accepted original file rather than retaining experimental additions.

## V72 actually failed, then V73 repaired the production route

The coordinator's v72 check and build exited 0 and codegen passed 53 tests. The resident suite passed **365 and failed 4 of 369**, all four database fixtures panicking at `hir-ty/src/next_solver/interner.rs:2477` with `Cannot change attached database`. The complete actual failure transcript and full failed source are retained. V72 was not timed.

The cause was calling original `lower(snapshot, bodies)` for a small wave. Its single-worker branch calls `hir::attach_db` on the current thread. Shared Salsa storage does not give the clone the same database address, so replacing an already attached original database correctly violates the HIR identity invariant.

V73 introduced the bounded production `prewarm_declaration_bodies` route. Empty, at-most-16-body or single-worker waves call actual `Body::of(db, body)` directly on the parent without cloning or reattachment. Larger multicore waves call the unchanged original `lower` with a safe typed clone, whose parallel workers attach their databases on fresh threads. The guard matches original `PER_THREAD = 16` and available parallelism. Mandatory parent fragment reads remain after both routes. The repair did not use `attach_db_allow_change`, weaken assertions or alter database identity.

All five strict v72 fixtures and full original/independent graph oracle were preserved byte-for-byte. The sixth fixture exercises actual production small/49-body/small warmup routing under an attached parent and then a large-to-one-nested-body graph transition, nested function/static mutation and restoration. Test-only TLS records actual routing decisions and actual provider entries, not simulated events.

## Actual correctness gates

V73 actual runs `20261031-063254128-1558563` and `20261031-063254498-1558730` passed **53 codegen and 370 resident tests**. All six strict native fixtures passed:

- Exact/default-OFF switch parsing.
- Real database cold provider count 1, repeat count 0, complete `Declared` equality against the original parallel walk and an independent direct full scope/body walk. Revisions cover executable-only edits, nested and macro items, provided methods, consts, statics, closures, cfg, signatures and restoration.
- Dependency macro/signature changes and unrelated-crate green reuse.
- Safe local typed snapshot and shared native memo reuse.
- A 96-body two-wave graph followed by nested block function/static mutation and restoration, explicitly checking parent dependency recording after parallel warmup.
- Actual small/multicore/transition production warmup routing and nested graph mutation.

All **28 strict runtime rows** passed in four seven-phase cases, using original raw parsers/contracts/oracles, fresh invocation and failure nonces, exact test names/counts/statuses and unchanged inputs. The 3,757 native-source files, root Cargo.lock, 63 workload-source files and 4,020 pinned-toolchain files were identity-checked. No lock edits were made.

The process-once `parallel-declared-query-provider-entry pid=...` marker appears once in every cumulative ON worker snapshot and zero times in every OFF snapshot. It proves actual provider entry only, not a cache hit, fresh work per phase, body counts, worker counts, avoided computation or latency attribution. Captured PIDs change across manifest/type/recovery worker generations, so the snapshots do not imply one process persisted across all phases. Native TLS fixtures, not the marker, check cold1/repeat0 provider behavior.

## Mixed scoped timing, not broad acceptance

Counterordered OFF/ON and ON-repeat/OFF-repeat cases used the same v73 binary, source snapshots, controls and pinned toolchain. Seconds below are actual captured elapsed values, rounded for display. Percent is `100 * (ON / OFF - 1)`, with negative meaning lower observed ON latency.

| Phase | OFF / ON seconds | ON vs OFF | OFF-repeat / ON-repeat seconds | ON-repeat vs OFF-repeat |
|---|---:|---:|---:|---:|
| Cold prepare | 20.619656 / 23.020290 | +11.64% | 20.864271 / 21.743726 | +4.22% |
| Repeated noop | 0.043301 / 0.035088 | -18.97% | 0.043940 / 0.037085 | -15.60% |
| Manifest feature add | 9.090530 / 9.763832 | +7.41% | 9.811860 / 9.706115 | -1.08% |
| Type error | 9.960306 / 10.152774 | +1.93% | 10.213072 / 9.926092 | -2.81% |
| Type recovery | 8.939242 / 9.203736 | +2.96% | 9.432704 / 9.274028 | -1.68% |
| Assertion failure | 2.147589 / 1.996452 | -7.04% | 2.184097 / 1.946617 | -10.87% |
| Assertion recovery | 0.475900 / 0.484158 | +1.74% | 0.591260 / 0.462504 | -21.78% |

Observed repeated-noop elapsed time was lower by about 8.2/6.9 ms, while assertion-failure elapsed time was lower by about 151/237 ms. Cold prepare was higher in both pairs. Manifest, type-error, recovery and assertion-recovery results were mixed across pairs. It would be incorrect to call every phase slower, claim a causal regression, or claim the marker proves the mechanism responsible for any measured difference. Noise remains unknown. These are focused diagnostics, not the comprehensive session-weighted synthetic gate, and do not establish a broad useful-roots win. Performance acceptance was rejected rather than promoting on selected rows.

## Restoration, cleanup and reproducible evidence

At `2026-10-31T13:40:07Z`, live main and proposal document matched frozen v73 and were copied into `live-before-restoration`. Only `main.rs` was restored via a write from the full frozen accepted original. Bytewise comparison and scoped empty HEAD diff passed, then production source ownership was released. This is a point-in-time receipt, not a claim about later independent changes. Root stopped all four exact completed private namespaces after measurement with exit 0 and without deleting stores.

The portable bundle is `bench/results/resident-parallel-declared-query-20261031/`. It includes full baseline/failed/candidate/live source, all proposal and repair patches, actual v72 failure, complete selected named v73 native results, inventories, original strict parser/oracle sources, original runtime driver/native gate scripts, root reanalysis, scoped arithmetic and restoration/cleanup receipts. Historical source-uncompiled freeze notes are preserved as history, not claims that later actual gates were skipped.

Run only standard-library offline evidence replay:

```sh
python3 -B bench/results/resident-parallel-declared-query-20261031/validate.py --manifest
python3 -B bench/results/resident-parallel-declared-query-20261031/test_offline.py
```

These commands launch no native job. The bundle uses fixed environment/control/output allowlists, exact source and selected-journal commitments, semantic negative mutations and isolated empty-HOME/PATH relocation. Private homes/cache paths, ambient environment, credentials, full private worker/state/daemon logs, telemetry data, executables and targets are excluded. The approved full actual v72 failure transcript contains no private paths. No source change from this experiment is promoted.
