# Rejected native resident root-classification experiment, 2026-10-30

**REJECTED. No production promotion.** Licensed Blaze-derived engine, not clean-room. Positive native classification reuse and all 347 native tests passed, but the final adjacent timing replication was worse: RSB03 → RSA09 raw **+11.53%**, reference-normalized **+12.63%**. These are descriptive scoped measurements with unknown noise, not a significance or causal-speed verdict. See `docs/resident-root-classification.md` at repository root for the full decision.

## Offline replay

From any working directory, with Python 3 and only the standard library:

```sh
python3 /path/to/resident-root-classification-20261030/validate.py
python3 /path/to/resident-root-classification-20261030/test_validate.py
```

Neither command builds, executes an engine, accesses the network, reads the original machine paths, or requires git. The mutation suite includes a relocated-copy replay. Synthetic mutations exist only in memory, rehashing the original artifact/manifest hierarchy so semantic checks are actually exercised. Original raw artifacts are never mutated by tests.

## Contents and provenance

- `sessions/{rsb02,rsa07,rsa08,rsb03,rsa09}/`: **every original top-level file**, byte-for-byte, including `suite.json`, `metadata.json`, `records.json`, `summary.json`, `source.original`, all stdout/stderr, versions, and every runtime failure/recovery/edit. Private daemon/cache/build directories are intentionally excluded. All five samples per stack remain present with no exclusions.
- `native/candidate-build-v{4,9,10,11,12}/`: original build/check exits, logs, source hashes, version and binary identities where the build reached that stage. V9 has no successful engine build because its actual native check failed.
- `native/candidate-unit-v10/`: original failed complete-reuse test. `frozen-oracle/` retains the already-compiled test executable's identity and independent oracle/listed/telemetry pass receipts. This does not turn the mechanism failure into a pass.
- `native/candidate-unit-v{11,12}/`: exact per-provider counts, full-classifier/listed/telemetry gates, source-after checks, and raw justrust run output/meta/summary. V12 also retains the complete **347 individual native test results** for run `20261030-091611542-269328` and full-native before/after provenance. Install IDs, telemetry delivery state and private build caches are excluded.
- `provenance/`: original build/test/timing scripts, the original ready receipts v9/v10/v11/v12, failed-gate blocking receipt for v10, and unchanged original `bench/suite.py compare --floor 10` outputs for all three comparisons. Scripts retain historical absolute paths and are **not replay instructions**. Executed command/environment provenance is also retained in each session's original records.
- `candidate-v12/`: exact final main/codegen/document snapshots, original license texts, and exact rejected two-file patch against capture HEAD. The archived document is the historical frozen pre-decision text, not the current rejection report.
- `baseline-head/`: the original two source files from capture HEAD `5e57bc13e4f562513a9f1c27760660e948dacded`. The validator replays the archived patch in memory and requires byte equality with the final candidate snapshots.
- `restoration.json`: observed exact-HEAD byte comparisons and successful scoped git diff at the source-restoration event, before production ownership was released. Future experiments may edit live source independently.
- `harness-captured/`: harness source captured when archiving. It is not asserted to be a historical source snapshot at every earlier run. Original raw comparison outputs are authoritative evidence and independently checked against all-sample medians.
- `provenance.json`: explicit REJECTED/no-promotion decision and limits of historical source availability.
- `.gitattributes`: disable whitespace normalization warnings only for original raw stdout/logs and archived unified patch context. Those original bytes contain trailing blank lines and context spaces and must not be rewritten. Authored Python/source/Markdown remain whitespace-checked.
- `manifest.json`: SHA256 inventory of raw/captured evidence and generated provenance. Validator/test code, this README and derived audit outputs are versioned by the enclosing git commit instead of recursively checksumming themselves.
- `collect.py`: original one-shot capture utility, machine-specific and guarded against rerun once a manifest exists. **Do not recapture after production restoration.** It is not needed for offline validation.

Older v9/v10/v11 full source bytes were **unknown/not captured** by this worker. Their historical hashes/logs/ready receipts remain untouched. No source reconstruction is claimed. V12 full source is actually retained, hash-verified against its original ready receipt:

```text
ee2ef625d670c5a646acf2f180495a4294123cfb92f0fb83b967418167e2e03c  candidate-v12/main.rs
304e148fdf9ef75d77a11f5f4802d81698c1104b88a1caeb0cb5fe2b2fbeb45e  candidate-v12/hir-ty-codegen-mod.rs
501047a0a6ee1a560740777fded8485c37eb8d63195f482ed4293cee5304b19d  candidate-v12/resident-root-classification.md
b79b5b51bda859f2442aa24df05ca2922fc0883b7cc0a081339d6e3283ae8124  candidate-v12/candidate.patch
```

## Native mechanism versus total-time decision

Counters are `[export, root, main]`:

| Version | Cold | Unchanged | Body-only edit | Result |
|---|---|---|---|---|
| v9 | not run | not run | not run | native reference-return compile failure |
| v10 | total 5 | reused | total 7 | original all-provider-zero assertion failed |
| v11 | `[2,2,1]` | `[2,2,1]` | `[2,4,1]` | selective reuse, native oracle passed |
| v12 | `[2,2,1]` | `[2,2,1]` | `[2,2,1]` | downstream reuse and all 347 native tests passed |

V12 tracked the actual compile-time-only primitive, retaining its exact raw attribute predicate. The full original classifier oracle uses **direct untracked** `AttrFlags::has_word` for compile-time-only decisions, so it is independent of that candidate query. Attribute toggles, generic/container transitions, provided nested bodies, main/no-main, kept statics and core-origin allocator plus external trait edits remain exercised.

| Comparison | Raw engine median delta | Reference-normalized delta |
|---|---:|---:|
| RSB02 → RSA07 (v11) | -1.07% | +0.37% |
| RSB02 → RSA08 (v12 first run) | -5.03% | -0.78% |
| RSB03 → RSA09 (v12 adjacent replication) | +11.53% | +12.63% |

The first v12 result was below the 10% floor. The final adjacent result failed the prospective optimization gate. Correctness and a positive cache mechanism do not establish a useful total optimization. All outcomes are retained and the live production changes were reversed before this evidence-only publication.
