# Ticketed immediate-allocation fastpath: frozen focused fixture

Scope is ONLY this new `bench/allowance-tickets/fastpath/` directory. The committed
`comparison-034` evidence and source archives in the parent directory remain
untouched. No native source edits or native/compiler bootstrap builds were done
by this fixture owner. All builds below are isolated hosted fixture builds.

## Immutable actual sources

Both variants use `include!` of the real native module with its unmodified sibling:

| input | SHA256 |
|---|---|
| before `archive/before/jobserver.rs` | `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7` |
| before sibling | `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18` |
| after `archive/after/jobserver.rs` | `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7` |
| after sibling | `1233aa16d73ae11cf447ab5bac0abd2adf48efcb2873574fb08e4e9428f7e631` |

The before copy was frozen and acknowledged before sibling editing began. The
after copy was archived only after sloth's final source-freeze acknowledgment.
There is no copied/synthetic admission state machine. This compares **ticketed
before versus ticketed after**, not ticketed versus legacy.

The after source avoids slot allocation in the immediate idle path, doing the
Request/IdleImplicitGrant/Ack operation under one State critical section in
production. A non-idle request takes an extra initial State lock before the
existing allocation/register/recheck slow protocol. Idle and contended costs
must therefore be examined separately.

## Build and provenance

Jobserver is exactly `=0.1.34`, parking_lot 0.12.5, same complete Cargo lockfile:
`cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236`.

Compiler nightly-2026-10-08, rustc 1.101.0-nightly,
commit `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, LLVM 23.1.3.
The shared workload harness is SHA256
`eca652fed0bb108abd6d14d18ffaeb5253afccc61e233b78a4a0537f1414a927`.

Production release MAIN artifacts, locally retained but Git-ignored:

* before `archive/before/executable`:
  `86e76145438916c201a449afa189d67118bae506db9926265c4dbcd23a08e394`.
* after `archive/after/executable`:
  `b5fc3065499e8d523694b25a60020a9ab8dd350c1db8cccac1cc6c309fb915e1`.

**Performance never uses a `--test` executable.** The actual candidate has a
`cfg(test)` hook seam that deliberately unlocks/relocks State on the idle path.
That is not the shipping one-critical-section path. All measurement binaries were
built with `justrust build --remote --release --locked`, not `test --release`.
Native source tests are correctness gates only, not timing samples.

Every sample retains source/sibling, artifact, compiler version/ELF stamp,
lockfile/harness/runner hashes. The compiler file checksum is of the local pinned
installation with matching compiler stamp, not a separately verified hash of the
remote compiler file.

## Commands

Run from this directory:

```sh
cargo fmt --check
timeout 180 justrust check --remote --locked
timeout 180 justrust build --remote --release --locked
cp target/release/allowance-tickets-baseline archive/before/executable

timeout 180 justrust check --remote --locked --features after
timeout 180 justrust build --remote --release --locked --features after
cp target/release/allowance-tickets-baseline archive/after/executable

