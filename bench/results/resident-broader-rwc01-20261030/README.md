# Resident broader rwc01/rwc02 evidence, October 30, 2026

This is a separately sanitized, portable derivative of two root-owned native
comparisons using the source-built pinned ra-jit reference and the frozen v28
native jr-engine. It is **four scoped workloads, not a comprehensive benchmark,
not a default-promotion gate, and not a statistical-significance claim**.
The evidence worker ran no native builds or resident workloads.

## Observations

Both native runs completed 82 invocations, including 40 accepted executed-marker
edits, across all four workloads. No stack/workload was excluded. rwc02 used new
private stores and reversed workload order. Every timed sample rotates the two
stacks while presenting identical edited bytes to both. Each stack/workload has
its own type-error, assertion-failure, and unmodified recovery gates before the
five timed edits.

| Workload | rwc01 reference / native median ms | rwc02 reference / native median ms |
| --- | --- | --- |
| summary test body | 109.276 / 82.207 | 112.474 / 81.560 |
| shim test body | 84.929 / 80.938 | 82.718 / 76.565 |
| record test body | 85.590 / 81.982 | 90.204 / 81.322 |
| ordinary shim `parse_pass` body | 63.599 / 62.812 | 78.593 / 75.478 |

These are descriptive medians from these runs, not confidence intervals or an
expected-session speedup. Each workload's exact native-reference-discovered
names/counts are retained and independently checked against the raw libtest
output: summary 4 passed / 144 filtered, shim 2 / 146, record 5 / 143. The ordinary
function edit uses the shim module's same two tests. Exactly one exercising test
fails in each deliberate assertion phase. The ordinary function is called by
`shim::tests::parses_time_passes_json`, not a newly added test. Background load
and limited resident-daemon CPU accounting are retained in each noise receipt.

## Retained material

- `runs/rwc01` and `runs/rwc02`: **164 rows, 80 timed edits, all 328 original
  stdout/stderr byte streams**, both summaries, and all six original source
  backups. `metadata.sanitized.json` retains both 63-file source SHA256 maps,
  all engine SHA256/path identities, complete before/after toolchain file/link
  identity maps, workload definitions/order, native contracts, and allowlisted
  environment policy. `records.sanitized.json` retains every native command,
  status, duration, noise measurement, accepted gate, input/output source hash,
  and raw-prefix identity. No phase or failed gate receipt is silently dropped.
- `input/source63` is the complete 63-file fixture reconstructed byte-for-byte
  from the pinned `f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d` Git blobs, not a
  copy of mutable current checkout files. `input/tree.json` retains Git blob
  identity, mode and SHA256 for every file. Every invocation's exact edited
  input is reconstructable from these bytes and the retained historical
  harness, and the portable validator checks its hash, including type errors,
  assertion failures, all five edits, and unmodified recoveries.
- `provenance/0dba7c33e` retains the exact historical harness, all three local
  helper modules, and its test file. The import chain is
  `resident_workload_compare` -> `resident_engine_compare` ->
  `blaze_runtime_compare` -> `blaze_edit_compare`. These files are provenance,
  not scripts to run during offline replay. Their SHA256s are independently
  pinned by the validator. `provenance/fb9e3f14c` retains the later receipt-only
  privacy/restoration hardening plus its tests. The native results predate
  that follow-up, so they are not mislabeled as runs of the newer harness.
- `root/bench-broader-v28-regression.log` is the actual root-owned **710-test
  PASS** receipt. `root/followup-command.json` and `followup-12-mocked.log` are
  the separately executed worker-owned 12-test source-only hardening receipt.
- `SHA256SUMS` inventories every retained file except itself, generated Python
  bytecode, and the derived `validation.json`. The latter is convenience output,
  not an admission oracle. All semantic acceptance comes from raw replay.

Toolchain and binary **identities** are retained, not the multi-gigabyte live
binary/cache/toolchain trees. Offline replay verifies the recorded before/after
identity agreement and known pinned hashes. It cannot re-observe the historic
machine, prove engine build provenance from a hash alone, or attest that a
process really executed. Fresh stores and restoration are historical harness
contracts/receipts, not an independently observed filesystem snapshot. The
source is fully reconstructable and native commands are never executed by the
portable validator.

