# Selective diagnostics traversal: rejected experiment

## Decision on October 30, 2026

**Reject v24. Do not promote the guard or its source-test additions.** The
small traversal change passed native correctness gates but its latency benefit
did not replicate in the independent reverse-order diagnostic. Production
`lib.rs` and `tests.rs` were restored byte-for-byte from the frozen original
backups at 20:10:37 UTC. Both `cmp` checks and `git diff HEAD --exit-code` passed
at that boundary, recorded in the bundle's `restoration.json`. Ownership was
then released. Later coordinator-owned edits to those files are not part of
this rejected experiment and must not be reverted on its account.

## Exact bounded proposal

The candidate wrapped the existing unfiltered syntax descendants loop in
`if bodies.is_none()` inside `semantic_diagnostics_asked`. The existing iterator
filter rejected every node for selective calls but still visited the full tree.
The candidate retained the three original handler calls, their arguments and
order. It changed no diagnostic filtering policy, cache/query policy, body
selection predicate, or accepted flat-text hashing helper.

The entire rejected candidate, both original Rust files, its initial pending-
gates documentation, exact scoped patch, source-freeze SHA-256 identities and
`READY.txt` are retained in
`bench/results/resident-selective-traversal-20261030/source-ready/`. The live
production source is not the candidate. `validate.py` reconstructs all three
candidate files from the original files and patch in memory and verifies the
production hunk is precisely the bounded guard.

## Native gates and invocation failures

Root's candidate check and release build passed with the frozen source
identities. All attempts, including failures, are retained separately:

1. Root-workspace testing of `ide-diagnostics` failed before fixture compilation
   because the crate requires dev dependencies and is not a workspace member.
   Exit 101, run `20261030-125716550-3148215`.
2. Testing the actual nested workspace with `--locked` also failed before
   fixture compilation because its existing lockfile was stale. Exit 101,
   run `20261030-125849389-3153839`. The original lockfile was not rewritten.
3. A metadata attempt confirmed the pinned Cargo did not support
   `--lockfile-path`. Its stderr and exit 1 are retained, not relabeled success.
4. Root copied the actual 2444-file, approximately 30 MB nested workspace into
   a private source snapshot, used adjacent patch-crate symlinks and the same
   toolchain/shared target, and resolved only the private lockfile offline.
   Before/after manifests report original files unchanged and only private
   `Cargo.lock` changed. Both private lock versions and the workspace manifest
   are retained. The bundle does not contain the whole workspace copy, build
   target, caches, or binaries.

The actual exported diagnostic APIs, not a copied traversal algorithm, passed
three tests under `resident_selective_traversal`, run
`20261030-130140108-3164280`. These cover full-API syntax-warning retention and
selective omission, full/all-bodies semantic error equality, and named-body /
no-body error selection. **This is representative source-snapshot native
acceptance, not successful original-workspace `--locked` acceptance.** It does
not repair or erase the first two failed invocations.

Additional root gates passed: flat-text helper 2, diagnostics timing 3, graph
reuse 1, listed-root 1, and telemetry 3. The full resident suite passed **351
passed, 0 failed, 0 ignored**, run `20261030-130705291-3342136`. Raw journals,
exact passing test names, summaries, invocation metadata, source boundaries and
positive-count admission receipts are retained in the portable bundle.

## Measured rejection evidence

All values below are descriptive warm-workload diagnostics. They are not a
comprehensive session-weighted workload, cold-build claim, statistical
significance test, or default-promotion gate. No samples were excluded.

| Diagnostic | Accepted v23 baseline | Rejected v24 candidate | Blaze reference |
|---|---:|---:|---:|
| Fixed five, engine median | 118.031 ms | 111.246 ms | Separate reference medians per suite |
| First balanced 42 edits | 97.0913835 ms | 92.8579205 ms | 102.7449355 ms |
| Independent reverse balanced 42 edits | 96.2947460 ms | 98.9767610 ms | 103.7726520 ms |

The identical-mode fixed-five RSB15/RSA21 comparison reports raw median change
**-5.75%**, below the requested 10% floor. Its reference-normalized ratio got
**10.77% worse**, exposing a reference/session confound rather than an accepted
improvement. `provenance/suite-v24-snapshot-compare.log` preserves the full
comparison output.

Each 42-edit diagnostic used all six stack permutations, identical edited bytes
across baseline/candidate/reference, and all 126 edit rows plus 15 preparation /
negative-control / recovery rows. The independent reverse diagnostic made the
candidate **2.785% slower than the accepted baseline**. Therefore the first
apparent benefit was not replicated and the source change was rejected.

Both 141-row states and all 564 stdout/stderr streams are retained along with
the original runners. The fixed-five suites retain all 44 process rows and raw
streams, original source, source/toolchain/binary identity boundaries, summaries
and full suite JSON. Timing and noise receipts do not establish a quiet machine
or isolate causality. Private-store cleanup is not a test result or PID-exit
attestation.

## Portable offline replay

From any copied bundle location, run:

```sh
python3 validate.py
python3 test_validate.py --scratch "$JCODE_SCRATCH_DIR"
```

The stdlib-only validator reads only bundle-local files. Recorded absolute
paths are historical provenance, never dependencies of replay. It verifies
825 byte-identical collected artifacts, frozen source/patch reconstruction,
unchanged selected shared sources and helper, 2444-file copy receipts and the
private-lock delta, real raw native counts/names/statuses, preserved original
failures, exact runtime test names/counts, exact edit markers and reconstructed
edited-byte hashes, expected type-error/assertion exits and recovery, balanced
permutation order, identity boundaries, no exclusions, medians and comparison
math. It never invokes the source runners, native tools, daemons or workloads.

Relocation passed. Eleven semantic corruption controls are rejected even after
collection integrity is recomputed, including wrong/duplicate edit markers,
zero-test native output, changed negative-control exit, erased original failure,
patch drift, copy-source drift, corrupted summary math, missing recovery,
changed permutation order and changed private lock bytes. Marker mutations also
update the state's stream hashes, so rejection is not merely a checksum failure.
The mutation harness prohibits subprocess execution.

Integrity hashes are reproducibility checks, not signed provenance or proof
that arbitrary coordinated fabrication is impossible. The 2444-file manifests
are retained receipts, not a standalone copy of every original source byte.
The source-only worker executed formatting/diff checks and these offline
evidence checks, but no native build, test, benchmark, or workload. Root ran the
native and measurement gates. Only this rejection document and evidence bundle
are eligible for the evidence commit.
