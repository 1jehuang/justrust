# Frozen real associated-item retention diagnostic

See `docs/real-associated-retention-20261030.md` for the experiment, selected
count tables, query-cache storage and limitations. This directory contains
only new evidence, not compiler or client changes.

- `native-acceptance.json`: full 34-case native correctness observations.
- `<step>-receipt.json`: compiler, source, flags, profile hash, analyzer exits,
  per-step query-cache logical/allocated bytes and isolated export status.
- `baseline-<step>-receipt.json`: unchanged copies of retained off receipts.
- `*-query_profile-unknown.json`: explicit-unknown full-profile metadata/counts.
- `*-reuse_ceiling-unknown.stdout`: supported explicit-unknown reuse inventory.
- `*-provider_cost-unknown.stdout`: supported elapsed-interval inventory retained
  as raw evidence, not used to make speed or time-saved claims.
- `*-strict.stderr` and `*-strict-full-traceback.txt`: strict failure evidence,
  never replaced with successful explicit-unknown outputs.
- `count-comparison.json`: independently decoded named observations per capture,
  including unknown labels by event kind. No cross-invocation numeric ID joins.
- `baseline-post-body-edit-storage.json`: read-only retained baseline snapshot,
  not a missing baseline per-step storage series.
- `identity.json`, `source-manifest.json`, `private-runner-sha256.txt`: exact
  compiler/std/analyzer/source and private runner provenance.
- `validation.json`, `*-tests.log`: assertions and focused test results.

Private scripts and raw profiles remain under
`/home/jeremy/.jcode/scratch/real-associated-retention-20261030/`, except the
retained baseline raw profiles at the exact paths in their receipts.
`run.py` accepts `native`, `prepare`, `unchanged`, `body-edit`, `restored`.
The body edit and restoration were done separately using the edit tool.
`compare.py` replays baseline analyzers and decodes named counts per invocation.
`validate.py` checks exact observations, identities, strict failure cause,
compiler/unknown-mode/export exits and the native acceptance matrix.

The isolated four-run export is in
`data/runs/xps14-real-associated-20261030/2026-10.jsonl`.
There is no wall-performance comparison, saved-time estimate, global maximum
or default promotion. Concurrent scalar work makes wall claims inappropriate.
