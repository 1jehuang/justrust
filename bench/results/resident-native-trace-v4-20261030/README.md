# Native resident telemetry acceptance, 2026-10-30

This directory is acceptance evidence, not a speed comparison. Frozen v4
`jr-engine` SHA-256:

```text
deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8
```

The actual workload is `test --lib -- summary::tests:: --test-threads=1
--nocapture` on the pinned justrust source used by the root native gate. The
four selected tests execute in the generated libtest executable. No source was
changed by `accept.py`, and its before/after source SHA is identical. The root
edit/error runner backs up and restores the source bytes and reports restoration.

## Retained gates

- `root-off-on-off/`: five root invocations, including initial settle, opt-in
  with both RA_JIT settings, both BLAZE aliases, and telemetry disabled on both
  sides. Every run succeeds. Every compared executable retains the exact same
  SHA-256, device, and inode. Opted-in traces contain actual client phases.
- `root-errors-recovery/`: edited body succeeds (0), a real type error fails
  (101), recovery succeeds (0), an assertion fails (101), and recovery succeeds
  (0). All five produce traces. Full compile and test failure diagnostics are
  retained. Edited body traces contain client, daemon, and worker phases.
  A compile failure correctly has no test-execution phase. The error reply
  schema does not carry the successful worker answer's detail-span suffix,
  so its trace has client/daemon spans but not the successful worker details.
- Top-level `acceptance.json` and matching stdout/stderr files: eight additional
  modes, all with the same daemon PID/version, complete worker PID/project/log
  identity set, and executable SHA-256/device/inode identities. The modes are
  disabled (0), stats-only (0), trace-only (0), BLAZE trace-only (0), a returned
  `anyhow` command error (1), a refused unsupported command option (101), trace
  writing to a non-directory (0), and disabled again (0). Stats output appears
  only in the stats-only invocation. Trace-only modes produce nonempty phase
  traces without the timing table. Trace-writing failure preserves success and
  emits its failure diagnostic.

The lightweight gate pins the original root client PATH. The first attempt
inherited a different PATH from the worker shell, adding a duplicate local-bin
entry and a JVM bin directory. That correctly selected a different worker
configuration on a **disabled** invocation, before any opt-in was tested. The
failure, original status snapshots, and raw output are retained in
`initial-unsettled-control/`; it is not reclassified as a pass. Process
environment inspection identified the PATH difference. The corrected gate
settles the original configuration, then takes a baseline and compares every
mode strictly. Both already-live workers remain in the compared set.

`old-missing-trace-control/` preserves the earlier failed missing-trace attempt,
including its failed state, empty accepted row list, and full available output.
It must not be mistaken for a timing observation or deleted after the fix.

## Explicit limitation

`metadata-hard-exit.stderr` and `known_metadata_hard_exit` in `acceptance.json`
show that an invalid manifest still exits 101 with its original diagnostic but
produces **no trace**. The pre-existing `cargo/metadata.rs::no_deps` calls
`process::exit` below the owning boundary, so guards do not unwind and command
finalization is bypassed. This is an observed remaining limitation, not a
successful error-finalization case. Native source was frozen and that file was
outside the telemetry worker's source ownership. Aborts, external termination,
and other lower-level hard exits are likewise not covered by Result finalization.

These warm acceptance runs do not establish cold-start coverage, universal
manifest/build-script/proc-macro coverage, independent daemon/worker trace opt-in,
or a performance win. The root gate separately compiled production v4 and
actual graph/root/telemetry unit-test filters. The root reports 700 tests passed.
That report is not substituted for the retained native acceptance traces here.
No native bootstrap or rebuild was run by this evidence worker.

## Offline verification

From the repository root:

```sh
python3 bench/results/resident-native-trace-v4-20261030/validate.py
python3 bench/results/resident-native-trace-v4-20261030/test_validate.py -v
```

The validator passed all retained evidence checks, including exact trace hashes,
nonempty actual phases, wall-time/breakdown equality, expected statuses, and
negative-control preservation. All 12 positive/mutation unit tests passed.
Mutations reject missing phases, negative durations, double-counted breakdowns,
altered statuses, changed worker flags, changed trace bytes, changed artifact
inodes, hidden compile diagnostics, and deletion/reclassification of old failure
controls. Tests run on temporary copies and do not launch the engine.

`accept.py` is the retained live acceptance runner. It is specific to the local
frozen namespace and refuses a different v4 tool hash. It need not be rerun to
validate this evidence. Timing fields are descriptive process measurements only,
not an uncontaminated before/after benchmark, and are not speedup evidence.
