# Failed resident state matrix, 2026-10-30

This is an offline, portable evidence bundle, not a passing correctness gate or a
claim that justrust is generally faster than Blaze. Root executed the native jobs.
The evidence worker ran only source inspection, read-only export and offline tests.

## Replay

From any checkout, or after copying this entire directory elsewhere:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-state-matrix-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-state-matrix-20261030/test_validate.py
```

The validator never invokes Git, engines or builds and never opens the historical
absolute paths. Those paths are provenance, not dependencies. It imports the
retained helpers only after checking their independently pinned hashes. Its `PASS`
means faithful replay of the failed experiment, not native matrix acceptance.
`--seal` regenerates the local checksum inventory for maintainers. It is not a
signature or trusted attestation. Re-signed mutation tests prove semantic checks
still reject altered acceptance, source, nonce, timings, restoration, environment,
controls, full-lib counts and cleanup scope even with coherent outer checksums.

## What happened

* `rsm01`: five samples per stack, 14 ordered transitions, 140 calls. Both tools
  refused all ten type-error calls with status101 but only generic `HasErrors`,
  without current-source `E0308` and nonce. All ten stay **REJECTED**, their medians
  excluded. Other130 observations passed their exact runtime/name/count oracles.
* `rstp01`: eight diagnostic calls on the existing s0 stores. Removing the nested
  signature helper did not restore detailed diagnostics. Four error calls remain
  rejected, four recoveries passed. This is not evidence of a harness fixture fix.
* `rstp02`: eight calls comparing exact original uninstrumented fixed-literal
  type-error against PROBE-only unique-literal instrumentation. Both still produced
  only generic errors. Four errors rejected, four recoveries passed. The two fixed
  canonical controls are observational only, never freshness-admitted.
* Original full-lib probe failed because its own guard demanded zero ignored
  tests. That original failed state/log/exit is preserved. A separate **offline**
  correction admits the actual identical148 named results,147passed/0failed/
  1ignored/0filtered, only `split_index::tests::attach_cost`. The pinned source
  independently declares this microbenchmark ignored. No native rerun or new timing
  is implied by the correction. New ignored tests or count/status/name drift fail.
* The root's full728-test log and eight scoped stop receipts are retained. Stop
  operations target only completed matrix sessions1-4, preserve session0 and all
  stores, and are not deletion receipts.

The v2 worker-log deltas show initial `cold: compiled 3988 of 3989` followed by
refusal without the detailed check. Frozen licensed imported Blaze `ra-jit`
source in `proof/` shows `cold.refused_reached` bails before the existing
`nothing_of_rustc && written.is_empty()` whole-file diagnostic block. This is a
source-order observation supporting a diagnostic-ordering hypothesis, not proof
of a repaired compiler, not a clean-room implementation, and not a reason to
weaken the matrix oracle. Any later native repair is a separate experiment.

## Per-state descriptive medians

Milliseconds, reference native ra-jit versus v28 jr-engine, five accepted samples
per admitted cell. There is intentionally no weighted aggregate or significance
claim. Expected/observed/accepted counts and individual failures are replayed from
raw receipts. An unsupported state is not silently removed.

| State | Reference ms | Engine ms |
|---|---:|---:|
| cold-prepare | 23679.448 | 27287.192 |
| repeated-noop-1 | 48.523 | 36.908 |
| repeated-noop-2 | 34.064 | 34.038 |
| summary-testbody-edit | 247.754 | 218.346 |
| summary-edit-recovery | 145.662 | 131.803 |
| ordinary-shim-edit | 134.767 | 121.236 |
| signature-base | 122.523 | 113.624 |
| signature-invalidation | 235.864 | 238.091 |
| manifest-feature-add | 5984.348 | 5970.560 |
| type-error | EXCLUDED (0/5) | EXCLUDED (0/5) |
| type-recovery | 5657.846 | 5641.854 |
| assert-failure | 1618.623 | 1665.963 |
| assert-recovery | 373.032 | 378.379 |
| all-lib | 868.916 | 884.599 |

Cold includes actual elapsed preparation, with fresh private tool/dependency state
per session, **not** flushed OS page cache, toolchain, HOME or global disk caches.
Ordered phases reuse stores. Stack order rotates across samples. Markers,
signature probes, manifest mutations and test instrumentation change the workload.
Removing the helper or instrumentation in followup probes does not make their
results exchangeable with matrix timings. Noise receipts explicitly say daemon
CPU is not waited-child CPU and quietness is unknown. No arbitrary aggregate,
comprehensive session-weighted synthetic gate, default promotion or general
faster-than-Blaze claim follows from these observations.

## Retained inputs and privacy

`input/source63` is the complete63-file `f97fcc2` snapshot, with Git blob IDs and
SHA256 inventory. `provenance/` contains exact frozen5ec494e60 harness/read helpers
and both root-only differential runners. `runs/` retains all explicit console raw
streams, source/manifest backups, initial worker logs and before/after/deltas.
No binary, dependency cache, private cache tree or native build output is copied.
`root/` retains the auxiliary raw streams, original failure and separate admission,
logs/exits and scoped-stop receipts. Imported Rust source licenses are preserved.

JSON is an explicitly allowlisted environment derivative, including nested version
receipts. Credentials and arbitrary ambient values are not retained. Each run's
`transformation.json` records omission counts, original byte commitments and
retained hashes; `evidence.json` records original private locations and hashes.
Full raw diagnostics and historical local paths remain visible, so this is not a
path-anonymized bundle. Export checks private original bytes unchanged and refuses
known omitted sensitive values appearing outside filtered maps. SHA256 commitments
provide reproducibility, not independent authenticity or a guarantee against
secrets absent from the inspected maps.

The portable replay checks complete source/manifest restoration, immutable tool and
pinned toolchain identities, before-phase inputs and recipe hashes, explicit cache
scope, bounded outputs, current nonce execution counts, exact parsed tests, raw
stream checksums, all phase/reference admissions, and per-state summaries. No
private originals are rewritten or relabeled.
