# Actual native Proxy baseline

This standalone workspace includes the immutable archived **real**
`toolchain/rustc/compiler/rustc_data_structures/src/jobserver.rs` through
`include!`. It is not a copied or modeled semaphore. Only this new directory
was edited. No native runtime/docs edits, bootstrap, commits, or pushes.

Frozen baseline source SHA256:
`941b83d84a8c29b3f0d6267b12120787a765ee6f18055e7bf5666ddb23f8684b`

Dependency requirements match the native manifest:
`jobserver_crate = { version = "0.1.28", package = "jobserver" }`,
`parking_lot = "0.12"`. `Cargo.lock` pins the resolution. The archived executable
was compiled remotely with the lockfile and is retained read-only locally.

## Commands

Run from `bench/allowance-tickets`:

```sh
cargo fmt --check
timeout 180 justrust check --remote --locked
timeout 180 justrust test --remote --locked -- --nocapture
timeout 180 justrust build --remote --release --locked
python3 measure.py --binary archive/baseline-executable --correctness --operations 2000 --out results/baseline-correctness-new.json
python3 measure.py --binary archive/baseline-executable --samples 15 --operations 10000 --out results/baseline-local-new.json
```

The executable also accepts `one|multi|immediate|surplus [operations]`.
Each invocation is a fresh process because the actual private global client
uses `OnceLock`. Test and measurement launchers clear all three make/jobserver
environment variables, ensuring `initialize(limit)` creates the real private
client rather than inheriting the build jobserver. Direct invocations must also
clear `CARGO_MAKEFLAGS`, `MAKEFLAGS`, and `MFLAGS`.

The executable watchdog exits after 20 seconds. Integration tests and the
Python launcher independently enforce 25-second subprocess deadlines. Sleeps
are used only for watchdog/polling, never to synchronize work or decide grants.
Barriers start the eight workers. Channels control local entitlement transfer.
A bounded read-only private pending-count probe establishes that a waiter
registered before transfer. External active-work counters, not pending,
`used`, dequeue order, or token identity, are the capacity oracle.

## Cases and contract

* `one`: real capacity 1, eight threads, 10,000 acquire/work/release operations
  each in the recorded timing run. Every external active-work count must be
  at most 1; all 80,000 operations must complete.
* `multi`: real capacity 4, same eight threads and 80,000 operations. Every
  external active-work count must be at most 4. Observed peaks were 3 or 4.
* `immediate`: real capacity 1, retained entitlement, 10,000 immediate
  acquire/release pairs. The last real token remains retained.
* `surplus`: real capacity 2. An external acquired token exhausts available
  capacity while a waiter registers. Main releases its initial entitlement
  locally, the worker reports grant through a channel, and only after the
  worker releases and joins is the external token returned. This makes the
  outstanding helper request surplus. A subsequent retained-entitlement pair
  completes, and a real-client availability snapshot is checked.

`Proxy::new()` assumes `used=1` for main. Contention and immediate scenarios
explicitly release main before workers/loops begin. The transfer scenario
releases main exactly once into the registered waiter, before join. No scenario
calls `release_raw` for the initial permanently retained token.

**Coverage blocker:** the immutable baseline exposes no helper callback
completion acknowledgment. Availability can become 1 before the helper consumes
and returns that token, so the surplus case cannot deterministically certify
that the surplus callback branch ran before process exit. It exercises the
real transfer/request setup, but does not claim callback completion, exact grant
binding, fairness, resource identity, lease correctness, or exhaustive race proof.
The included source is not instrumented or modified to pretend otherwise.
Candidate protocol diagnostics must provide a callback acknowledgment if this
branch is to be deterministically verified without a dequeue oracle.

## Recorded evidence

Locked hosted commands succeeded with zero warnings:

| operation | run ID |
|---|---|
| check | `20261014-130322393-958982` |
| test | `20261014-130324140-959364` |
| release compile | `20261014-130325997-959467` |

Logs are retained under `results/`. The hosted test executed all four actual
source scenarios with 16,000 operations in each contention case, observed peaks
1 and 3, and all assertions passed. `baseline-correctness.json` independently
records successful local correctness-mode invocations.

`results/baseline-local.json` retains **every** individual timing, stderr,
completion count, external peak, artifact/source/harness/lock hashes, compiler
version/commit, ELF compiler stamp, and machine identity. The compiler binary
checksum is of the local pinned compiler installation with matching commit and
version stamp, not a separately verified checksum of the remote compiler file.

Archived executable SHA256:
`d78a7ca865c7db799c93dd8778a8905dd398d12aa6e92d440d1fc4851345692e`

Compiler: nightly-2026-10-08, rustc 1.101.0-nightly,
commit `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, LLVM 23.1.3.

Local XPS14 recorded timing, 15 individual samples per timed scenario:

| case | completed pairs per sample | min | median | max |
|---|---:|---:|---:|---:|
| one, eight threads | 80,000 | 236.887 ms | 384.083 ms | 997.318 ms |
| multi, eight threads | 80,000 | 132.588 ms | 172.389 ms | 190.371 ms |
| immediate | 10,000 | 0.284 ms | 0.284 ms | 0.291 ms |

No failed samples. These measure this bounded microfixture, including the active
counter and completion counter overhead. Contention timing starts just before
main releases the start barrier and ends after joins. Immediate timing includes
final assertions. Process startup/helper creation are excluded from those timers
but separately retained as `process_elapsed_ns`. Scheduling noise is substantial,
especially in the one-token case. These are baseline measurements, not a speedup
claim, broad workload gate, optimality claim, or stock-versus-justrust comparison.

## Candidate follow-up

The candidate is not yet archived here. Keep this baseline archive immutable.
Once the coordinator supplies the exact real native protocol and its sibling
source files, archive those separately, include the real module with matching
paths, and compare independently hashed artifacts under the same workload and
execution mode. Do not replace this baseline with a synthetic state machine or
change live native sources from this fixture task.
