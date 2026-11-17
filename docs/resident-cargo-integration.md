# Native resident Cargo integration, 2026-11-17

## What is now connected

An explicit experimental `justrust cargo test` invocation can use native
`jr-cargo` and a native `jr-rustc` worker on the authenticated hosted build
machine. This is an ordinary Cargo dependency/build execution path, not a
standalone resident-protocol driver and not the Blaze-derived compiler.
It does **not** mean the entire justrust repository is now compiling resident.

The native Cargo executor owns routing, transport and worker lifetime. Eligible
requests enter the compiler directly. A worker survives source edits, a compiler
error, and the next successful build. It retains the owned dependency graph and
encoded frontend archive in RAM. The compiler still creates fresh per-request
compiler context and imports eligible archived values. This is not retention of
an unrestricted live `GlobalCtxt`, nor arbitrary live Typeck/MIR references.

The initial admission boundary is deliberately narrow: Linux GNU, serial
frontend/backend/linker capacity, fixed cwd/argv/effective environment,
std-only crates. External crates, proc macros, custom compiler wrappers and
other unsupported requests remain on the ordinary compiler path. In particular,
`--extern` rejection means this slice does not cover the dependency-heavy
justrust client crate itself.

## Correctness and scheduling boundary

- Cargo probes native compiler capability before attempting RPC.
- Worker identity includes exact compiler/sysroot identity, arguments, cwd and
  effective environment. Observable environment is preserved. Jobserver transport
  variables are deliberately replaced by request-scoped accounting.
- Cargo retains its build slot until completion. `RequestLease::begin_serial()`
  reserves real private capacity and offers no extra frontend/backend/linker
  capacity. Idle workers do not retain Cargo's jobserver descriptors or tokens.
- Compiler output uses transferred pipe descriptors. Cargo retains its normal
  diagnostic and metadata-pipelining path.
- Pre-commit refusal may fall back. A committed compiler failure is never replayed
  by falling back to another compiler process.
- The worker is a process-group leader. Disconnect cancellation kills descendants,
  including linker-like children. Request, idle and memory limits bound lifetime.
- The private socket directory must be canonical, user-owned and mode 0700.
  Linux Unix-socket path length is checked by the acceptance harness.

The server lives in
`toolchain/rustc/compiler/rustc_driver_impl/src/resident_cargo.rs`; the Cargo
executor client lives in `toolchain/cargo/src/compiler/jr_resident.rs`.
Neither is a permanent shell interception layer.

## Frozen candidates and evidence

These are unpublished explicit-path candidates, not installed releases:

| Component | SHA-256 |
| --- | --- |
| Compiler driver `librustc_driver-b2a9739b03c30db5.so` | `36fe816110ac6bf4b46745c484e600b15021f31d49387ebc2729c80832c9bc6c` |
| Portable native Cargo | `98d4a05a03a91e001323de04fe4295dc94d9ccd967a5afa151b5188961609483` |
| Explicit client candidate | `0bca5a1871203709f7a13746694e73ecbc1e777390529a1452cea01556fa1870` |

The compiler sysroot and its libraries were built together. The client candidate
was built by `justrust run --bin justrust -- --version`, which executed locally,
then frozen and transferred to the authenticated hosted machine. Client checks,
focused tests, clippy, full library tests and the real acceptance below ran on
hosted. No source candidate replaced the published default client or toolchain.

Account-private original receipts are available through:

```text
justrust records show bench rc0321
justrust records show bench rj0335
```

`rc0321` passed three repetitions through native Cargo directly.
`rj0335` passed three repetitions through the actual justrust client and native
Cargo. Each compares resident-off/on behavior using the same candidates and
serial flags. Both archive original shipping `src/remote_sync_queue.rs` bytes,
compiler/Cargo identity, full stdout/stderr, actual compiler traces and semantic
checks. The client receipt also verifies the client's unchanged binary digest.

Every repetition requires:

1. Initial successful build and six executed named tests.
2. A Cargo-fresh control that must not count as a compiler request.
3. Forced same-source recompilation, not just a cached binary execution.
4. A real source-body edit with an executed changed nonce.
5. Deliberate compiler error with status 101 and normal Cargo diagnostics.
6. Recovery and restored-source recompilation using the same worker key/PID.
7. Wrapper fallback with correct executed test results and no resident request.

In `rj0335`, the three on-arm worker PIDs were 970015, 971724 and 973427.
Each survived all six actual compilations and reached five cumulative graph
hits and five archive hits. Edited builds also reported eligible encoded
Typeck/MIR loads. These counters do not imply arbitrary typed-context retention.

The first actual-client attempts are preserved as failed receipts `rj0331`,
`rj0333`, and `rj0334`. The first two used unsupported routing-flag placement.
The third already proved worker reuse but failed the acceptance parser because
owned libtest appends numeric durations to named test results. The harness now
accepts only that strictly numeric suffix, with a regression test rejecting
arbitrary suffixes. Failures were not relabeled as passes.

Full hosted client library regression run
`20261116-193430240-2708236` passed 1,231 tests with 21 ignored. Hosted check,
focused selection tests and warning-free clippy also passed. The Python harness
has 18 passing mock tests; those are not native acceptance evidence.

## Reproduce without installing experimental components

On an already-authenticated build guest, provide immutable component paths:

```sh
python3 bench/stable_resident_cargo.py \
  --compiler /private/frozen-sysroot/bin/rustc \
  --cargo /private/frozen-cargo/cargo \
  --justrust /private/frozen-client/justrust \
  --output "$HOME/.justrust/rj-new" -n 3 --no-upload
```

Use a short new output path because Linux Unix-socket addresses are bounded.
Run `python3 -m unittest discover -s bench -p test_stable_resident_cargo.py`
first. Omit `--justrust` for the native Cargo-only control.

