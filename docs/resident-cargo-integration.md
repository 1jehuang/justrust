# Native resident Cargo integration, 2026-11-17

## Own-source macro transport and measurement contract

The dependency-bearing justrust library now passes real hosted edit loops through
native resident Cargo workers, including fresh per-request proc-macro helper
processes. This remains explicit experimental routing, not the default toolchain.
The published client is alpha.37. Retaining the process does not yet retain
arbitrary live typed queries across these macro-bearing revisions.

`JUSTRUST_CARGO_RESIDENT_MACRO_SHM=1` selects authenticated shared-memory helper
transport. The original Unix socket remains alive and authenticates the peer
before the inherited sealed memfd is attached. Fixed-size rings use bounded
futex waits, permanent role claims, and terminal failure without replay.
`JUSTRUST_CARGO_RESIDENT_MACRO_SPINS` accepts only canonical decimal `0..=256`.
It defaults to zero and a nonzero value requires shared-memory transport.
The explicit spin budget does not change the request timeout or admission rules.

The real-source benchmark is `bench/stable_resident_own_loop.py`. It changes a
function expression in this repository, checks the root RPC and stable worker
identity, retains raw output, and restores the exact source in `finally`.
Diagnostic-only and `-Ztime-passes` captures cannot be performance evidence.
`bench/suite.py compare` accepts these original receipts and requires all four
exact compiler/driver hashes when comparing different native candidates. Host,
workload, client, Cargo identity, harness and unrelated settings stay matched.

The exact `stable-resident-own-loop-spin0-20261117-1923` and
`stable-resident-own-loop-spin256-20261117-1923` originals show resident body-edit
medians of 8.20 to 4.96 seconds and 8.39 to 5.37 seconds, with residency-off near
4.2 seconds. This is a focused reduction in resident overhead, not a 10× result,
a general build speedup, or grounds for default promotion. A later cacheline
padding/local-copy experiment did not consistently improve the matched loop and
was withdrawn. Its original candidate and matched control are archived as
`stable-resident-own-loop-local-layout-20261117-1943` and
`stable-resident-own-loop-matched-baseline-20261117-1947`.

Native request telemetry now includes a bounded 256-counter callback wire-tag
histogram, reset per request and serialized sparsely without retaining payloads.
The actual-source diagnostic `stable-resident-own-loop-wire-tags-20261117-1953`
identified token concatenation, cloning and dropping as the dominant round trips.
Wire tags are interpreted against that exact compiler's bridge API, not assumed
to be an independently stable protocol. This telemetry does not affect cache
identity, admission, or macro execution.

## Production-hosted compiler-owner followthrough, 13:00 UTC

With `JUSTRUST_CARGO_RESIDENT_OWNED=1` and external admission enabled, the actual
Cargo worker now holds native SessionGlobals, the source map and compiler arena
owners across requests. Each revision still creates a fresh compiler context.
Live typed query reuse and managed decoder retention remain disabled. This is
an explicit experiment, not the default published toolchain.

The complete 20-stage dependency acceptance passed through published alpha.36
on the production hosted machine, including exact relative diagnostic paths,
root edits/error recovery on the same worker, increasing native revision/context
counts, and dependency identity invalidation. Original evidence:
`justrust records show bench stable-resident-owned-paths-20261117-1300`.
The exact driver is
`9e42043aeb23abaa57678b3eaf40d4c0dcdd2a065b8b450f828b87842a5a1d6b`.
Hosted gates also passed 71 source-map tests, 23 interface tests and 21 driver
tests. A real diagnostic mismatch found by acceptance was fixed in the native
source map, not hidden by normalizing the oracle. In that correctness run the
root worker reported RSS of 151,265,280 bytes after its first revision and
261,533,696 bytes after its fourth. These are per-worker post-compile RSS
observations, not retained-heap size, peak memory, or aggregate fleet memory.
The control owner retains fresh revision contexts despite having no live typed
cache entries yet. Resource growth is therefore an optimization requirement,
not evidence that retained ownership alone provides useful query reuse.

Build the matching stage1 compiler and standard library before interface tests
that load external metadata. Testing a new compiler identity against the prior
sysroot produces legitimate E0514 failures and wastes an iteration. Synthetic
discovery now supports `JUSTRUST_SYNTH_PROBE_REMOTE=1` so the comprehensive
hosted benchmark does not silently perform its discovery compilations locally.
No representative speedup or 10× result is established by these correctness
checks. Unrestricted typed reuse, resource measurement and matched broad timing
remain open.

## Production-hosted external-input followthrough, 12:35 UTC

