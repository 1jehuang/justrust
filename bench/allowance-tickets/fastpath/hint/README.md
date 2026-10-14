# Advisory idle hint: independent three-way production fixture

This new `fastpath/hint/` workspace owns only its own files. Parent archives,
720-sample matrix, path-failure evidence, and README remain frozen and unchanged.
No native source edits, native bootstrap, commits, or pushes were done here.

## Actual frozen inputs

All variants `include!` the real `jobserver.rs` and its unmodified sibling.
The wrapper source SHA256 is identical for all three:
`5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7`.

| stage | sibling SHA256 | description |
|---|---|---|
| original | `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18` | original ticketed allocation path |
| fast | `1233aa16d73ae11cf447ab5bac0abd2adf48efcb2873574fb08e4e9428f7e631` | immediate slot-free path with initial State lock |
| hint | `f3849e80b7a1f72ee9472592d66a57a83477900d52354a772ac345c77f07def8` | advisory AtomicBool selector for the fast path |

The hint's false relaxed load takes the original allocation/register path without
an initial State lock. True only selects a State-protected recheck and cannot
authorize admission. Stores occur under State when aggregate use crosses zero.
The tests include stale-true, stale-false, and release/acquire races. This fixture
is not a synthetic semaphore or a copied independent admission implementation.

Dependencies are unchanged: jobserver `=0.1.34`, parking_lot 0.12.5, complete lock
SHA256 `cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236`.
Compiler nightly-2026-10-08, rustc 1.101.0-nightly commit
`1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, LLVM 23.1.3.

The hot-loop body from `fn wait_pending` through the end of `src/main.rs` is
**byte-identical** to the frozen parent fastpath fixture, SHA256
`07e03eaa16a78f2740dca511a75d323f56f85f8ad8acbef5f5688afcee09ae6d`.
Only include selection and outside-timer variant labels differ. Shared complete
harness SHA256 is
`de91c937a7b92b24817d5dce6067768be518b6b1514aaa99df150b862e29f21d`.

Production release MAIN artifact hashes, both capture modes using the same ELF:

| stage | SHA256 |
|---|---|
| original | `b58fece6e57a25dac3140b54e32285aef7a883882bec6c547f2a23cacd0b1283` |
| fast | `954d15f8ba65e22a03d4b157b00125819e12bbddd6a275365f1d6fb27aa0cf89` |
| hint | `7eb9a2f97f516ef402f419d2eec067fd67db1078e0aa25f96a7227d5a5245286` |

All are Git-ignored, locally retained. Timings use `build --release` MAIN binaries
only, never `--test` binaries. The cfg(test) hook seam is not the production hot
path. Source tests and their console durations are correctness evidence only.
Every sample retains artifact/source/sibling/harness/lock/runner hashes, compiler
commit/ELF stamp and local pinned compiler file checksum. That checksum is not a
separately verified hosted-compiler file hash.

## Reproduce

Run from this directory, with fresh evidence paths:

```sh
cargo fmt --check
justrust check --remote --locked
justrust build --remote --release --locked
cp target/release/allowance-tickets-baseline archive/original/executable
justrust check --remote --locked --features fast
justrust build --remote --release --locked --features fast
cp target/release/allowance-tickets-baseline archive/fast/executable
justrust check --remote --locked --features hint
justrust build --remote --release --locked --features hint
cp target/release/allowance-tickets-baseline archive/hint/executable

