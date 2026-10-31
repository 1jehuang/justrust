# v79 client planning subphases: portable historical attribution

Authority is immutable shipped source commit
`34dc0a188534edbe92d0b848b9dfa68de71d8bb3`, not the later live v80 resolver
instrumentation. These are projections of root-executed October 31, 2026 native
journals, not newly executed native results. Only Python offline replay was run
while packaging. The source retains its licensed Blaze-derived MIT/Apache-2.0
lineage, not a clean-room claim.

## Preserved positive evidence

- Eight actual runtime rows, four source edits each followed by one noop. The
  reachable nonce is `JR_RUNTIME_EXECUTED_7901` through `7904`, once per actual
  stdout, repeated only by its corresponding noop. Each row runs the same four
  named summary tests successfully with 144 filtered tests. The original source
  plus the exact byte edit recipe independently reconstruct every source hash.
- Every row retains the five actual complete trace events: inclusive parent
  `client planning` and its four successive rustc-host, target-cfg, asked-setup,
  and owned-resolver subspans. Exact original microsecond `dur`, timestamp, PID,
  thread, category and complete-event type are preserved. Only these fixed
  labels are exported, never ambient environment or arbitrary trace args. Public
  original process counts are three on edits, two on noops.
- Actual native gate exits all zero, all **53 codegen and 373 resident named
  PASS lines**, four recorder-run projections, and historical Python regression
  **761 tests, 51.426 seconds, OK**. Selected report lines are literal original
  journal lines, not a fabricated raw transcript. Recorder originals remain in
  `data/runs/resident-planning-v79` in the repository, not a replay dependency.
- Full native source3757 hash inventory and successful after-check count,
  unchanged source63 and toolchain4020 inventories, identical native executable
  identity, successful preparation and stop0 without timeout/launch error.
  Hashes and relative inventory paths are retained, no 3757-blob source import,
  toolchain files, binaries, caches, targets or giant runtime snapshots.
- Complete owned original/candidate `metadata.rs`, existing `Phase` source,
  exact seven-line instrumentation patch and exact initial-to-finaldoc patch
  replay. The native inventory joins the candidate and existing guard bytes.
  Frozen accepted documentation joins the immutable authority, before this
  package's evidence-only link addition. Archived drivers are text only and
  are never imported or executed by replay.

Recomputed inclusive ranges, microseconds, over all eight rows:

| Label | Min | Max |
| --- | ---: | ---: |
| client planning | 28930 | 35106 |
| client planning rustc host | 2669 | 2985 |
| client planning target cfg | 801 | 998 |
| client planning asked setup | 19 | 26 |
| client planning owned resolver | 25154 | 31148 |

The parent includes the subphases. Do not add parent and subphase times as
separate wall time. Guard overhead and uncovered planning work prevent an exact
sum promise. This coarse resolver span does not identify its internal cause.

## Offline replay and adversarial tests

Copy this directory anywhere. Only Python stdlib and bundled files are needed:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

63 offline tests passed. They include coherent resealed stdout+receipt,
source-pair+inventory, timing+range and historical-input-receipt mutations,
missing/duplicate labels, nonces, process errors, source/toolchain changes,
native named counts, stop gates, privacy and contract changes. Semantic replay
runs before fixed historical commitments. A coherent timing rewrite can remain
internally plausible but still fails the fixed original observation commitment.
A mutable SHA inventory alone is integrity, not historical authenticity. Trust
requires the reviewed validator and Git commit, not a resealed manifest.

Relocation uses the same Python interpreter with isolated `-I -S -B`, empty HOME
and PATH. After stdlib preload an audit hook forbids outside file/directory
reads, writes, subprocess execution, native-library loading and networking.
Actual denied probes exercise all six restrictions. No engine, Rust compiler,
Cargo, lock operation, installation or native benchmark was run by this worker.
Python replay checks preserved historical records, not a new native measurement,
and cannot independently establish the honesty of the original clock/kernel.

## Privacy, provenance and preservation

`export_private.py` is separate read-only authoring machinery. Inputs are
explicit frozen-v79 paths plus immutable Git blobs, not the working tree. It
projects fixed fields/labels and literal named-test/stdout line grammars, checks
originals unchanged after export, and records original SHA256 and byte size in
`private-inputs.json`. Relative input roles replace the private root. The profile
recipe replaces only five explicit author-supplied private root paths with
public roles. No ambient environment is exported. A public relative fixture
inventory path containing `home` is not a private absolute path.

`SHA256.json` seals package files. `RELEASE` names fixed observations, historical
aggregate and owned-source hashes. Original raw trace/stderr/private state hashes
are provenance receipts, not claims that their full bytes are published or can
be recovered from the projection. No new historical compare stdout is invented.

**Contract:** diagnostic attribution only, no before/after comparison, no
speedup, no instrumentation-overhead estimate, no general performance claim,
no default promotion, no statistical significance and no comprehensive
session-weighted benchmark. Noise/contamination remains **unknown**. A quiet
machine is not established. v78 context in the frozen document is not a paired
comparison. Native source ownership and live v80 work remain untouched.
