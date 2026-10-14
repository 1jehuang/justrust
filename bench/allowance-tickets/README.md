# Frozen actual-module allowance comparison

Owned scope: only this new `bench/allowance-tickets/` directory. No live native
runtime/docs edits, bootstrap, commits, or pushes. Compiled ELF files are retained
locally for provenance and ignored by Git. Reproducible commands and hashes are
kept instead of committing binaries.

## Exact sources and dependencies

This standalone Cargo workspace includes the **real** source through `include!`:

* Baseline: immutable `archive/baseline-jobserver.rs`, SHA256
  `941b83d84a8c29b3f0d6267b12120787a765ee6f18055e7bf5666ddb23f8684b`.
* Candidate: immutable `archive/candidate-jobserver.rs`, SHA256
  `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7`.
* Its unmodified real sibling `archive/jobserver_ticketed.rs`, SHA256
  `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18`.

No synthetic semaphore or copied admission state machine is used. The candidate
archive was taken only after sloth's final source-freeze acknowledgment. The
baseline archive remains unchanged. Candidate strips to the same legacy source
when its four module/reexport prefix lines are excluded.

`jobserver_crate` aliases package `jobserver`, **pinned to `=0.1.34`**, matching the
owned native Cargo lockfile. `parking_lot = "0.12"` resolves to 0.12.5. Both builds
use the exact same local lockfile SHA256:
`cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236`.

Compiler: nightly-2026-10-08, rustc 1.101.0-nightly, commit
`1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, LLVM 23.1.3. Every sample records
compiler version/commit, ELF compiler stamp, and the checksum of the **local**
pinned compiler installation. The latter is not a separately verified checksum
of the hosted compiler file. Both artifacts have matching compiler/linker stamps.

Final compared artifact SHA256:

| artifact | SHA256 |
|---|---|
| baseline | `e7a7fd459d3275c2f6997f664396e45d2e2547471e816cd88fc7191f679daf40` |
| candidate, both capture modes | `4c46fb86ee8c79c4d3d0cd03b58df23336be25d5d2e7ce366624777ee06ab543` |

## Reproduce

Run from this directory, noninteractively:

```sh
cargo fmt --check
timeout 180 justrust check --remote --locked
timeout 180 justrust test --remote --locked -- --nocapture
timeout 180 justrust build --remote --release --locked
cp target/release/allowance-tickets-baseline archive/baseline-executable-0.1.34

timeout 180 justrust check --remote --locked --features candidate
timeout 180 justrust test --remote --locked --features candidate -- --nocapture
timeout 180 justrust build --remote --release --locked --features candidate
cp target/release/allowance-tickets-baseline archive/candidate-executable-0.1.34

