# Rejected selective traversal v24, October 30, 2026

**Do not promote this candidate.** Its correctness checks passed, but the
latency benefit did not replicate. Only evidence and rejection documentation
are committed. The source patch and test additions are retained as inert
experimental artifacts, not live production code.

## Result

- Fixed-five RSB15/RSA21 engine medians: 118.031 -> 111.246 ms, -5.75%,
  below the requested 10% floor. Reference-normalized ratio got 10.77% worse.
- First balanced 42-edit diagnostic medians old/new/Blaze:
  97.0913835 / 92.8579205 / 102.7449355 ms.
- Independent reverse balanced 42-edit diagnostic:
  96.2947460 / 98.9767610 / 103.7726520 ms. Candidate 2.785% slower than old.
- Actual API tests: 3 passed. Full resident tests: 351 passed. Raw journals
  retained. No native code was executed by the evidence collector or validator.

These are descriptive scoped warm diagnostics, not comprehensive workload or
significance/default-promotion gates. A quiet machine and causal isolation are
not established by these records.

## Layout

- `source-ready/`: exact originals, all three rejected candidate files, initial
  readiness receipt, SHA manifests, and source patch. The pending-gates doc is
  intentionally preserved verbatim, not rewritten with the final decision.
- `build/`: baseline/candidate selected source freezes, check/build raw reports,
  source boundary and binary identity receipts. No binaries or target output.
- `native/`, `journals/`: native logs and raw JSONL output, exact invocation and
  summary records. Both original exit-101 attempts remain. Unsupported alternate
  lockfile-path metadata stderr and exit 1 remain distinct.
- `snapshot/`: the original/copied private lock before resolution, private lock
  after offline resolution, and actual copied workspace Cargo.toml. The 2444-
  file before/after SHA receipts are in `native/v24-snapshot/`. The actual API
  gate is source-snapshot representative acceptance, not successful original
  nested `--locked` acceptance. Only private Cargo.lock changed in the snapshot.
- `fixed-five/`: full RSB15/RSA21 suites, metadata, rows, summaries, raw streams.
- `interleaved/`, `interleaved-reverse/`: 141 rows each, all 282 streams each,
  original edited source, identities and medians. First exact edit markers
  start at 1801, reverse markers at 2001. Each uses all six stack permutations.
- `provenance/`, `harness/`: original native and timing runners and harness
  source, retained for inspection only. **Do not execute these historical
  runners to replay evidence.** They reference original machine paths.
- `collection.json`: 825 byte-identical artifact copies with historical origins
  and SHA-256 identities. Replay never follows those absolute origin paths.
- `restoration.json`: timestamped byte-exact Rust restoration checks and release
  of production ownership. It does not assert later coordinator edits are clean.

## Offline replay

```sh
python3 validate.py
python3 test_validate.py --scratch "$JCODE_SCRATCH_DIR"
sha256sum -c SHA256SUMS
```

Set `JCODE_SCRATCH_DIR` to a writable directory for throwaway mutation copies,
or pass an explicit `--scratch` path. Python 3.9 or later is required. Only
stdlib modules are used. No network, compiler, native tests, daemon, workload,
or recorded absolute path is needed. The mutation harness prohibits subprocess
execution and cleans only its own newly-created temporary directory.

`validate.py` independently recomputes source-patch reconstruction, selected
source and helper identity boundaries, exact raw native test counts/names and
statuses, preserved invocation failures, private lock identity/delta, complete
runtime row coverage, type-error and assertion-failure/recovery behavior,
exact markers, source-edit hashes, balanced order, identity boundaries and
median/ratio math. Both 141-row diagnostics and all fixed-five rows are checked.

The eleven negative controls re-sign collection hashes before validation, and
marker controls also update raw-stream hashes in state. All must still fail
semantic checks. Relocation and control results are retained separately from
native acceptance. `SHA256SUMS` covers collected and authored evidence files,
but is not itself an authenticated signature. Mutation controls deliberately
exercise the validator's collection/raw semantic checks, not this outer seal.

See `docs/resident-selective-traversal.md` in the repository for the full
rejection decision, native invocation limitations and production restoration.
The 2444-file manifests attest the original comparison, but the bundle does not
include every source byte of that approximately 30 MB snapshot. No standalone
rebuild or original nested-lock correctness guarantee is claimed.
