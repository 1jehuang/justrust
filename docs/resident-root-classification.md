# Resident root classification: REJECTED

**No promotion. No production change retained.** This experiment was in the licensed Blaze-derived resident engine, not the clean-room compiler rewrite. The cache mechanism worked, but prospective total-time replication did not support keeping the optimization.

## Decision and measurements

All five original session artifacts are retained under `bench/results/resident-root-classification-20261030/sessions/`. Each session has five edit samples per engine/reference, actual filtered libtest execution, real type-error/assertion-failure gates and recovery, source/toolchain/binary identity checks, and no sample exclusions.

| Session | Role | Engine median seconds | Reference median seconds |
|---|---|---:|---:|
| RSB02 | v4 baseline | 0.110655683005 | 0.112896352002 |
| RSA07 | v11, selective classification reuse | 0.109474718993 | 0.111278088007 |
| RSA08 | v12, full downstream classification reuse | 0.105086910989 | 0.108055189994 |
| RSB03 | fresh adjacent v4 baseline | 0.113249293994 | 0.110273671991 |
| RSA09 | adjacent v12 replication | 0.126309995991 | 0.109196114005 |

The original `bench/suite.py compare --floor 10` outputs are archived unchanged:

- RSB02 → RSA07: raw **-1.07%**, reference-normalized **+0.37%**. No reliable total win.
- RSB02 → RSA08: raw **-5.03%**, reference-normalized **-0.78%**. Below the requested 10% floor and not a robust win.
- RSB03 → RSA09: raw **+11.53%**, reference-normalized **+12.63%**, worse in the final adjacent replication. Reject.

Normalization is `(candidate_engine / candidate_reference) / (baseline_engine / baseline_reference) - 1`. These are scoped descriptive measurements, not significance estimates, general workload speed claims, or proof that this cache alone caused the measured regression. Noise remains recorded as unknown. The earlier favorable raw result is retained rather than selected as the conclusion.

## What was tested

The candidate introduced native Salsa per-function queries for export/link/allocator classification, compile-time-only/generic root classification, and main recognition. Original ordering, short circuits, provided-method distinctions, module/body declaration queries, static retention, and parallel lower warmup were preserved. There was no manual revision or file allowlist.

Native feedback was retained at every stage:

1. **V9 failed native checking** with E0308/E0277 because the local Salsa default returns references. Original logs, hashes and ready receipt remain intact.
2. **V10 passed check/build**, then failed its complete-reuse mechanism assertion with cumulative provider counts 7 versus 5. The root separately ran the already-built frozen graph/full-classifier oracle, listed roots, and telemetry tests successfully. Original failure is not rewritten as success.
3. **V11 passed native gates** and established exact `[export, root, main]` counts `[2, 2, 1]`, `[2, 2, 1]`, `[2, 4, 1]` for cold, unchanged, and body-only revisions. Export/main reused, but the raw attribute dependency forced both root providers to reexecute.
4. **V12 tracked the existing `compile_time_only` boolean primitive**, preserving its exact `AttrFlags::has_word` body. Backdating its unchanged result established exact native downstream counts `[2, 2, 1]` for all three revisions. The primitive itself can reexecute its raw attribute read. This proves downstream classification reuse, not zero work throughout the dependency graph.
5. **V12 passed all 347 native engine tests**, including the independent full-classifier oracle, real attribute/generic/main/provided/static/allocator transitions, cross-file allocator trait edits, and telemetry. Raw output with all 347 individual passing tests is preserved for run `20261030-091611542-269328`.
6. All runtime/error/recovery gates and all five edits passed in the retained timing sessions. Correctness and a working mechanism did not override the negative total-time decision.

The v12 full-classifier oracle independently calls the original direct `AttrFlags::has_word(db, it.into(), "rustc_comptime")` in both rooted/provided filters, rather than the tracked candidate primitive.

## Preservation and restoration

The final v12 complete `main.rs`, `hir-ty-codegen/mod.rs`, frozen experiment document, and exact two-file patch against capture HEAD `5e57bc13e4f562513a9f1c27760660e948dacded` are archived in `candidate-v12/`. Archived `baseline-head/` source permits in-memory patch replay. The final snapshot hashes match the original SOURCE READY v12 receipt.

After preserving those bytes and the patch, this worker reversed **only** its two owned production files. Both matched archived HEAD byte-for-byte and `git diff --exit-code HEAD` for those two paths passed before ownership was released. No live production source file is included in the evidence-only publication. Full source appears only inside the evidence archive. Subsequent experiments own their source changes independently.

Older v9/v10/v11 full source bytes were not captured by this worker and remain **unknown/not captured**. Their actual historical hashes, ready receipts, scripts, build/native logs and identities are preserved. There is no reconstruction claim and no retrospective relabeling of versions. Original machine paths remain in raw provenance but are not dereferenced by the portable audit.

## Offline replay

```sh
python3 bench/results/resident-root-classification-20261030/validate.py
python3 bench/results/resident-root-classification-20261030/test_validate.py
```

The audit checks immutable checksums, original failed controls, exact native counts, independent oracle source, archived patch replay, all 347 individual raw tests, every session's positive/negative/recovery gates and runtime edit markers, all-sample medians, matching source/toolchain/binary identities, original compare outputs, and the explicit rejection decision. Mutation tests rehash modified fixtures in memory so semantic checks are exercised beyond checksum mismatch. They never execute engines, compile, or rerun timing workloads. The archive is relocatable and does not require the original scratch directories.
