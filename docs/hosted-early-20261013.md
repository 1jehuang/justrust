# Bounded hosted provisional diagnostics, 2026-10-13

## Result and scope

On the pinned Desktop snapshot, the first visible type error arrived **37.5% earlier** with an opt-in, prewarmed hosted analyzer. The complete Cargo verdict did not get measurably faster. This is not a 10x compile speedup, nor a comparison against stock local Rust.

`bench/suite.py compare bench/results/hosted-early-same-client-off-20261013.json bench/results/hosted-early-after-20261013.json` reports:

```text
  first error                   2.19    1.37   -0.82 (ranges 2.12-2.41 / 1.26-1.99)
exit overhead (med/max)      1.430/1.647s -> 1.405/2.088s
```

| Metric, seven measured runs per setting | Diagnostics off | Query-only hosted diagnostics | Decision |
|---|---:|---:|---|
| First visible error, median | 2.192 s | 1.369 s | 0.823 s earlier, 37.5%, nonoverlapping observed ranges |
| Full command exit, median | 2.430 s | 2.400 s | Within noise, no full-build win |
| Host compile, median | 0.808 s | 0.810 s | Unchanged |
| Early errors confirmed by rustc | Not applicable | 1 in every measured run | No early-only or rustc-only errors |

Every measured command actually ran on `hosted`, compiled one local UI unit, and returned the complete Cargo failure, exit **101**, for the injected E0308. Query-only mode never returns an early final verdict. The benchmark refuses a result when the analyzer is not ready, no provisional error was shown, rustc does not confirm it, or an early exit occurred. False `early.exited` is omitted by summary serialization, so the raw timing projection contains null for that optional field. The actual summaries record Cargo exit 101 and the matching rustc diagnostic.

The host-side diagnostic query took approximately 45-120 ms in measured runs. Client admission and transport dominate the remaining first-error latency: later passing-build traces spent 1.10-1.33 s in the charged phase, with approximately 90-107 ms sync. Those traces are observations, not an isolated latency comparison.

## Reproducibility

Both controlled settings use the same frozen installed client, declared revision `7d4474b`, SHA-256:

```text
a89c983da61666c32a3d4ea2f1dad5a790ef56277b0333709de0451cc8966891
```

The benchmark verifies executable identity before and after the suite. The checkout is explicitly recorded as dirty because other agents have pending changes. Exact executable identity, not repository HEAD alone, establishes the controlled comparison.

Snapshots are Desktop `7d38fb7` and sibling Jcode `abff94eeb`. The compiler is the unchanged default `nightly-2026-10-08`, on the previously accepted own-account M8azn host. Each setting has two discarded warmups and seven measured runs in the persistent hosted target.

Commands, with `JUSTRUST_BIN` selecting that frozen executable and `JUSTRUST_BENCH_REV=7d4474b`:

```sh
python3 bench/suite.py run --remote --toolchain nightly-2026-10-08 desktop-type-error -n 7 --warmup 2 --out bench/results/hosted-early-same-client-off-20261013.json
python3 bench/suite.py run --remote --hosted-early --toolchain nightly-2026-10-08 desktop-type-error -n 7 --warmup 2 --out bench/results/hosted-early-after-20261013.json
```

The older `hosted-early-before-20261013.json` preserves the initial seven-sample observation with frozen caller-umask client `7096694`, a different SHA and earlier network epoch. It is **not** the controlled off setting and is not used to attribute an implementation speedup. The three `hosted-early-prewarm*.json` files preserve admitted startup observations, not performance comparisons. `hosted-binary-provenance-smoke-20261013.json` preserves the actual executable-selector acceptance.

## Safe startup and failures found

The analyzer was launched only from an authenticated, admitted hosted build. Each later query benchmark also follows ordinary admission and runs full Cargo. No speculative build daemon, altered quota, identity shortcut, machine resize, release/tag, or worker deployment was added.