The client mode sets `JUSTRUST_TOOLCHAIN=system` plus absolute explicit `RUSTC`
and `JUSTRUST_REAL_CARGO` candidates. This does not fabricate an installed
owned toolchain marker or bypass complete-toolchain installation verification.
It is scoped to the library-test workload above, not a claim that these
component candidates provide a complete rustdoc/clippy/rustfmt installation.
`justrust cargo` records in place on the guest; this is not a promoted default
hosted routing policy. Guest receipts can be uploaded through the signed-in
local account with `justrust records put bench ID RECEIPT.json`.

The explicit experiment omits unused per-process shim run destinations at the
Cargo boundary in **both** arms. It does not secretly strip observable inputs
inside an already selected worker. The ordinary recording path is unchanged.
Shim per-invocation phase telemetry is unavailable in this experiment and must
be reported as unavailable, not zero work.

## Focused actual-client timing gate, 2026-11-17

The new `bench/suite.py stable-native-cargo` adapter measures the actual explicit
`justrust cargo test` invocation above, not a standalone compiler RPC. Component
SHA-256 attestations are checked before executing the candidates. The adapter
preserves the existing correctness helper and all its stage, nonce, worker,
error-recovery and wrapper-fallback checks. It counterbalances off/on order,
uses independent target directories, records effective credential-filtered
invocation environments and hashes all transport helpers. Root suite dispatch
rejects mixed workloads, changed component candidates and synthetic-gate claims.
The integrated Python regression selection passed 45 tests.

Two matched four-repetition captures ran on the authenticated guest with CPU
affinity `[3]` and Linux boot ID `19c7b35b-caff-4dec-9124-7d476ae182cd`.
The guest was checked for competing rustc/Cargo/Python processes before the
initial diagnostic. No overlapping guest workload was started by this lane.
Each successful stage executes six named tests: five shipping-module tests and
the nonce probe already defined by the correctness helper. Timings include the real
client, Cargo, compilation and test execution. They exclude the separate
wrapper-fallback and initial/cached stages from the reported edit-stage medians.

| Stage | Replica A off / on, ms | Replica B off / on, ms |
| --- | --- | --- |
| Body edit | 511.921 / 503.755 | 510.417 / 499.753 |
| Recovery after error | 480.786 / 470.454 | 477.266 / 465.314 |
| Forced same-source build | 476.866 / 469.394 | 476.247 / 464.606 |

These are modest scoped observations: the enabled medians are approximately
1.6–2.5% below their disabled counterparts. They do not establish a general
compile-time speedup, statistical significance, or a tenfold improvement.
The strict `bench/suite.py compare` accepted identical candidate, workload,
helper, effective-environment, boot and affinity provenance. Enabled stage
medians differed by about 0.8–1.1% between replicas, below the 10% comparison
floor. This floor reports drift, not proof of benefit or a promotion threshold.

Original account-private records:

```text
justrust records show bench stable-resident-cargo-suite-20261117-0428-diagnostic
justrust records show bench stable-resident-cargo-suite-20261117-0428-a
justrust records show bench stable-resident-cargo-suite-20261117-0428-b
```

The raw source snapshots, trace streams and receipts were transferred without
rewriting the generated originals. Authenticated readback verified exact JSON
equality, including floats, for all three records, with original bytes unchanged.
Build artifacts remain on the guest rather
than being included in the local evidence transfer. This remains the narrow
std-only library fixture, not the dependency-heavy client crate or the complete
session-weighted synthetic gate. No installed client or default was changed.

## Dependency-bearing fallback acceptance, 2026-11-17

A separate real `justrust cargo test` acceptance now covers an offline local
path dependency while keeping external-crate resident admission rejected.
`bench/stable_resident_cargo_dependency.py` runs ten stages in each of the
ordinary-off and enabled arms. It exercises a root-only edit and compiler error,
recovery, dependency signature, exported constant and runtime-body changes,
a dependency compiler error and recovery, and a final root-only edit. Dependency
source paths remain unchanged throughout the edits.

The actual frozen guest run passed all 20 observations. Runtime nonce/value
checks and full diagnostic JSON matched the ordinary-process oracle. Every
root-only stage required Cargo to report the dependency fresh and required zero
resident completion events, proving those dependent-root builds stayed on the
ordinary fallback path. A std-only dependency may itself use the worker when
rebuilt. Such an event is not evidence of retained external typed state or
admission of the dependent root. Artifact hashes are recorded as observations,
not treated as a byte-equivalence oracle.

The frozen helper and original Cargo helper regression selection passed 43
Python tests before native execution. The native run used the same immutable
compiler, Cargo and explicit client candidates listed above, with SHA
attestations checked before candidate execution. No compiler gate was relaxed.
Original source snapshots, complete command streams, artifact observations and
trace events are preserved in the private record:

```text
justrust records show bench stable-resident-cargo-dependency-20261117-0439
```

This is fallback correctness evidence only. It is not dependency-bearing
resident acceleration, unrestricted external metadata compatibility, or a
performance result.

## Performance status and next gate

The focused gate above adds actual-client timing evidence, but no tenfold,
whole-project, general compile-time or production-default speedup has been
established. `rj0335` remains correctness/integration evidence rather than a
performance comparison. Concurrent hosted correctness activity also makes that
older record unsuitable for a performance claim. No default was promoted.

The next material boundary is dependency-bearing crates with exact external
artifact identity and invalidation, followed by broader frontend retention.
Admission must not simply remove the `--extern` rejection: dependency artifacts
can change at the same path. Proc-macro/build-script semantics and wrapper
fallback remain part of the external correctness contract. Only after relevant
real-project acceptance and matched before/after comprehensive suite results
can this support a default promotion or a whole-project speed claim.
