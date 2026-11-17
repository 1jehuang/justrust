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

## Performance status and next gate

No tenfold, whole-project, general compile-time or production-default speedup
has been established. `rj0335` is correctness/integration evidence, not a pinned
`bench/suite.py` performance comparison. Concurrent hosted correctness activity
also makes it unsuitable for a performance claim. No default was promoted.

The next material boundary is dependency-bearing crates with exact external
artifact identity and invalidation, followed by broader frontend retention.
Admission must not simply remove the `--extern` rejection: dependency artifacts
can change at the same path. Proc-macro/build-script semantics and wrapper
fallback remain part of the external correctness contract. Only after relevant
real-project acceptance and matched before/after comprehensive suite results
can this support a default promotion or a whole-project speed claim.
