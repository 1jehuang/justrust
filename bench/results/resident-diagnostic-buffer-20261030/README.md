# Diagnostic text buffer v20: REJECT

Licensed Blaze-derived experimental engine evidence, not clean-room code and not a default promotion. Production main.rs was restored to HEAD by root and independently checked before this evidence-only followthrough. No native workload was run by the evidence collector.

## Result

Correctness passed: focused helper 2, graph reuse 1, listed roots 1, telemetry 3, and **full native 348** (baseline 346 plus two helpers), run `20261030-113913476-2300871`. Do not describe that run as 349. Exact names and counts are checked from raw journals, not just summary counters.

Fresh fixed-five RSB11 v4 versus RSA17 v20: engine median 0.11409415000525769 to 0.1133263889933005 seconds, -0.673%, reference-normalized -1.107%. Separate extended 42-edit observation: old 0.10399137149943272, new 0.10771834100160049, Blaze 0.10450627249520039 seconds, candidate **3.584% slower**. All six execution orders occur seven times, without exclusions. All failure/recovery and exact execution marker gates pass.

The small fixed-five movement did not establish a repeatable wait improvement. Noise is unknown. No allocation counts were measured. Neither result establishes general performance, significance, comprehensive session-weighted coverage, or a default-promotion gate. Candidate source and helper tests remain archived only.

## Portable offline audit

```sh
python3 -B validate.py
python3 -B test_validate.py
```

Both work from this directory or by explicit path. `validate.py /moved/bundle` also works. There are no third-party Python dependencies, native tool executions, network reads, absolute-path dereferences, or sibling-bundle dependencies during replay. Historical absolute paths in receipts are values to validate, not required locations. The mutation suite includes a copied-directory subprocess replay. Nineteen tests cover positive replay and fail-closed marker, status, testcount, raw name, hash, medians, source, tool, profile, toolchain, edit, order, failure/recovery and restoration mutations. Marker corruption is rejected even when the raw-stream checksum is updated to match.

## Inventory and binding

- `source-ready/`: exact full candidate and HEAD source, HEAD identity, exact two-file HEAD-to-candidate patch, original source-ready document, original SHA receipt and source-ready handoff. Candidate reconstruction runs entirely in memory.
- `native/candidate-build-v20/`: source manifest and 11 frozen files, check/build logs/exits, source verification, version and binary SHA receipt, two allowlisted native runs.
- `native/candidate-unit-v20/`: focused and full logs/exits, positive-admission receipt, source verification, five allowlisted native runs. Each v20 run contains only `output.jsonl`, `meta.json`, and `summary.json`. `native-runs.json` lists the exact seven identifiers.
- `native/candidate-build-v4/`: retained historical baseline provenance from the existing root-snapshot evidence bundle, now local so replay has no sibling dependency. These are not additional v20 runs.
- `exports/`: metadata-only `justrust export --out` results from the two exact private homes. Nothing was written to global `data/runs/`.
- `sessions/rsb11/`, `sessions/rsa17/`: top-level original suite, metadata, raw records, summary, original source and stdout/stderr. No build/cache/store directories or executables are collected.
- `observations/interleaved-v20/`: original source, final state and all 282 streams for the 141 rows. Identity, process status, four test names, markers, edit hashes, exact paired ordering and medians are replayed.
- `provenance/`: root scripts and raw suite receipts, interleaved runner/log, independent root-restoration receipt. Archived scripts are never executed by the audit.
- `harness-captured/`: original harness source for historical inspection, not imported/executed by replay.
- `raw_checks.py`, `warm_checks.py`: self-contained offline parsers adapted from the prior evidence validator, with v20 identities and warm rejection bounds updated. The audit binds exact tools/profiles/source63/toolchain identities and source/test/stream hashes.
- `manifest.json`: artifact inventory and SHA-256 checks, generated only by explicit `seal.py`. `audit.json` and `offline-tests.log` are recorded results, not trusted inputs to admission. Validator/test/collection/readme files are excluded from artifact inventory to avoid circular bookkeeping.

`collect.py` is the allowlisted historical collector, requiring the original scratch inputs if rerun. `seal.py` only seals evidence files. Neither is part of offline validation and neither runs native workloads. See `docs/resident-diagnostic-buffer.md` at repository root for the bounded hypothesis and decision.
