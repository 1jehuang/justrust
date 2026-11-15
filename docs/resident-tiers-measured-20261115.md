# Resident compiler tiers: actual hosted measurements, 2026-11-15

## Bottom line

Keeping the experimental compiler worker alive gives about **10.8x faster small-edit test results** than the current shipping compiler on this fixture. Keeping only the active project resident preserves that hot speed, but returning to an evicted project costs **8.17 seconds**, versus **0.0945 seconds** when that project remained resident. This is a real latency/RAM tradeoff, not a measured full-heap snapshot solution.

| Mode | Hot edit to test result, median | Return to project after using peer, median | Retained daemon + workers PSS after peer |
|---|---:|---:|---:|
| Current shipping compiler | 0.989706 s | Not separately measured | No comparable resident-worker measurement |
| Uncapped resident prototype | 0.091533 s | 0.094498 s | 4.590479 GiB, two projects |
| Memory-budgeted resident prototype | 0.091692 s | 8.171734 s, eviction miss | 1.922605 GiB, one project |

Five samples per cell. RAM is the median summed process PSS of the isolated namespace at the same post-peer transition, not peak usage or total host RAM. Bounded hot-edit post-result retained PSS was 1.435197 GiB. The two-project post-peer comparison saves about 58% retained PSS by discarding the inactive worker.

The eviction-miss response is not the entire reload lifecycle: median response-plus-readiness observation was **9.014815 s**. That includes status probes, source/log inspection and memory accounting after the response, so it is not a pure uninstrumented reload timer. Uncapped hot response-plus-observation was 0.243693 s and bounded hot was 0.189190 s. Compare response latency with response latency, not these instrumented readiness figures.

## What was actually tested

- Same authenticated hosted account VM: 16 vCPUs, 28.9 GiB RAM, no swap. Host was shared and busy. No local laptop compilation.
- Pristine pinned historical justrust source `f97fcc2`, 63 tracked files, private copies, opt-level 0, debug 0, incremental enabled. Source bytes for matched native phases were identical.
- Current published justrust executed on the hosted guest with its normal owned compiler (`jr-ac20de5e7921`). This excludes upload, client network and remote dispatch latency.
- Experimental Blaze-derived `jr-engine`, built on the hosted guest against its exact nightly-2026-10-27 toolchain. Engine SHA-256: `d3a08ee76b1f45882a3ff44b0b4c87f25bab76b096ec2c010b5b5229c275c314`.
- The resident prototype uses Cranelift, fine-grained function reuse and executable patching. Current and prototype compiler versions/backends differ. **The 10.8x is the combined prototype improvement, not a causal measurement of RAM residency alone.**
- Uncapped mode retains A and B. Bounded mode uses native `RA_JIT_MEMORY_MAX=1M` to force an idle worker eviction, with spare workers disabled. This is a soft worker budget: the most recent/busy worker is exempt and can exceed the configured number. It is not a 1 MB total memory limit.
- A hot edit follows a prepared ready A worker. Then B is edited, and A is edited again. Bounded misses prove A's old PID/starttick exited, a fresh native budget-eviction log was appended, the daemon persisted, and a distinct ready replacement A worker appeared.
- Fresh native logs show **cold compilation, function-store reuse and linking** on misses. They are not reported as hot edits or as restoration of all decoded compiler state.

## All measured phases

Median response seconds, five accepted samples per phase:

| Phase | Current shipping, repeat | Uncapped resident | Bounded resident |
|---|---:|---:|---:|
| Hot edit | 0.989706 | 0.091533 | 0.091692 |
| Edit after peer project | Not measured | 0.094498 | 8.171734 |
| Type error | 0.618616 | 0.082721 | 0.288002 |
| Type recovery | 0.947483 | 0.098928 | 0.141646 |
| Assertion failure | 1.041890 | 0.092215 | 0.091175 |
| Assertion recovery | 1.051316 | 0.091655 | 0.090208 |

The earlier current baseline hot-edit median was 1.005744 s, versus 0.989706 s after the native experiment, about 1.6% difference. This is a drift check, not proof of an uncontended host. Native order rotates across repetitions. Hot and miss latencies are not averaged together without a real workload's miss rate.

## Correctness, recovery and provenance

- Native run `/home/jeremy/bt15c`: **60/60 accepted timed rows**. Every runtime result had the expected fresh nonce/source marker and exact four-test filtered contract. Type errors, assertion failures and both recovery paths were exercised.
- Current repeat `/home/jeremy/rt15-base-c`: **25/25 accepted timed rows**. Full recorded compiler/test logs were fetched by explicit run ID outside timing, because the compact client console omits passing test details.
- Separate full-library gate `/home/jeremy/rt15-gate`: both engines produced the same **147 passed, 1 ignored, 148 total, 0 filtered** test statuses on this pinned fixture. This is not broad Rust compatibility certification.
- The owning agent server reloaded after all 60 native measurements and both successful daemon-stop commands, but before the final integrity save. Original `metadata.json` honestly remains `incomplete`; it was **not rewritten** to claim a clean uninterrupted run.
- `independent-verification.json` separately validates all completed rows, pristine source, restored copies, executable and full toolchain identities, toolchain symlinks, all raw hashes, successful stop receipts and absence of recorded process identities. **All nine checks passed.** Original metadata/records/receipts hashes are recorded there.
- Exported native evidence includes **651 raw streams**, independently SHA-256 checked locally after export. Current repeat has 106 raw streams, also independently checked. The earlier baseline and full-gate exports were checked separately.
- Failed or interrupted preliminary attempts are not pooled into the medians. In particular, `/home/jeremy/bt15b` was interrupted with 37 rows and had an overly strict hot-edit oracle for cold reloads. Its results are excluded.
- Harness checkpoint for the accepted native run: `519b05eda`. Python benchmark/control suites: **76 tests passed**. No production code changed or new default enabled.

## Is it worth it?

**Yes for hot project residency.** Keep active projects resident, with admission/eviction based on measured RAM and expected reuse. The measured small-edit result is around 92 ms instead of around one second. A tight budget can keep that speed for the active project, but aggressively cycling projects gives severe misses.

**Not yet evidence for full-state disk snapshots.** This prototype's eviction saves selected artifacts and rebuilds decoded state on return. It does not serialize every compiler query/IR/codegen structure and resume it without recomputation. The user's proposed complete RAM-vs-full-state-disk comparison still requires implementing a real resumable state format or process snapshot path, then measuring it. Byte-read/decompression benchmarks cannot fill that gap.

These are small-edit host-only prototype results, not a claim about uv release builds, production multi-tenant throughput, network-inclusive latency, pure residency attribution or a universal 100x improvement.

## Evidence and reproduction

Raw top-level exports and source provenance are in `bench/results/resident-tiers-20261115.tar.gz`. This contains no build targets/toolchains and no synthetic replacement logs. It preserves the native original incomplete metadata plus its separate verification receipt. The recovery verifier is included as provenance, with its explicit experiment paths.

```sh
python3 bench/resident_tier_compare.py --protocol budget \
  --engine /ABS/jr-engine --source /ABS/pristine-f97fcc2 \
  --output /ABS/fresh-short-output \
  --toolchain /ABS/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --samples 5 --timeout 360 --memory-max 1M --settle-timeout 120

python3 bench/resident_current_baseline.py --justrust /ABS/justrust \
  --source /ABS/pristine-f97fcc2 --output /ABS/fresh-baseline \
  --samples 5 --timeout 180
```
