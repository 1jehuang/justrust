# Resident semantic subphases v25: actual native evidence

## Accepted change and scope

Source commit `90164925c` atomically changed the three assigned Rust sources and
`docs/resident-semantic-subphases.md`. This bundle retains frozen pre-gate full
files and original backups, not a recapture of subsequently changing production
sources. The frozen documentation intentionally predates native acceptance. The
owned documentation records the later acceptance result.

This is native telemetry acceptance and inclusive attribution, **not a speedup
claim**. The disabled fixed-five comparison's raw -1.60% change was below its 10%
floor. These are scoped, uncontrolled diagnostics, not the comprehensive workload
or a default-promotion gate. The noise proxy does not establish a quiet machine.

## Retained receipts

- `source-ready/`: four frozen full files, three exact original Rust files, the
  new-document absence marker, four-path patch and original/frozen hashes.
- `build/v23/` and `build/v25/`: build/check exits, raw condensed logs, frozen
  selected source manifests and selected source files, engine identity hashes.
  No executables are included.
- `native/v25/` and `native/v25-snapshot/`: original focused gates and exact
  snapshot gates, their logs/exits, admission receipts, full 2444-file hash
  manifests before/after and source-integrity receipts. No full source tree or
  target directory is included.
- `snapshot/`: the exact original Cargo.lock, private resolved Cargo.lock and
  snapshot workspace manifest. The 2444-file copy was initially exact, only the
  private lock was substituted, and source/lock hashes stayed unchanged in the
  recorded before/after receipts. The live nested lock was not modified.
- `journals/`: raw `output.jsonl`, `meta.json`, `summary.json` and other available
  journal metadata for build and native test runs. In particular, actual owned
  API run `20261030-133307142-3479390` passed five tests, and full native run
  `20261030-133629708-3652589` passed 351. The remaining focused snapshot gates
  passed `2 + 3 + 1 + 1 + 3` tests.
- `offline-export.json`: explicit offline projections of retained summaries with
  run IDs, arguments, test counts and summary digests. These are export metadata,
  not a new `justrust export` invocation or a new run.
- `fixed-five/rsb16/` and `fixed-five/rsa22/`: exactly 22 raw runtime rows each,
  with stdout/stderr, suite/records/metadata/summary receipts and original edited
  source. They include version, preparation, actual type-error and assertion
  failures, both recoveries, and five marker-checked edits for reference/engine.
- `profile/`: all six actual positive profile rows, original edited file,
  preparation/stop logs, traces, worker delta logs and state receipt.
- `provenance/`: exact runners and orchestration logs/exits, including the raw
  700-test benchmark-harness regression log. Runners preserve original absolute
  paths for provenance. **Do not execute them to replay evidence.**
- `harness/`: the offline-retained benchmark harness scripts imported by the
  runners. Collection and validation did not rerun native code or workloads.
- `collection.json`: original origin strings, byte-identical copied paths and
  hashes. Origins are provenance only, never opened by the validator.

## Actual phase proof and attribution limit

`validate.py` independently replays the four-file unified patch in memory against
original backups, anchors accepted frozen source hashes and ties them to build
and snapshot receipts. It verifies exact native run/filter/count identities and
actual API test names from full journals, not just condensed summary claims.
It verifies raw runtime failures/recoveries and marker-checked edits.

For every positive profile row, raw trace events must exactly match the state
receipt. Each of the five semantic phase intervals must occur exactly once on
the same `(pid, tid)` as its containing actual semantic outer interval. Adjacent
boundaries allow at most one microsecond of transport truncation. Outer skeleton,
filter and broken-in labels, positive collect durations and zero transport drops
are required. Callback/file rows are independently replayed from raw worker logs.

**First row is not one file.** Row 0 contains two same-thread phase groups on two
different threads, and two raw file rows: `FileId(540)` with 373 callbacks and
`FileId(33)` with 231 callbacks. It retains 17 spans and excludes 249 stale spans,
with zero drops. Rows 1 through 5 have one semantic thread group, one raw
`FileId(540)` row with 373 callbacks, nine spans and zero drops.

Trace spans have thread identity but no file argument. Callback lines have a
file ID but no thread ID. The evidence does **not** establish which first-row
thread corresponds to which file, so neither validator nor report invents that
bijection. Per-thread intervals and per-file callback rows remain separate.
Never sum overlapping files as critical-path time, or add callback sums or
semantic outer durations to child phases. Integer microsecond truncation can
represent a nonempty actual interval as zero microseconds.

## Offline replay

From any checkout location:

```sh
python3 -B bench/results/resident-semantic-subphases-20261030/validate.py
python3 -B bench/results/resident-semantic-subphases-20261030/test_validate.py
```

The validator uses only its bundle and the Python standard library. The control
runner copies the bundle into disposable private scratch directories, repairs
all mutable hashes after each mutation, and demands rejection by a specific
semantic constraint. It also executes the copied validator from a relocated
bundle under an independent working directory. No compiler, resident daemon,
Cargo, justrust, network, source checkout or workload runner is invoked.

`SHA256SUMS` inventories retained inputs and replay code. Generated
`validation.json`, `mutation-controls.json` and `audit.json` are excluded from
that inventory to avoid self-reference. The inventory is an integrity checksum,
not a cryptographic signature or authentication against an adversary replacing
the validator. Re-signed controls test consistency and semantic constraints,
not provenance authenticity.

The retained baseline passed. Fifteen fully re-signed controls were rejected for
the intended semantic reasons and relocated replay passed. Additional strict
build-identity, source-integrity, export-coverage and outer-wall checks were then
added, and the strengthened validator baseline passed. Final independent replay
of the complete strengthened control runner was deferred at the coordinator's
request to protect the next v26 timing window. `audit.json` records this ordering
explicitly. No earlier control result is presented as a new run during that
protected window.
