# Native resident suite diagnostic

`bench/suite.py resident` adapts the existing `resident_engine_compare.py` runtime
harness. It is a **scoped, descriptive diagnostic**, not the synthetic session-weighted
workload, a comprehensive performance gate, or evidence for default promotion.
Only the coordinator should execute prospective native measurements. Unit tests
use mock processes and do not invoke a compiler or resident engine.

## Before, after, compare

Use the exact pristine 63-file source tree at `f97fcc2`, the frozen source-built
reference executable, and the complete already-installed pinned toolchain. Paths
must be absolute and without symlinks. Output roots must be fresh, disjoint from
the source, and at most 70 bytes. Keep the reference executable and toolchain
unchanged. Preserve both raw output directories for later comparison.

```sh
python3 bench/suite.py resident \
  --reference-engine /absolute/source-built-reference \
  --engine /absolute/before-jr-engine \
  --toolchain /absolute/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --source /absolute/source63 --output /short/resident-before

# Build the optimized candidate separately. Do not change the source63 workload.
python3 bench/suite.py resident \
  --reference-engine /absolute/source-built-reference \
  --engine /absolute/after-jr-engine \
  --toolchain /absolute/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --source /absolute/source63 --output /short/resident-after

python3 bench/suite.py compare \
  /short/resident-before/suite.json /short/resident-after/suite.json --floor 10
```

If needed, supply the **same** `--proc-macro-server /absolute/native-server` to
both invocations. Without it, the harness explicitly disables both resident
proc-macro-server variables and uses its rustc-built compatibility control.
The required sample count is exactly five, `--samples 5` is optional, and
`--timeout 360` sets the positive per-process timeout in seconds. There is no
seed option and no cache seeding. Cold failures remain failures, not a reason
to import warm state. Failed runs retain raw evidence but do not emit a valid
suite result.

## Evidence and fail-closed comparison

The `native_resident/v1` result stores both stacks' five samples and medians,
full source/toolchain/tool identities, host/CPU/execution identity, and SHA256
receipts for metadata, records, original source backup, summary, and every
stdout/stderr file. These private artifacts contain paths and should not be
published without reviewing them. Receipts detect changes, not malicious
fabrication or authenticity. Keep original artifacts at their recorded paths.

Comparison verifies the receipts and **replays raw logs**. Cached `accepted`,
`parsed`, `gate_passed`, and summary medians are not acceptance authorities.
Each stack must have its version query, preparation, real E0308 type failure,
successful type recovery, actual assertion failure, successful assertion
recovery, and exactly five real edits. Successful tests must report the exact
four names and counts, matching filtered counts. Each edit must print its
unique `JR_RUNTIME_EXECUTED_N` marker exactly once. Command and edited-source
hashes must match the recorded phase. Missing logs, process errors, timeouts,
nonpositive/nonfinite timings, stale commands/edits, and incomplete rows fail.

All 63 source hashes are recomputed from the repository's pinned Git blobs.
Source restoration, unchanged engine/reference/server identities within each
run, and unchanged complete toolchain file manifests are required. Before and
after must use the same full toolchain, source, profile, Cranelift backend,
proc-macro policy/server, source-built reference executable, and host/CPU/local
execution mode. Candidate executable hashes and candidate source identities
may differ, since those are the optimization under test. This harness records
executable hashes, not a cryptographic executable-to-source build attestation.
Mixed native and synthetic/named results are rejected before the legacy
runtime-context path. Existing named/synthetic behavior is unchanged.

## Interpretation

Output reports the engine median delta in seconds and percent, plus the
engine/reference median ratio for each run and its relative delta. The
`--floor` threshold is descriptive only. Noise remains **unknown** because
resident daemon CPU is not waited child CPU. Even a delta above the floor is
uncontrolled, with no significance, faster verdict, or speedup claim. Use the
comprehensive synthetic gate and other relevant correctness checks separately
before considering a default promotion.

## Mock regression tests

```sh
PYTHONPATH=bench python3 -m unittest \
  bench/test_resident_suite.py bench/test_resident_engine_compare.py \
  bench/test_suite.py bench/test_suite_failures.py
```
