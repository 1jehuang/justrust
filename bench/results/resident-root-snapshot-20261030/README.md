# Rejected native resident root snapshot experiment, 2026-10-30

**REJECTED_NOT_REPLICATED. No compiler change ships from this bundle.**
This is licensed Blaze-derived experimental resident work, not a clean-room implementation.
The first fresh v4-to-v19 observation was -18.51%, but candidate-first replication was only -1.16% raw and +1.67% reference-normalized. A separate 42-triple warm diagnostic found v19 +0.2335% slower than v4. Noise remains unknown.

## Offline replay

```sh
python3 -B bench/results/resident-root-snapshot-20261030/validate.py
python3 -B bench/results/resident-root-snapshot-20261030/test_validate.py
```

These are Python standard-library checks. They never execute archived binaries, compiler commands, network access, or workloads. Copy this directory elsewhere and run its `validate.py` there to verify portability. `audit.json` and `offline-tests.log` record the final replay. The manifest inventories original artifacts, not a cryptographic trust signature. Verifiers and generated reports are excluded from self-hashing.

## Retained evidence

- `source-ready/v16..v19`: complete main/codegen/signature bytes, original readiness receipts, original hashes, scoped patches, rejected v18, and v17-to-v19 cleanup lineage. Native frozen-source trees independently bind these to the build receipts. Never reconstruct candidates from current live source.
- `native/`: build/check receipts, all five scoped test filters, full351 positive names and raw journals for each v16..v19. Baseline v4 source bytes are explicitly hash-matched historical archives, not falsely described as an original full-tree freeze.
- `sessions/`: all twelve original top-level fixed-five-edit suites, including deliberate compile failures, assertion failures, recovery, raw stdout/stderr, metadata, identities, summaries, and per-sample timings.
- `traces/`: all three original instrumented 12-row sets and independently replayed nested phase summaries. Nested durations are not critical-path savings or CPU work.
- `provenance/`: exact orchestration/comparison scripts, failures, exits, source identity receipts, and the 34-private-namespace stop receipts before the final pairs. No stores were deleted. This cleanup changed historical memory pressure and forbids attributing historical-to-final differences solely to source.
- `observations/interleaved-v19/`: separately appended original runner, state, original workload source and all 282 raw streams for 141 invocations. Existing RSB09/RSA15 stores, 15 control/failure/recovery invocations and 126 timed invocations, all six stack permutations seven times. This warm diagnostic does not replace or pool with any fixed-five suite, cold support gate, or comprehensive benchmark.

`collect.py` captured the original historical evidence once. `append_interleaved.py` first verified every original artifact hash, then appended only the separate terminal observation. Neither script is needed for portable replay. `validate.py`, `raw_checks.py`, and `warm_checks.py` independently recompute identities, commands, source edits, all runtime markers and gates, raw test counts, sample medians, decimal ratios, trace summaries, and the rejection decision. Mutation tests exercise semantic checks beyond simply failing a checksum.

See `docs/resident-root-snapshot.md` for architecture, dependency constraints, cold/parallel regression risk and the six-pair result table. No broad/default-promotion or shipping speed claim is supported.