justrust test --remote --locked -- --nocapture
justrust test --remote --locked --features fast -- --nocapture
justrust test --remote --locked --features hint -- --nocapture
justrust clippy --remote --locked --all-targets -- -D warnings
justrust clippy --remote --locked --all-targets --features fast -- -D warnings
justrust clippy --remote --locked --all-targets --features hint -- -D warnings
python3 compare.py --rounds 15 --operations 10000 --pending-rounds 200 --event-capacity 4096 --out results/threeway-new.json
```

Archived binaries are read-only, so reproduce into fresh paths or first make only
those owned binary outputs writable. Never change frozen input sources. The
comparator refuses to overwrite evidence files or raw batch directories. A
single-mode correctness workflow is available through `measure.py --correctness`.

## Shared workloads and conservation

* Idle: 10,000 immediate retained-entitlement pairs.
* Natural one/multi: capacities 1/4, eight workers, 10,000 pairs each, 80,000 total.
  A start barrier creates contention, but idle paths and scheduling variation
  remain possible and observed peak capacity need not saturate.
* Controlled pending one/multi: 200 channel/barrier-controlled episodes, eight
  pairs per episode, 1,600 total. After capacity-many externally acknowledged
  active grants, setup holds them until the other seven/four requests register
  Pending, then opens release gates. The supplementary timer includes setup,
  channel/barrier overhead and snapshot cloning, not pure lock cost.
* Surplus: real capacity two, externally exhausted extra token, local transfer
  to a registered waiter, joined release, then external return. Exactly one real
  helper surplus callback completes before a further idle pair.

Each process initializes the private global real Client with native
`initialize(limit)`. Launchers clear all three make/jobserver environment variables.
Main's initial `used=1` entitlement is explicitly released before workers and
joins. No fixture releases the permanent final real token.

External active counters, not dequeue/identity/private inventory, are the capacity
oracle. Every expected pair must complete. Both capture modes additionally assert
requests=grants=acks=completed pairs, releases=completed+1, used=pending=errors=0,
and no overflow. Disabled event storage has zero stored/lost events. Enabled has
bounded 4096 preallocated events and retained loss counts. Inventories and IDs
remain active when event storage is disabled.

Surplus callback completion is explicitly acknowledged after actual token drop or
consumption. Small enabled surplus traces are retained. Timed final snapshots may
observe a supply before its outside-lock callback completion, so both counters
are preserved without claiming helper quiescence. There are no holder leases,
physical token identity, fairness, exhaustive concurrency, or broad admission
mapping claims. Native frontend integration remains separately coordinator-owned.

Timeouts are finite: production executable watchdog 20 seconds, each child 25,
each batch runner 160. Watchdog sleeps are not a grant oracle. Timers exclude
startup and final snapshots; natural contention includes joins, idle is loop-only.
Process elapsed time is separately retained.

## Ordering, machine context, and evidence boundaries

The three-way extension of ABBA uses a mirrored six-stage block such as
original/fast/hint/hint/fast/original. First-half order rotates and reverses across
rounds. Capture-mode order reverses each round. Each stage appears twice per mode
per round, so 15 rounds yield 30 samples per scenario/stage/mode: 1,080 total
samples in 180 raw batches. Within-block median ratios for hint/original,
hint/fast, and fast/original remain diagnostics, not causal estimates.

No CPU pinning is applied; unchanged inherited affinity (currently CPUs 0-15)
is checked across all records. Per-sample load averages are retained. Quiet status
is unknown, and heterogeneous CPU placement, frequency, thermal state, scheduling,
and unrelated activity can vary. All outliers remain in the raw evidence.
Natural and forced-pending cases stay separate so regressions cannot be averaged
away by idle improvements. There is no comprehensive session workload gate,
legacy comparison, broad speed, optimality, or default-promotion claim.

## Preliminary evidence and coordinated timing hold

`results/preliminary-threeway.json`, SHA256
`098d370b82941ec7d39d399b400fb67a06302e6581cf7ed63b1fdb57df29685a`,
retains 72 successful samples in 12 raw mirrored batches, two per scenario/stage/
mode. It ran October 14, 2026 from 20:54:34 to 20:54:42 UTC, after coordinator
short native crate test `106914oxke` ended at 20:52:01 UTC. No performance
acceptance follows from this small-n run; all counts/capacity/conservation passed.

After preliminary completion, all local timing was explicitly held for the
coordinator's sole native compiler/acceptance task `326345s2gi`, started at
20:55:26 UTC and completed successfully at 20:58:53 UTC. After explicit clear,
full production task `705228qyre` ran from 21:01:45 to 21:03:37 UTC (112.76s),
with no coordinator native build overlapping. This does not establish machine
quiet. Hosted fixture correctness/lints during the hold were remote.

## Final focused measurements

`results/threeway-015.json`, SHA256
`91f8b1391cb31320908a43036467b731ceb3e3cc5792ef24048a4a15bc1c39b3`,
retains all 1,080 successful samples, 180 individual raw batches, 30 paired
blocks, artifact/compiler/source provenance, throughput, load and affinity.
There were zero failures. Every sample passed external capacity, exact completed
operation count, inventory conservation, disabled-event and overflow checks.
All 180 surplus workflows acknowledged the actual helper completion. All forced
pending episodes observed the required seven/four registered waiters and peak
active exactly one/four. All natural peaks were within real capacity.

Median elapsed milliseconds, 30 samples per cell. Pending setup includes
barrier/channel/snapshot overhead. These are focused observations, not a broad
speed claim or a default promotion decision.

| capture | case | original96d803 | fast1233 | hintf384 |
|---|---|---:|---:|---:|
| disabled | idle | 1.073451 | 0.273216 | 0.323714 |
| disabled | natural capacity1 | 274.841013 | 288.706223 | 273.728690 |
| disabled | natural capacity4 | 177.024627 | 190.520716 | 170.861729 |
| disabled | pending capacity1 | 20.951721 | 21.442925 | 21.711507 |
| disabled | pending capacity4 | 15.925952 | 16.283987 | 15.828140 |
| enabled | idle | 1.205256 | 0.359494 | 0.398261 |
| enabled | natural capacity1 | 274.421025 | 280.020776 | 280.244090 |
| enabled | natural capacity4 | 185.876300 | 204.724416 | 176.426170 |
| enabled | pending capacity1 | 37.331013 | 39.665145 | 38.414608 |
| enabled | pending capacity4 | 24.316450 | 23.561635 | 22.966841 |

Hint/original within-block median ratios and blocks slower than original:

| capture | case | median ratio | slower blocks /15 |
|---|---|---:|---:|
| disabled | idle | 0.310653 | 0 |
| disabled | natural capacity1 | 1.029387 | 8 |
| disabled | natural capacity4 | 0.996619 | 6 |
| disabled | pending capacity1 | 1.029458 | 8 |
| disabled | pending capacity4 | 0.924961 | 6 |
| enabled | idle | 0.336483 | 0 |
| enabled | natural capacity1 | 1.038799 | 9 |
| enabled | natural capacity4 | 1.018570 | 8 |
| enabled | pending capacity1 | 0.997236 | 6 |
| enabled | pending capacity4 | 0.978705 | 7 |

Idle is consistently lower than original, but hint is slower than slot-free fast
in the idle aggregate medians. Natural capacity4 aggregate medians favor hint,
but paired ratios are approximately neutral, including enabled ratio above one.
Natural capacity1 and disabled pending capacity1 paired medians exceed one.
Thus this run does not establish a general contention benefit or eliminate all
regressions. Wide paired ranges and every outlier are retained, not trimmed.
The fixture is actual-module scoped, not native frontend integration or the
comprehensive session-weighted performance gate.

## Hosted fixture receipts

| stage | check | production release | correctness | clippy |
|---|---|---|---|---|
| original | `20261014-135203488-1494952` | `20261014-135205140-1495162` | `20261014-135742292-1562780` | `20261014-135751946-1564893` |
| fast | `20261014-135207564-1495488` | `20261014-135209543-1495949` | `20261014-135745317-1563214` | `20261014-135753580-1564976` |
| hint | `20261014-135212330-1496391` | `20261014-135214013-1496549` | `20261014-135748169-1564341` | `20261014-135757332-1565738` |

All passed remotely with zero warnings. Actual source tests plus the shared
integration workflow give 12 original, 16 fast, and 19 hint tests passed. The
integration test covers all six scenarios in both capture modes, including
2,000 controlled pending episodes per correctness child. Expected caught
boundary panic messages remain in full logs. No test-binary timing is used in
performance evidence. `cargo fmt --check` passed.