The std-only boundary described below is the earlier baseline. Explicitly setting
`JUSTRUST_CARGO_RESIDENT_EXTERNAL=1` now admits bounded Rust dependency closures
in the native Cargo worker. Both Cargo and the compiler capture actual BLAKE3
file contents, including matching `.rlib`/`.rmeta` pairs and dependency search
directory membership. The compiler independently recaptures the invocation's
inputs before accepting it and verifies them again after execution. Dependency
content changes rotate the worker identity. Unsupported dynamic/proc-macro and
native-library closures still fall back. The feature remains opt-in.

The full 20-stage dependency acceptance passed through the published alpha.36
client on the authenticated production build machine. Root-only edits, compiler
errors and recovery kept the same root worker PID and key. Dependency signature,
constant and body changes changed the key and produced the expected new runtime
values. A failed dependency did not execute a stale root test. Original evidence:
`justrust records show bench stable-resident-external-20261117-1234`.

All compiler builds, Cargo builds and execution in this followthrough ran
directly on that hosted machine, retaining the native build cache there. There
was no local native-build/transfer cycle or separate canary. The first native
bootstrap took 185.1 seconds, subsequent native rebuilds approximately 42–45
seconds, and a warm Cargo-only rebuild 6.1 seconds. These describe iteration
costs, not an application speedup. Hosted regression gates passed 17 driver
tests, 13 Cargo resident tests, 5 Cargo source-identity tests, 9 bootstrap identity
tests and 8 shared manifest tests.

The successful pair has compiler driver SHA-256
`aecf67115b17f5c8b7d9d5030176245bb3ae6ef9f7245bbdf9b61be3045ee545`
and Cargo SHA-256
`711d32d0644faf2e1164c7a21c55f3ebf0f5d2b71eaa70f127ba6d604410b4a8`.
Neither replaced the default toolchain. This proves dependency-bearing native
worker reuse, not retained external typed queries, a representative justrust
repository speedup, or the requested 10× improvement. A matched comprehensive
performance comparison and broader admission remain unfinished.

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

## Idle external-owner retirement (2026-11-17)

The real hosted dependency fixture exposed obsolete worker generations retaining
about 1.06 GB after dependency changes. `JUSTRUST_CARGO_RESIDENT_EVICT=1`
now installs nonblocking inotify hints for exact external files and relevant
`.rlib`/`.rmeta` directory membership. Changed or lost watches retire idle
workers, without signaling unrelated processes or interrupting active builds.
Full content-manifest verification before request commit and after compilation
remains authoritative. Watch installation failure retains the existing bounded
idle lifetime and content checks. The option remains off by default.

Matching native compiler/Cargo bytes and the exact published alpha.36 client
passed 37 hosted Python tests, native check/build, 28 driver tests, and both
counterbalanced n=2 dependency suites. At the final fixture stage, aggregate
post-invocation RSS of all known live worker generations was
1,060,716,544 / 1,060,687,872 bytes without eviction and
361,451,520 / 361,582,592 bytes with eviction. Confirmed exited processes count
zero, unreadable processes remain unknown. This is fixture RSS, not peak memory
or retained heap. Timings are retained but no eviction speedup is claimed.

Exact originals: `bench/stable-resident-watch-20261117-1346-off` and
`bench/stable-resident-watch-20261117-1346-on`. Driver SHA256
`3c96114707c2212c6f445c932d643f2949f9b8254eb6026d6c060152848d84d5`,
Cargo SHA256
`4d8fcac842a3d47322903ef9df703cebde9b05d720bf682194689bbe17c27f1e`.
The comprehensive published baseline stopped at point 8 because remote CPU
samples were too short for its provenance gate. That result is invalid for a
general speed comparison, and the tenfold target remains unmet.

## Managed external decoder Cargo experiment (2026-11-17)

`JUSTRUST_CARGO_RESIDENT_DECODERS=1`, together with the explicit owned compiler
mode, connects the managed native decoder scope to actual Cargo requests. It
never enables live typed queries. Decoder retirement is honored after flushing
the actual result, including failed revisions. The dependency oracle requires
increasing real hit counters on root-only edits, a retired owner after errors,
and a new process for recovery. All other content and scheduler gates remain.

The frozen hosted candidate passed native check/build, 28 driver tests,
38 Python tests and the actual-client n=2 dependency suite with strict decoder
hit assertions. Each root first missed 23 decoders, then reported 23 hits on
its edit and 46 cumulative hits on its error. Recovery used a new owner.
Exact original: `bench/stable-resident-cargo-decoders-20261117-1352`, driver
`abc47ade0885457b9ce81ce613b0bb610769c28901159f15037182395f5b4ce8`.

This is correctness and overhead attribution, not a speedup. First publication
cost approximately 40–49 ms, repeated selection approximately 15 ms, and final
live-worker RSS approximately 643 MB. This mode remains default-off and is not
recommended for ordinary compiles. Useful decoder hits alone do not justify
activation. Lower publication/selection cost and representative real-project
measurements are still required, as is actual useful typed-query reuse.

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