The machine-local launcher is opt-in and installed only on the verified own-account host. It starts a systemd user service with hard 6 GiB memory, no swap, 256 tasks, 30-minute maximum lifetime, at most two workspace services, disk/memory-headroom checks, and control-group child cleanup. Cargo compile-time dependency jobs are limited to two inside this service. These limits do not change the admitted Cargo build's job budget. Missing or invalid hosted launcher policy fails closed for diagnostics and leaves the real build unchanged. Clients cannot forward the trusted resource-limited marker or `JUSTRUST_EARLY_EXIT`.

Actual kernel evidence is in `hosted-early-control-20261013.json`:

1. Initial unrestricted Cargo concurrency exhausted the 256-task cap, with 232 task-limit events and rustc EAGAIN helper-thread failures. Cargo stalled with zombie compiler children. The real admitted build still passed. Private launcher `2ba0e1e` fixed analyzer-only Cargo concurrency. The retry had zero task-limit events and a peak of 112 tasks.
2. Eager cache priming still was not ready after 227 seconds. It reached 6,052,585,472 bytes peak cgroup memory and 55,945 soft MemoryHigh events, without OOM. I stopped that exact experimental service rather than claiming a warmed success.
3. Public `ba5fc2d` fixes offline source discovery. Unfiltered metadata on a Linux mirror tried to fetch absent `accesskit_macos` sources and lost sibling path crates. Host-platform filtering and the selected compiler environment recovered 53 local/path packages. The actual restarted daemon tracks 45 deduplicated roots and 780 files, versus one root and 289 files before the fix.
4. Public `7d4474b` and private machine policy `321324f` disable eager priming and use four analysis threads only inside the bounded service. Local analyzer defaults remain unchanged. The actual daemon reached healthy quiescence in **2.219 s**, using 3,862,499,328 bytes cgroup memory, zero memory/task-limit events, and 63 peak tasks at the readiness observation. This is a readiness observation after earlier dependency startup attempts, not a completely cold analyzer speed ratio.

After the error measurements and passing-build acceptance, the analyzer was still ready, with zero erroring files and 12 queries. Peak cgroup memory was 5,480,386,560 bytes. It accumulated 37,120 soft MemoryHigh events, but no hard-limit, OOM, or task-limit events. This pressure is a reason **not to enable diagnostics by default**. The exact experimental service was stopped, leaving no live analyzer processes. Three defunct analyzer entries were visible immediately after cleanup. No private source or artifact caches were deleted.

## Acceptance and publication

Public changes `57440a9`, `ba5fc2d`, and `7d4474b` were checked and installed incrementally. Final source validation passed 355 hosted Rust tests, zero failures, two ignored, warning-free all-target clippy, and 21 Python harness tests. The earlier Python module-form invocation failed because `suite` was not on its import path. The correct `python3 bench/test_suite.py` invocation passed, with no test-code workaround.

After restoring the injected error from the snapshot backup, query-only passing UI check `20261013-140402019-3055523` returned zero, and unchanged FPS tests `20261013-140404921-3055825` passed all three tests. No stale provisional error was displayed. The source SHA remained unchanged across these acceptance commands.

Installed **plain default** self-dogfood check `20261013-140412004-3056545`, full test `20261013-140415130-3056940`, and lint `20261013-140418660-3057272` all ran on hosted. Each reported zero compiled and 56 fresh units, preserving recording/cache behavior. The full suite passed 355 tests with no failures.

All 56 exactly owned measured, warmup, startup, source-validation, and installed-acceptance runs listed in the control artifact are exported into `data/runs/xps14/2026-10.jsonl`. Unrelated agents' pending records and source changes are excluded from this evidence commit.

**Decision:** retain the validated optional query-only feature and bounded machine policy, with the experimental service stopped and fleet/default diagnostics still off. Earlier diagnostics are useful, but successful check/test loops and the complete error verdict still need deeper optimization. The 10x stock-local target remains unmet.
