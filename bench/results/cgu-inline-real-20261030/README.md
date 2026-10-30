# dp47 real shared-inline traversal evidence

This is a lossless, offline replay bundle of six already-completed coordinator
runs, not a workload runner. See `docs/cgu-inline-real-20261030.md` for the full
interpretation, exact run IDs, source/driver scope, and timing limitations.

* Enabled prepare/edit/restore: traversal counts 14040/14041/14040, with one
  bounded cross-root revisit event each. Disabled markers: zero.
* Four exact tests pass in every run. Edit journals contain precisely one
  `JR_RUNTIME_EXECUTED_47` each. Original 63-file snapshot restored.
* Off edit place-items 34.061566 ms, on 51.518598 ms, both nested elapsed
  intervals. This is **not a speedup**, CPU time, or time-saved claim.
* Six gzip profiles are lossless. `profiles.json` pins compressed and raw
  sizes/hashes. All selected per-thread intervals are in `observations.json`.
* Source bytes, full run journals, logs, original runner/state/exit receipts,
  six dedicated redacted metadata exports, and a pinned offline decoder remain
  bundle-local. Absolute historical paths are intentionally retained in raw data.

From the repository root:

```sh
python3 -B bench/results/cgu-inline-real-20261030/validate.py
python3 -B bench/results/cgu-inline-real-20261030/test_validate.py
```

The first command verifies the inventory and hashes, decompresses/reparses all
six profiles strictly, and checks exact intervals, runtime, source, identity,
flags, and exports. The second mutates the retained in-memory derivation and
small receipts to exercise rejection checks without six full decodes per test.
Neither command launches any compiler, native workload, or historical runner.
Do not use Python `-O`, which disables the assertion-based acceptance checks.

`capture.py` is the one-time archival copier. It does not execute workloads and
refuses existing destination copies. The retained `runner/` scripts are historical
inputs only and must not be executed as part of offline replay.

`sha256.json` inventories every bundle file except itself and Python bytecode.
`validation.json` records the successful six-profile replay, and
`mutation-tests.log` records mutation-suite execution. Both generated receipts
are included in the final inventory.
