# Rejected resident demand-lowering experiment, 2026-10-30

**REJECTED.** Native mechanism and semantic gates passed, but the fresh scoped five-edit suite showed no useful latency benefit. Engine median `0.109617051 -> 0.110432806` seconds, descriptively **0.74% slower**. The same source-built reference changed `0.110923157 -> 0.110535542` seconds. Noise is unknown and these observations are not a significance claim, a general speed claim, or a default-promotion gate.

The production `main.rs` was restored exactly to `c4a20a5f6` using a scoped reverse edit, not reset/checkout. The rejected candidate remains only as evidence. Shipping defaults are unchanged. No native workload was run by the evidence-preservation lane.

## Portable audit

Run from any working directory with Python 3 and Git available:

```sh
python3 /path/to/this/bundle/audit.py --self-test
```

Do not pass `--seal` when checking an existing receipt. Sealing is an explicit collector operation, not validation. The manifest is a content-integrity receipt, not a cryptographic authenticity guarantee.

The audit checks the complete bundle inventory, exact accepted/rejected source hashes, reconstruction of the candidate by applying `source/candidate.patch` to `source/baseline-main.rs` in a temporary directory, failed v5 E0425 receipt, successful v6 build/unit statuses, eight actual native query execution deltas, suite artifact hashes, expected type-error/assertion-failure and recovery exit statuses, five successful edits per stack, medians recomputed from raw records, and unchanged reference binary identity. No original host paths or native engine binaries are required. Tamper self-checks cover an altered candidate hash and removal of the rejected-build receipt.

## Preserved evidence

- `source/candidate-main.rs`: exact tested v6 source, SHA-256 `7792413378f3508b3d3692ec4ea9e2046ebb8adfd6464756be5c40131eda1855`.
- `source/baseline-main.rs`: accepted `c4a20a5f6` source, SHA-256 `471f0b5c10187ecbea879acfe3e5eae52925161e32bce6feeebe49b67af0368f`.
- `source/candidate.patch`: complete candidate delta, including test-only counters and assertions. The original eight revision sources and full-walk comparisons were not weakened.
- `provenance/candidate-build-v5`: rejected compile receipt. The initial implementation mistakenly removed a helper used by `outline.rs`.
- `provenance/candidate-build-v6` and `candidate-unit-v6`: repaired actual build/unit receipts with original source and binary identity hashes, logs, and exit markers.
- `provenance/build-test-candidate-v6.sh`, `run-suite-after-v6.sh`, and suite log/exit/compare: original coordinator execution scripts and receipts. Do not run these host-specific historical scripts as part of an audit.
- `rsa01`: actual before suite. `rsa02`: actual after suite. Both contain raw top-level stdout/stderr, records, metadata, summaries, source original, suite manifests, and retained engine/reference worker logs. The nested `home/jeremy/...` directory names in copied worker logs are historical provenance only.

Native query executions by revision were `7, 1, 3, 2, 3, 3, 2, 1`. Cold execution and zero additional executions on a second same-revision listing were asserted in the passing test. These are declaration-query execution counts, not direct `Body::of` counts, CPU savings, or latency evidence. Mechanism effectiveness did not translate into useful measured latency improvement.

This evidence contains snapshots of the explicitly licensed Blaze-derived resident frontend, not the separate clean-room compiler. The applicable source licenses remain `toolchain/resident/LICENSE-MIT` and `toolchain/resident/LICENSE-APACHE` in the repository. Original raw logs retain local paths for provenance. Caches, build outputs, binaries, and account state are not included.
