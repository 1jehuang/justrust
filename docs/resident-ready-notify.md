# Resident readiness notification v65: rejected experiment

**Historical evidence only. The candidate was rejected on October 31, 2026 and `cargo/daemon.rs` restored exactly to accepted HEAD. No default promotion, speedup, causal regression or production acceptance is claimed.**

The portable archive is `bench/results/resident-ready-notify-20261031/`. It contains full original/candidate source, exact patch, historical documentation and FROZEN release, distinct failed analysis attempts and corrected results, allowlisted selected native observations, original private-file hashes, and offline semantic/mutation/relocation checks. Full private originals remain unchanged. This document is outside the portable package's checksum inventory.

## Candidate design, now removed from live source

The experiment used exact `RA_JIT_READY_NOTIFY=1`, default OFF, latched once per daemon process at startup before serving threads. No client/worker request or serialized configuration format changed. Each worker owned a Condvar with its readiness predicate under the existing checkouts mutex. Publication matched the checkout key and generation, updated Warmth while locked, released the lock, then notified all waiters.

The actual accepted `Daemon::built` loop polls readiness every 20 ms. The candidate retained the original OFF `sync`/`loads`/sleep path. ON used `wait_timeout_while` for that same exact key/generation Loading predicate, covering ready-before-wait, concurrent publication and spurious wakes without spinning. The worker map was not held while waiting. An old worker cannot be notified by its replacement, and in-place generation insertion is not readiness publication. Therefore the existing `POLL` bound remained, after which normal sync/load/replacement/error handling ran again. Scheduler and mutex delays were not wall-clock guarantees.

The fixed line `ra-jit ready-notify: observed loading transition` was recorded at most once per process only when a wait entered Loading and returned not timed out with the same key/generation terminal. It was not a flag-activation marker. Ready-before-wait, wrong identities, replacement generations and loading timeouts did not qualify. The once-only atomic was consulted on the successful transition path only. Stderr writes used ignored-error `writeln!` on locked stderr. A marker proves an exercised transition, not the causal wake source, a useful speedup or universal fallback correctness.

## Root-owned actual results

Root check/build and native gates passed: **53 codegen tests, 368 resident tests**, including four real production-helper fixtures for ready-before-wait/terminal errors, notify-during-wait, wrong key/generation/loading timeout, and generation transition during wait. Channel and mutex coordination avoided elapsed-time performance assertions. Four fixture pass lines are retained from the actual output journal, not just inferred from source. Source checks covered 3,757 files and the root lock.

Four sequential same-binary OFF/ON/ON/OFF cases passed all **28 strict seven-state rows**. Each successful native invocation required the exact four named tests and full counters, a fresh runtime nonce and correct exit status. Type-error and assertion-failure states required their exact fresh diagnostics and status 101. Both ON daemon logs showed cumulative marker snapshots `[1,1,1,1,1,1,1]`, both OFF logs all zeros. These are cumulative observations of one event per daemon, not seven distinct transitions. A distinct accepted v58 baseline passed seven rows and is not part of the same-binary pairs. Root's final Python benchmark regression suite passed 733 tests, a correctness check rather than a performance benchmark.

| Descriptive ON versus OFF change | First pair | Reverse-order pair |
|---|---:|---:|
| cold prepare | +18.924% | +19.901% |
| manifest feature add | +25.320% | -9.543% |
| type recovery | +15.642% | -14.916% |

Noise remained **unknown**, execution was sequential and uncontrolled, and no useful replicated win was found. The candidate was rejected. These values do not establish a causal regression. No comprehensive session-weighted performance gate was run for promotion, and no general performance-goal completion is claimed.

Two root analysis attempts failed because JSON deserialization produces list test entries whereas the raw parser produces tuples. v65b normalized one equality check but still passed JSON list entries to the strict oracle. v65c verified JSON-normalized exact raw equality, then supplied the fresh raw-parser tuples to the unchanged oracle. Native output, test counts and the oracle were unchanged. Both failed scripts and actual empty JSON outputs remain distinct from corrected v65c. Historical stderr was not retained or fabricated through reruns.

## Restoration, privacy and offline replay

The live candidate daemon/doc were preserved privately before restoring only daemon source from its exact original backup. Worker byte comparison and worker/coordinator independent scoped HEAD diffs passed. Restored daemon SHA-256 is `d488bffe1498847e8e234e6513474c26b38b19939b94915feacaa60decbe0b5c`. Source ownership was immediately released to root. Subsequent work touched only this doc and the evidence archive, with no native jobs, install or commit by this worker.

Export uses an explicit eight-name/value environment allowlist and five fixed startup controls. Ambient/raw state environments, private homes/caches, toolchain path listings, binaries, credentials and telemetry are excluded. Full original hashes and byte counts allow custodian verification without publishing excluded bytes. Cleanup receipts describe successful **post-validation operational stops** only, with stores preserved, not original execution timing attestations.

From any directory, with Python stdlib only:

```sh
python3 -I -B /path/to/resident-ready-notify-20261031/validate.py --manifest
python3 -B /path/to/resident-ready-notify-20261031/test_offline.py
```

Offline validation checks exact selected runtime semantics, source/patch replay, native fixture witnesses, failed-analysis history, nonclaims, cleanup and package integrity. Mutation and relocation controls run only Python, not Cargo/rustc/justrust/resident tools. A fixed historical observation commitment rejects coherent history changes even after the mutable outer inventory is rehashed. These are repository integrity checks, not third-party attestation or fresh native validation.
