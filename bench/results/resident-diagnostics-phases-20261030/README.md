# Native diagnostics phases: portable v22 evidence

Instrumentation acceptance only. **No speedup claim.** Source commit: `1a0326d59`.
The accepted frozen `main.rs` SHA256 is
`56e2045c4f57a04631bdd206b8141e62e3a706abef0a940a59fea1e2b1df818a`.
This directory deliberately retains the accepted instrumentation baseline, not
later borrowed-text hashing integrations or the current live toolchain tree.

## Recheck without the original machine

From any directory, run `python3 /path/to/this/bundle/validate.py`. It uses only
stdlib and bundle-local files, never the original absolute paths, live stores,
network, Cargo, native executables, or live source. The raw artifacts intentionally
preserve original paths and bytes as provenance. `collection.json` maps original
sources to local copies and their exact hashes. `manifest.json` seals the complete
portable inventory except the manifest itself. `seal.py` regenerates that inventory
when intentionally maintaining the bundle. `collect.py` is the original-machine
copy recipe, not required for portable validation and not a workload runner.
The copied root scripts/harness are provenance, not portable replay commands.
Run `python3 -B test_validate.py` from this directory for four evidence-only
controls: a relocated positive bundle and integrity-retied missing-phase,
transport-drop, and raw zero-test rejection. No native workload is invoked.
The offline validator and all four controls passed after the protected native
measurement window closed. Export-generated install/update/telemetry state is
not retained, only original copied journals and the 15 slim exported runs.

`journals/` contains exact raw justrust runs and indexes, while `exports/` contains
bundle-local path-redacted exports made from those copied journals. Exports are
not substitutes for raw positive libtest counts. Runtime suites retain top-level
stdout/stderr and metadata only, not native binaries, dependency stores or caches.

## Acceptance and failure controls

- `frozen/v21/` retains the original v21 source/receipt and repaired v21b source/receipt.
  `frozen/v22/` retains the accepted direct-transport full source/doc/receipt.
- `build/v21` and `build/v22` retain eleven selected frozen source files, build logs,
  selected-source identity manifests and engine identity. No live candidate source
  was recaptured after root took ownership for the next optimization.
- `profiles/v21` is a failed positive-marker control. Client-only telemetry flags
  did not enable diagnostics in workers. `profiles/v21c` is a second failed control:
  worker callback work was positive but tracing detail was absent. The copied
  `profiles/v21c/worker-logs-after-failure/` retains daemon/worker logs after that
  failure. Root observed the actual process flag at capture time, but no separate
  `/proc` receipt was retained, so process-flag observation is not independently
  attested by a portable environment artifact.
  Both failures restored the workload source. Neither supports zero-cost inference.
  The exact root cause of the old recorder loss was not established.
- `profiles/v22` contains six actual runtime rows: client export off/on/off-again,
  then three real body edits. A dedicated daemon inherited exact startup
  `RA_JIT_DIAGNOSTICS_STATS=1` into its workers. The client export toggle does **not**
  disable the worker's timing. All three edit traces have four positive labels.
- `native/v22/positive-test-admission.json` and its raw journals require exact
  focused counts 3 timing + 1 graph reuse + 1 listed-root + 3 telemetry.
- `suites/rsb13` and `suites/rsa19` are fresh disabled-instrumentation validation
  suites. Older RSB12/RSA18 suites and v21 gates are retained as context, not pooled
  into a general performance comparison. Timing noise and descriptive results
  remain visible in the unmodified suite records.

## First pass race versus corrected full-native acceptance

The first full-native run `20261030-121731644-2778234` reported 349 passing tests,
with all eleven **selected** source hashes passing. That did not detect an extra
unlisted `diagnostic_hash.rs` file introduced during the pass. The original
worker-produced `race/original-source-ready.txt` receipt records creation at
19:16:27.868Z, immediate scratch-only refinement request at 19:16:27.869Z, scoped
removal at 19:19:15.481Z and absence verification at 19:19:22.741Z.
`race/diagnostic_hash.pre-refinement.rs` preserves the exact original bytes with
SHA256 `ac81c907f0170e30b625c19da9b85a9b93bda566bce16649a91457603c2475cf`.
This is a provenance limitation, not a native test failure, and is never concealed
by eleven-file hash success.

Root ran a distinct corrected full-native test after restoring the tree:
`20261030-122938036-2828152`, **349 passed, 0 failed, 0 ignored**, exit 0.
`native/v22-restored/` and `journals/unit-v22-restored/` preserve that pass separately.
`provenance/full-native-v22-restored.sh` explicitly checks helper absence at both
boundaries in addition to the eleven before/after selected-source hashes. Do not
merge the two passes or retroactively treat the first as frozen-tree acceptance.

## Actual observations, not saved time

| Actual edit | Skeleton/key/CLEAN us | Inclusive semantic us | Filter/cache us | Outer wall us | Asked callbacks | Callback cumulative ns |
|---|---:|---:|---:|---:|---:|---:|
| edit-one | 1099 | 98950 | 1 | 101261 | 373 | 7969758 |
| edit-two | 1050 | 88040 | 2 | 89626 | 373 | 7676528 |
| edit-three | 1109 | 64988 | 4 | 66549 | 373 | 8332561 |

The corresponding direct transport receipts were `(retained, stale, dropped)`
`(4,94,0)`, `(4,0,0)`, `(4,0,0)`, cap 4000. Stale work is explicitly counted.
The four labels are actual executing-thread intervals, not residual fabricated
spans, with no duration threshold. The semantic interval **includes** asked
callback source/hash/predicate work, so callback durations must not be added.
Outer calling-thread wall overlaps work/waits on diagnostic threads. Cumulative
thread durations are not critical path. Collection/reporting adds overhead.
The source contract is documented in `docs/resident-diagnostics-phases.md` at the
source commit, with the original frozen version retained here.