timeout 180 justrust clippy --remote --locked --all-targets -- -D warnings
timeout 180 justrust clippy --remote --locked --all-targets --features candidate -- -D warnings
python3 compare.py --rounds 15 --operations 10000 --event-capacity 4096 --out results/comparison-new.json
```

Existing archived ELF files are read-only, so reproduction should first make only
those locally owned output files writable, or use fresh paths with `measure.py`.
The immutable source archives must not be made writable or replaced.

One-mode correctness or timing commands:

```sh
python3 measure.py --binary archive/baseline-executable-0.1.34 --variant baseline --correctness --operations 2000 --out results/baseline-check-new.json
python3 measure.py --binary archive/candidate-executable-0.1.34 --variant candidate-disabled --correctness --operations 2000 --out results/candidate-off-check-new.json
python3 measure.py --binary archive/candidate-executable-0.1.34 --variant candidate-enabled --samples 15 --operations 10000 --event-capacity 4096 --out results/candidate-on-new.json
```

The binary accepts `one|multi|immediate|surplus [operations]`. Each invocation is a
fresh process because the actual private global client uses `OnceLock`.
Launchers remove `CARGO_MAKEFLAGS`, `MAKEFLAGS`, and `MFLAGS`, so native
`initialize(limit)` creates a private **real** client rather than inheriting
Cargo's jobserver. Candidate calls the real `TicketedProxy::new_with_event_capacity`
with `ALLOWANCE_CAPTURE_CAPACITY=0` or `4096`. Direct invocation must set this
variable for candidate and clear all three make/jobserver variables.

Capture disabled means **event storage only** is disabled. Inventories, operation
IDs, and ticket protocol remain active. Enabled means bounded preallocated 4096
transition events, with explicit `lost_events` after saturation. Neither setting
is an uninstrumented candidate.

## Workload and correctness

* `one`: real capacity 1, eight threads, 10,000 acquire/work/release pairs each.
* `multi`: real capacity 4, same eight threads and 80,000 completed pairs.
* `immediate`: real capacity 1, retained idle entitlement, 10,000 pairs.
* `surplus`: real capacity 2, one external token exhausts availability. A worker
  registers while main uses the implicit entitlement. Main releases locally to
  the registered worker, waits for a channel acknowledgment, then tells the worker
  to release and joins it. Only then is the external token returned, making the
  outstanding helper request surplus. A further retained-entitlement pair runs.

`Proxy::new()` and candidate construction start with main's `used=1`. Contention
and immediate scenarios explicitly release main before beginning. Surplus releases
main exactly once into the waiter, before join. No initial permanently retained
real token is released through `release_raw` by the fixture.

Eight-worker starts use a barrier. Transfer uses channels and a bounded read-only
pending-count setup probe. External atomic active-work counts are the capacity
oracle, not queue/dequeue order, private `used`, identity, or event traces. Every
active count must be at most the real capacity, and every expected operation must
complete. Final candidate inventories additionally assert:

`requests == grants == acks == completed_pairs`,
`releases == completed_pairs + 1`, `used == pending == helper_errors == 0`,
`overflow == false`.

The extra release is main's initial entitlement. Disabled samples also retain
zero stored/lost events. All 15 surplus samples per candidate mode show exactly
one supply and one **completed** callback. The real candidate callback counter
increments after consuming or dropping the acquired token. Enabled surplus
samples retain the small actual transition trace, including `HelperSupply` with
`granted: false`. This is grant-decision evidence, not holder/token ownership.

The immutable baseline does **not** expose callback completion acknowledgment.
Its surplus setup is exercised but an availability snapshot can precede the
helper's acquisition/return, so baseline surplus branch completion is explicitly
unverified. No sleeps or dequeue oracle are used to disguise that limitation.

For timed candidate contention, a final snapshot may catch one helper callback
between supply accounting and completion, so the stored supplies and completed
callbacks can differ by one. We preserve both. This does not certify helper
quiescence, token provenance, physical token identity, fairness, lease ownership,
or exhaustive race safety. The acknowledged surplus scenario provides the
narrow completion check.

The executable watchdog exits after 20 seconds. Test and measurement launchers
independently enforce 25-second subprocess deadlines. Watchdog sleeps/polling
are not scenario synchronization. Compare subprocesses also have finite deadlines.

## Durable measured results

`results/comparison-034.json` contains all **180** successful samples: 15 rounds,
three variants, four scenarios. `results/comparison-034-individual/` preserves each
of the 45 independently collected raw batches with hashes and complete outputs.
Variant order rotates each round. Each variant has 15 samples of each scenario.
No samples were discarded, and there were no failures or timeouts.

The runner verifies identical compiler stamps, lockfile, workload harness, runner,
and machine across variants and unchanged provenance throughout the run. Candidate
capture-enabled and disabled use the same executable hash. Each timed sample
retains duration, completed count, throughput, process duration, external peak,
and conservation inventories where available. Long timed event traces are not
printed, but stored/lost counts are preserved. The small surplus trace is retained.

Local XPS14, same compiler/dependencies/harness, 80,000 pairs for contention and
10,000 for immediate:

| variant | scenario | min ms | median ms | max ms | median pairs/s |
|---|---|---:|---:|---:|---:|
| baseline | one | 242.232 | 286.793 | 375.194 | 278,947 |
| candidate, events disabled | one | 247.894 | 314.302 | 402.855 | 254,532 |
| candidate, events enabled | one | 258.623 | 311.409 | 2034.054 | 256,897 |
| baseline | multi | 122.922 | 165.949 | 219.192 | 482,076 |
| candidate, events disabled | multi | 149.335 | 201.659 | 239.339 | 396,710 |
| candidate, events enabled | multi | 11.937 | 204.504 | 368.138 | 391,191 |
| baseline | immediate | 0.244 | 0.277 | 0.607 | 36,135,392 |
| candidate, events disabled | immediate | 1.115 | 1.244 | 2.052 | 8,039,044 |
| candidate, events enabled | immediate | 1.174 | 1.308 | 2.717 | 7,642,911 |

Observed multi peaks: baseline 3 or 4, candidate disabled 2 or 3, candidate enabled
1, 2, or 3. One-token peaks were always 1. These are observed active-work bounds,
not proof that capacity is always saturated. All contention samples completed
exactly 80,000 pairs, and immediate samples completed exactly 10,000.

Timers exclude startup/helper creation and final diagnostic snapshots. Contention
includes barrier release and joins; immediate measures the loop only. Atomic
capacity/completion instrumentation is shared between variants. Process elapsed
is separately retained. Tiny work sections and OS scheduling create substantial
variation, including the enabled outliers shown above. The enabled bounded trace
saturates, so it cannot be used as a complete causal inventory for long runs.

All candidate medians are larger than the matched baseline in this focused run.
This is not a broad speedup, architecture-optimality, default-promotion, full
compile-loop, or stock-versus-justrust claim. No comprehensive session-weighted
workload gate was run because this task owns only the focused allowance fixture.

## Hosted validation receipts

| operation | run ID |
|---|---|
| pinned baseline check | `20261014-131408699-1072502` |
| pinned baseline test | `20261014-131410816-1072848` |
| final baseline release | `20261014-131508079-1082784` |
| candidate check | `20261014-131523267-1085919` |
| candidate test | `20261014-131525176-1086123` |
| candidate release | `20261014-131527584-1086391` |
| baseline clippy | `20261014-131739693-1114884` |
| candidate clippy | `20261014-131741688-1115587` |

All commands succeeded on hosted, zero warnings. Candidate test ran the 11
unmodified actual-source protocol tests plus this fixture's integration workflow
in both capture modes, total 12 passed. Expected caught overflow/underflow panic
messages from boundary tests remain in the full log, not hidden as errors.
Baseline integration scenarios passed. `cargo fmt --check` passed. Native
compiler bootstrap/integration acceptance is separately coordinator-owned and
is not claimed from this focused fixture.

## Superseded initial 0.1.35 evidence

The initial caret requirement matching native manifest syntax resolved jobserver
**0.1.35**, not the owned native lockfile's 0.1.34. That mismatch was identified
before any candidate comparison. Initial evidence is retained but **superseded**:

* `results/baseline-local.json`, 46 original samples, medians 384.083 ms one,
  172.389 ms multi, 0.284 ms immediate.
* `results/baseline-correctness.json` and initial hosted logs.
* `archive/superseded-0.1.35/` stores exact original manifest, lockfile, harness,
  measurement script, and original README.
* Initial lockfile SHA256
  `4319829e6b53e9157d9d928104a23f87c628c38d330fe5cb52be3155f33b88bc`.
* Initial ELF, locally retained/ignored, SHA256
  `d78a7ca865c7db799c93dd8778a8905dd398d12aa6e92d440d1fc4851345692e`.

Those original records were not rewritten as 0.1.34 results. Their archived
harness/lockfiles provide the original hash provenance. Immediate timing in the
old fixture included final assertions, whereas the matched fixture excludes all
final diagnostic snapshots equally. Only `comparison-034.json` is used for the
matched candidate comparison. The frozen baseline jobserver source is unchanged
throughout both generations.