## Privacy and transformation, intentionally not a raw-metadata archive

Original `/home/jeremy/rwc01` and `/home/jeremy/rwc02` remain unchanged and private.
**Their raw `metadata.json` must never be published** because the historical
harness recorded the full inherited environment. Original row environments are
also filtered through the same explicit allowlist, even where no entries need
removal. This derivative omits all non-allowlisted ambient keys and values,
including any credentials, entirely. It records omission counts, never omitted
key names, values, or value hashes.

Every `transformation.json` identifies private original filenames/SHA256s,
public derivative filenames/SHA256s, the exact environment allowlist, omission
counts, and the transformation policy. For each run, 104 metadata environment
entries were omitted and zero historical process-row entries required omission.
All non-environment metadata/row fields are retained. Raw stdout/stderr,
backups, and summaries are byte-identical to their private originals. The
collector refuses export if an omitted sensitive ambient value of at least
eight characters occurs in exported bytes, but this check is not a general
secret detector. Absolute private paths remain deliberately visible as
historical identities. This is an explicitly documented sanitized derivative,
not a retroactive claim that original metadata was redacted.

Historical private original SHA256 commitments:

| Run | Raw private metadata | Raw private records |
| --- | --- | --- |
| rwc01 | `de1ae993296cb3cd6b42113e8e78d669346501b3eb9a57fe2272237e3c5e0161` | `97612d3da6e929570629914ed76f8bb23e8a4e89dca634510b7e507d7b648110` |
| rwc02 | `c5abb9e504ef87cbbc83dde76d7b31296bfea893d733c7d32db4eac7dcc66f74` | `4172a4a73f8330a2cedd832eb53e9ca0d154e7640aeb7e8cb9105abc6daa288e` |

Checksums are content commitments, not signatures from a trusted third party.
Without access to the private originals a reader cannot prove omitted ambient
values or independently rerun the sanitization. The exporter, policy and
private-original commitments make the limitation explicit.

Historical runs did not include the newer explicit `source_restored` field.
The validator derives `source_restored_derived: true` from full source manifest
agreement without adding a retroactive field to either sanitized historical
metadata file.

## Portable offline replay

Copy this entire directory anywhere, without the repository, `.git`, source63,
engines, caches, toolchain, or original `/home/jeremy` paths:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 validate.py
PYTHONDONTWRITEBYTECODE=1 python3 test_validate.py -v
```

Or invoke `validate.py /absolute/path/to/relocated/bundle` from an unrelated
working directory. Only Python's standard library is needed. The validator
reads relative retained files, never Git, native commands, or original absolute
paths. It validates the complete checksum inventory, pinned source/harness
bytes, transformation commitments, source/tool/toolchain identity agreement,
exact environment/cache policies, every invocation's reconstructed input hash,
command/filter, schedule/order, exact libtest names/counts, type/assertion
oracles and recoveries, all 80 unique executed markers, raw/parsed agreement,
and recomputed medians/exclusion lists.

The test suite coherently re-signs modified derivative hashes, raw-content
hashes, and the outer inventory before semantic checks. It rejects marker
removal/staleness/duplication, coherent name/count changes, missing type/assert
oracles, failed recoveries, timeout/nonfinite duration/rejected samples,
ordinary-function input-hash drift, coherent source/tool/toolchain mutations,
secret-bearing environment maps, shared caches, backup corruption, invocation
order changes, median tampering, missing streams, extra source input, changed
historical harnesses, unsafe paths and symlinks. It also tests export-time
API_TOKEN omission with unchanged private originals and replays a relocated CLI
under empty HOME/PYTHONPATH. Re-sealing is not evidence validation by itself.

`collect.py` documents the original read-only export operation. Do not run it
in an existing bundle because it uses exclusive creation. Recollection requires
private originals, the source Git history, and a fresh export destination, while
offline validation requires none of them. No subsequent evidence packaging step
should run native benchmarks or modify original private receipts.
