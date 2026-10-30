# Native resident phase telemetry

The native whole build and test client records phase telemetry when explicitly
requested. This instruments the actual `cargo::whole` path, not the older
artifact-replay path. Telemetry measures time and does not establish a speedup.

## Opt in

Set `RA_JIT_STATS=1` for the stderr timing table. Set
`RA_JIT_TRACE=/absolute/path/trace.json` for a Chrome trace JSON file, or give a
directory to create a uniquely named JSON file there. The trace contains
`traceEvents` and `raJit` wall-time/breakdown fields. Both options may be set.
`BLAZE_STATS` and `BLAZE_TRACE` are public aliases normalized by the existing
entry point, with the BLAZE spelling taking precedence if both are present.
An empty trace path alone does not enable client spans. As in the existing
trace writer, presence of the stats variable enables its output even if its
value is `0`.

Without opt-in, new client phases use a cached flag and do not read clocks,
allocate spans, decode worker span payloads, or print timing output. Existing
daemon/worker recording and the ordinary Cargo-like `Finished` line are
unchanged. The trace options are not propagated to checkout `env!` inputs under
either spelling, including explicitly populated checkout environment entries.
They are absent from the existing worker/build environment allowlists and do
not change worker configuration, checkout keys, or artifact keys.

## Recorded operations

Client phase scopes cover command dispatch, whole build/test, metadata,
planning, checkout construction, request construction, client/daemon exchange,
artifact verification/installation, program execution, test execution, and doc
tests. Only operations actually executed appear. For example, `test --no-run`
has no test-execution phase. Both ordinary and unit-test checkouts are measured
when both are needed. Nested client spans overlap intentionally. The existing
breakdown attributes each interval to the latest-starting client phase, then
the deepest available daemon/worker phase, rather than adding overlapping spans.

`Reply::Synced` already carries daemon spans and successful worker answer lines
may carry a ` spans=` JSON suffix. The whole path collects both using the same
trace helpers as the legacy path. Span parsing is best effort. Missing or
malformed span JSON does not fail a build. The answer suffix is ignored when
recognizing `ok unchanged`, so telemetry cannot turn an unchanged answer into
a reported rebuild. Diagnostics and error replies are not filtered or hidden.
No request or reply schema changes are needed.

`cargo::command` finalizes after its result-producing dispatch returns, before
exiting with its original status or propagating its original error. Scoped
phases record on ordinary error returns as well as success. Trace-writing
failure is diagnostic-only and cannot replace the command's exit status.
Hard process exits in lower-level code, aborts, and external termination cannot
unwind a client scope. In particular, the pre-existing metadata failure exit in
`cargo/metadata.rs::no_deps` must be handled by its owner if that path also
requires finalization. This change does not suppress its diagnostic or change
its status 101.

## Validation contract

Rust unit coverage in `cargo/mod.rs` checks telemetry spelling classification,
avoids accidentally stripping unrelated inputs, and checks the disabled phase
representation without mutating process environment. Pure local tests in
`cargo/cli.rs` verify both spellings are excluded from build and worker identity
selection, using the private classifiers without widening their visibility.
The root native gate owns compilation and real workloads: cold/warm successful
tests, a failing test, a compile error, command errors, trace-only/stats-only and
disabled invocations, alias handling, and artifact/worker identity stability.
Check complete original diagnostics and exact exit statuses alongside nonempty
trace events and the expected actual-operation phase names. Compare disabled
runtime independently from opted-in traces. The initial implementation phase
ran no native workload or bootstrap. The subsequent frozen-v4 acceptance phase
ran only the additional lightweight checks described below, without rebuilding.
No performance claim is made here.

## Frozen-v4 acceptance evidence (2026-10-30)

Retained evidence is in
`bench/results/resident-native-trace-v4-20261030/README.md`. The frozen binary
SHA-256 is `deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8`.
Root off/on/alias/off and real edited-body/type-error/assertion/recovery gates
passed. Eight additional modes passed exact expected statuses: disabled (0),
stats-only (0), trace-only (0), BLAZE trace-only (0), a returned command error
(1), refused option (101), trace-write failure with successful tests (0), and
disabled again (0). Across the additional modes the daemon/worker PID and log
identities and executable SHA/device/inodes remained identical. Full raw logs,
traces, original failed missing-trace control, and a PATH-mismatch negative
control are retained. The offline validator and all 12 positive/mutation tests
pass without launching the engine.

The invalid-manifest metadata hard exit was tested explicitly: status 101 and
its diagnostic are preserved, but **no trace is written**. This remains a
known limitation outside the owned telemetry source changes, not a passed
error-finalization case. These warm acceptance checks are not cold-start or
comprehensive language/project coverage and establish no speedup.