timeout 180 justrust test --remote --locked -- --nocapture
timeout 180 justrust test --remote --locked --features after -- --nocapture
timeout 180 justrust clippy --remote --locked --all-targets -- -D warnings
timeout 180 justrust clippy --remote --locked --all-targets --features after -- -D warnings
python3 compare.py --rounds 15 --operations 10000 --pending-rounds 200 --event-capacity 4096 --out results/abba-new.json
```

Existing archived binaries are read-only, so make only those owned binary outputs
writable before rebuilding or use fresh output paths. Never change the immutable
source archives. `compare.py` refuses to overwrite an existing evidence file or
raw-batch directory.

A single-mode correctness command:

```sh
python3 measure.py --binary archive/after/executable --variant after-disabled --correctness --operations 2000 --pending-rounds 200 --out results/after-check-new.json
```

Modes are `before-disabled`, `after-disabled`, `before-enabled`, `after-enabled`.
Capture disabled disables **event storage only**, while IDs/inventories remain
active. Enabled stores at most 4096 preallocated events and explicitly counts
loss after saturation. It is not a full causal trace of a long run.

## Workload and correctness contract

All variants initialize the private global **real** Client via native
`initialize(limit)` in a fresh process. Launchers remove `CARGO_MAKEFLAGS`,
`MAKEFLAGS`, and `MFLAGS`; no inherited build jobserver is used. Main's initially
accounted `used=1` entitlement is explicitly released before workers begin and
before joins. The permanent last real token is not released by the fixture.

* `immediate`: 10,000 retained idle acquire/release pairs.
* `one`, `multi`: real capacities 1 and 4, eight threads starting on a barrier,
  10,000 acquire/work/release pairs per thread, 80,000 total pairs. These natural
  contention cases retain scheduling variation and can also take idle paths.
* `pending-one`, `pending-multi`: controlled supplementary pending workload,
  200 batches, eight pairs each, 1,600 total pairs. A barrier starts each batch.
  After the first capacity-many workers externally acknowledge a grant through
  channels, main holds them until exactly `8 - capacity` further requests have
  registered Pending. Main then releases all worker gates. A second barrier and
  remaining acknowledgments close each batch. Thus every episode has at least
  seven/four actually registered pending requests respectively. The read-only
  pending snapshot is a setup probe, not a grant/capacity/dequeue oracle.
* `surplus`: the existing controlled local transfer followed by outstanding
  helper surplus callback, acknowledged after token consumption/return through
  real `callbacks_completed`, then one further idle pair.

External atomic active counters must never exceed the real capacity. Exact
completed pair counts must match the workload. At completion, both before and
after additionally assert `requests == grants == acks == completed_pairs`,
`releases == completed_pairs + 1`, `used == pending == helper_errors == 0`, and no
counter overflow. Disabled events/loss stay zero. Surplus records one supply and
one completed callback, and enabled small traces preserve `HelperSupply` with
`granted: false`. These are grant-decision/conservation checks, not physical
resource identity, holder leases, fairness, or exhaustive concurrency proof.

The forced-pending supplementary timer includes barrier/channel coordination and
private snapshot cloning, which can materially dominate its cost, especially
when enabled. It is not pure lock/allocation attribution. Natural contention is
kept separately to avoid averaging away an extra-lock regression. Long timed
traces are not dumped, but event/loss counts and all inventories are retained.
Final snapshot supply/completion counts may differ by one in flight; this is
preserved, not treated as callback quiescence. Surplus explicitly waits for its
single callback acknowledgment.

Timers exclude startup and final diagnostic snapshots. Contention includes joins,
immediate is loop-only. Process duration is separately retained. The executable
has a 20-second watchdog, each child launcher a 25-second timeout, and each ABBA
batch process a 160-second deadline. Watchdog sleeps do not synchronize grants.

## Preliminary samples, not final conclusions

`results/preliminary-individual/` retains eight raw batches, 48 successful samples.
One ABBA block per capture mode supplies two samples per before/after row. The
preliminary forced-pending setting is only **one batch**, not the final 200-batch
workload, and is not a pending performance verdict.

Preliminary medians, production release MAIN, same compiler/deps/harness:

| capture | scenario | before ms | after ms |
|---|---|---:|---:|
| disabled | idle 10,000 pairs | 0.845712 | 0.212749 |
| enabled | idle 10,000 pairs | 0.986634 | 0.307835 |
| disabled | natural one 80,000 pairs | 233.943 | 218.536 |
| enabled | natural one 80,000 pairs | 337.324 | 146.233 |
| disabled | natural multi 80,000 pairs | 160.722 | 142.842 |
| enabled | natural multi 80,000 pairs | 159.443 | 81.232 |

All preliminary counts/capacity/conservation checks passed, no failures/timeouts.
The preliminary run ended October 14, 2026 at 20:35:46 UTC, before the
coordinator's local native build started at 20:36:13 UTC. Nonetheless quiet status
is **unknown**, the CPU is heterogeneous, and no causality or broad performance
conclusion follows from these small-n medians. A 6-ms single pending-batch outlier
is retained rather than discarded.

## Full sampling coordination

The full 15-round local matrix was held while the coordinator ran native build
`173203c39a` and cached acceptance retry `3640775w60`. The coordinator explicitly
cleared local bootstrap contention at 20:40:29 UTC. The successful matrix task
`523798sl96` ran October 14, 2026 from 20:42:03 to 20:43:20 UTC, 76.48 seconds,
exit 0. No full-matrix samples overlap those coordinator builds. This does not
establish a quiet machine: background activity and thermal/core placement remain
unknown.

`compare.py` rotates ABBA/BAAB stage order and reverses capture-mode order each
round. Each mode has two before/two after batches per round, giving 30 samples
per scenario/variant for 15 rounds. Six scenarios give 720 total samples and
120 independently retained raw batches. It preserves every sample, per-block
paired diagnostics, aggregate min/median/max and throughput, and exact order.
Inherited CPU affinity is unchanged and recorded per sample, currently CPUs
0 through 15. No pinning is performed. Load averages are retained; quiet status
stays unknown. Scheduling, frequency, P/E core placement, and thermal effects can
vary. Individual paired ratios are diagnostics, not causal estimates or a broad
speed/default-promotion claim. The successful frozen result is
`results/abba-015-retry.json`, with raw batches in
`results/abba-015-retry-individual/` and full console output in
`results/abba-015-retry.log`.

Result SHA256:
`22829467a07bffa91cd91eea23a84bbfe922d999431b26e91f9562d8e8b3eae5`.
All 720 samples and all 120 raw batch files were independently verified, zero
runtime failures or timeouts. Counts, external capacity bounds, request/grant/ack
conservation, final aggregate use, and error/overflow checks passed. All 120
surplus samples have one supply and one acknowledged completed callback. Every
controlled pending case completed all 200 episodes, with peak exactly equal to
real capacity and seven/four registered waiting requests at each setup point.
No outlier was removed.

## Final focused measurements

Thirty samples per scenario/variant. Natural contention completes 80,000 pairs,
immediate completes 10,000, and each controlled pending case completes 1,600.

| capture | case | before median ms | after median ms | before median pairs/s | after median pairs/s |
|---|---|---:|---:|---:|---:|
| disabled | idle | 1.073910 | 0.266289 | 9,311,788 | 37,555,506 |
| enabled | idle | 1.176994 | 0.383702 | 8,496,681 | 26,064,483 |
| disabled | natural one | 287.338010 | 277.143257 | 278,630 | 288,660 |
| enabled | natural one | 280.705362 | 279.864320 | 284,997 | 286,008 |
| disabled | natural multi | 174.861898 | 189.085572 | 457,592 | 423,183 |
| enabled | natural multi | 170.185051 | 214.829179 | 470,146 | 372,389 |
| disabled | pending one | 20.361399 | 22.166133 | 78,584 | 72,194 |
| enabled | pending one | 39.655131 | 41.391242 | 40,348 | 38,656 |
| disabled | pending multi | 16.419868 | 16.718671 | 97,447 | 95,701 |
| enabled | pending multi | 24.025003 | 24.051120 | 66,650 | 66,528 |

The idle elapsed medians are lower after the change, including every one of the
15 paired blocks per capture mode. **Natural capacity-four contention regresses
in this focused run**, rather than being averaged into the idle observation.
Its aggregate median durations increase from 174.862 to 189.086 ms disabled and
170.185 to 214.829 ms enabled. Per-block median after/before duration ratios are
1.0902 disabled (11 of 15 blocks slower) and 1.1655 enabled (14 of 15 slower).
These paired observations do not prove that the additional lock alone caused
the change, given quiet-unknown scheduling on a heterogeneous CPU.

For controlled pending-one, per-block ratios are 1.0902 disabled and 1.1070
enabled, with 10/15 and 11/15 blocks slower respectively. Pending-multi paired
ratios are 0.9852 disabled and 1.0381 enabled, with 7/15 and 8/15 blocks slower.
These setup-heavy supplementary cases must not be presented as pure lock cost.
Natural one is noisy/mixed: paired ratios 1.0178 disabled and 0.9642 enabled,
with 8/15 and 6/15 blocks slower, respectively.

All min/max durations and block-level samples/ratios are retained in the JSON.
Examples of retained variation: before-disabled natural-one max 1008.639 ms,
after-enabled natural-one min 21.560 ms, after-enabled pending-multi max
407.465 ms. None is replaced or removed as inconvenient. There is no broad
compile-loop speed, causal attribution, optimality, legacy comparison, or default
promotion claim. The non-monotonic focused outcome supports investigating a
separate advisory-hint candidate, not rewriting this before/after evidence.

## Preserved runner failure and evidence boundaries

Initial task `4717919awt` failed after collecting its first six successful child
scenarios because the comparator attempted `relative_to(ROOT)` on a relative
output path. Its traceback remains in `results/abba-015.log`, and the six raw
sample records remain in `results/abba-015-individual/`. This was comparator
path bookkeeping, not a native failure. Only comparator output normalization was
fixed, without changing the frozen native source, workload harness, child runner,
or production artifacts. The retry used a fresh path, not an overwrite.

The initial unbalanced six samples and the preliminary 48 samples are preserved
separately from the complete final 720-sample matrix. They are not silently
merged, discarded, or presented as equivalent evidence. No final sample failed.
This README and the complete before/after archive/matrix are frozen for subsequent
comparisons in a **new** nested `hint/` workspace. That future comparison must
retain all three independently hashed real source variants and must not replace
this record of idle benefit plus contended regression.

## Hosted receipts

| operation | run ID |
|---|---|
| before check | `20261014-133422655-1293138` |
| before production release | `20261014-133424273-1293298` |
| after check | `20261014-133427344-1293521` |
| after production release | `20261014-133428807-1293769` |
| before correctness | `20261014-133732028-1334792` |
| after correctness | `20261014-133735049-1335435` |
| before clippy | `20261014-133738247-1335674` |
| after clippy | `20261014-133740217-1336120` |

All succeeded on hosted, warning-free. Before has 11 unmodified native protocol
tests plus this integration workflow, total 12 passed. After retains those 11
plus four new native fastpath tests plus integration, total 16 passed. Both
integration variants cover all six scenarios in both capture modes. Expected
caught boundary overflow/underflow panic output remains in logs. `cargo fmt
--check` passed. No test-binary timing is used in the preliminary matrix.
Native compiler acceptance/driver relinking is coordinator-owned and is not a
claim of this focused include-module fixture.
