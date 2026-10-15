# A2 outcome and harness isolation investigation, 2026-10-15

## Preserved evidence

Read-only inputs: `bench/results/native-dispatch-synth40-A2-20261015.json`
and `bench/results/native-dispatch-synth40-A2-audit-20261015.json`.
Neither input was rewritten. The audit records baseline SHA256
`7b59851e5a660e9fdf18f2953184645ce230307f489ef8a122b228613ff601d3`.

Seven *points*, not seven individual tests, have unexpected test outcomes.
All have exit 101, `compile_failed=false`, and `expect_fail=false`.
There are 36 failed tests across these points:

| Point | Failed tests | Preserved diagnostic evidence |
|---|---:|---|
| 6 | 1 | bundled GitHub applet lacks `Review requested 1` |
| 11 | 5 | repository discovery failures, two tool-description token caps, feedback-hook output |
| 13 | 2 | test-harness detection with custom target directory, live provider catalog includes additional models |
| 15 | 10 | Git repository assumptions, ambient/onboarding state, cost model, reload/observe state, image clipboard text, UI behavior |
| 21 | 8 | repeated Git/ambient/onboarding/cost/reload/image issues, model-picker catalog |
| 28 | 1 | hint-chip horizontal position differs by 25px |
| 37 | 9 | repeated ambient/onboarding/Git/cost/reload issues, login/catalog and dispatch-state tests |

These are diagnostics, not established root causes. Repository discovery
messages support investigating snapshot Git metadata. Catalog, clipboard,
and reload messages support investigating process environment and persistent
application state. Token-cap failures may be ordinary pinned-source failures.
The test-harness detection failure needs examination of the pinned binary's
path/environment assumptions. None establishes compiler miscompilation or
proves serial test execution would fix the failure.

A2 is ineligible independently of this patch: 39/40 samples are noisy, only
point 34 is quiet, and seven points violate the existing outcome gate. The
five expected compile-error points remain 16, 17, 18, 23, 31. Identity audit
reports 86 invocations (40 pre, 40 run, 6 prev), no metadata hash mismatches,
and no missing exported identity run IDs. This is not matched A/B evidence.

## Safe changes implemented

`run.py` now acquires the existing mandatory synth lock before sampling and
before per-point restoration. Previously restoration could precede `apply`'s
mandatory lock, particularly with `BENCH_NO_LOCK=1` or direct `run_point` use.
This does not introduce parallel benchmark execution. The shared snapshot
and journal still require serialization.

`apply` now runs inside the cleanup region. If it raises after a partial
edit, the existing on-disk journal recovery runs immediately instead of
leaving the sources dirty until the next invocation. Normal measured-child
exceptions continue to restore completed edits.

Unexpected test failures now return a measurement error and retain the
measured log. Previously `run_point` accepted those failures and deleted the
log, although the report correctly blocked their aggregate. This aligns
collection with the existing report gate, not a change to expectations.
Expected compile failures remain accepted. Generator, weights, commands,
filters, test parallelism, and noise thresholds are unchanged.

## Validation and remaining gates

No full benchmark, compiler build, Rust build, or install was launched.
Run focused checks with:

```sh
python3 bench/synth/test_isolation.py
PYTHONPATH=bench python3 -m unittest bench/test_synth_reporting.py bench/test_synth_context.py
```

The final run passed 7 new isolation tests and 22 existing reporting/context
tests. `git diff --check` and `cargo fmt --check` passed, and the baseline
SHA256 still matches the value above. A disposable filesystem test exercises
actual journal restoration and generated-module cleanup. Mocked child
execution checks cleanup, lock ordering, expected errors,
and failure log retention. It does not demonstrate that the seven real
workload failures are fixed or validate actual multi-process lock contention.

Before an eligible new series:

1. Inspect pinned snapshot repository-discovery code and archived test
   environment without copying real credentials or mutating the snapshots.
2. Design an explicit, recorded application-state isolation policy. Do not
   blindly replace HOME: rustup, Cargo, toolchains, and justrust recording use
   it. Preserve declared toolchain/cache controls. Isolate application data
   and runtime paths only after identifying what the pinned code actually
   reads. Pre/run/prev must have a defined state lifecycle across stacks.
3. Reproduce the specific failed tests under unchanged parallelism and then
   controlled application state. Serial execution may be a diagnostic, not
   a replacement benchmark definition. Do not relax assertions or filters.
4. Resolve pinned-source correctness failures rather than relabeling them
   expected. Do not add Git metadata silently: build scripts may observe it.
5. Run a fresh complete matched baseline/candidate under quiet conditions,
   preserve all point plans, and apply unchanged outcome/noise gates. A2
   cannot be repaired by dropping noisy or failed points. No speedup or
   mapping/parallelism completion is established by this isolation patch.
