# Resident compiler speed estimate, 2026-11-14

## Bottom line

A reasonable research target is roughly **10x compiler-side latency on selected warm, small-body-edit workflows**, provided the design includes retained frontend state, function-level backend reuse and executable patching. This is supported by historical architecture comparisons, not a measurement of residency alone, not broad Rust compatibility, and not a demonstrated hosted speedup.

For the current hosted workflow, a planning scenario that removes 50–75% of recorded compilation time gives **1.30–1.52x aggregate end-to-end speedup**, holding everything else fixed. Those removal fractions are assumptions, not measured resident-cache effectiveness. There is no justified point prediction for residency alone. Even eliminating compilation entirely gives only **1.84x** on this particular retained recording mix before secondary effects on contention. Do not describe these aggregate ratios as median-call improvements.

## Actual warm-edit evidence

`docs/native-backend-frontier-br44-20261030.md` and its retained evidence bundle compare the same pinned source and four executed filtered tests, with five rotated edit samples per stack, deliberate type-error/test-failure and recovery gates, and fresh runtime markers:

| Stack | Median edit-to-test wall seconds |
| --- | ---: |
| Owned compiler, experiments off | 1.288858 |
| Owned native Cranelift plus four threads | 1.186774 |
| Blaze, pinned Cranelift | 0.100719 |
| Blaze, pinned LLVM | 0.111574 |

The off/LLVM ratio is about 11.55x. Even the lowest owned median divided by Blaze LLVM is about 10.64x. This controls backend family better than a Cranelift-only comparison, but not LLVM version, frontend implementation, diagnostic coverage or compiler semantics. It does not isolate retention from function-level codegen and patching. Stores were seeded, cold support had writable-static failures, and machine quietness was unknown. Five samples are descriptive, not a statistical performance claim.

Independent earlier focused observations corroborate the direction, not an independent broad workload sample:

* `docs/blaze-edit-comparison-20261030.md`: library check 1.015603s versus 0.114129s, about 8.90x, different diagnostic coverage.
* `docs/blaze-runtime-results-20261030.md`, br34: executed test edit 1.318050s versus 0.143916s, about 9.16x, Cranelift versus LLVM and seeded stores.

Replayed on 2026-11-14:

```
python3 bench/results/native-backend-frontier-br44-20261030/validate.py
python3 bench/results/blaze-runtime-comparison-20261030/validate-complete.py
```

Both passed retained-evidence validation. These are offline validators, not new compiler performance runs.

## Current hosted wait distribution

Command collected in this session:

```
justrust profile --remote --measured --days 7 --by command,scope --json
```

The raw local aggregate is saved at `$JCODE_SCRATCH_DIR/resident-estimate-hosted-measured.json`. It contains 2,683 measured recordings totaling 117,415.191867s (32.62h). These are observed invocations, including benchmarks/failures/contention, not an independent representative customer sample. The measured-only filter excludes 443 reconstructed recordings. No claim of deduplicated logical user requests is made.

| Non-overlapping recorded segment | Seconds | Share |
| --- | ---: | ---: |
| Compilation | 53,513.610854 | 45.58% |
| Host lock wait | 26,184.759167 | 22.30% |
| Source-sync lock wait | 6,751.858757 | 5.75% |
| Source push | 4,684.601676 | 3.99% |
| Host overhead outside Cargo | 4,520.806149 | 3.85% |
| Tests | 3,834.093890 | 3.27% |
| Artifact retrieval | 3,694.344419 | 3.15% |

Compilation includes backend/link work, so counting all of it as removable by frontend residency would be wrong. Lock time might improve indirectly with shorter jobs, but the aggregate does not identify a queueing model, so that effect is not included.

For compile fraction f and assumed removable fraction r, conditional speedup is `1 / (1 - f*r)`. Other recorded segments remain unchanged.

| Recording group | Calls | Compile share | Remove 50% of compile | Remove 75% | Remove 100% |
| --- | ---: | ---: | ---: | ---: | ---: |
| All measured hosted | 2,683 | 45.58% | 1.295x | 1.519x | 1.837x |
| Incremental test | 947 | 50.23% | 1.335x | 1.604x | 2.009x |
| Incremental build | 222 | 59.81% | 1.427x | 1.813x | 2.488x |
| Incremental check | 333 | 29.29% | 1.172x | 1.282x | 1.414x |
| Incremental Clippy | 147 | 44.95% | 1.290x | 1.509x | 1.817x |

For illustration only, making *all* recorded compile time 10x faster (90% removal) would give about 1.70x aggregate. Real resident workers only accelerate eligible warm work, so applying that factor to all compilation is optimistic.

## What query profiles establish

`bench/results/reuse-ceiling-20261030/macro-cache.json` preserves 70,506 proven-green provider-recomputation scopes, occupying a 110.519125ms interval union on profiler thread 3. The wider green value-acquisition union is 248.143496ms. These scopes overlap and must not be added. They are instrumented per-thread elapsed diagnostics, not CPU time or removable critical-path time.

The corresponding forced unchanged-source local check took 3.18s (`20261029-192203045-1482188`), but the thread scope and full-command denominator do not establish total reuse potential. The raw profile path documented in `docs/reuse-ceiling.md` was no longer present when checked in this session. The committed summary is retained evidence, not a newly decoded raw capture.

## Next measurement needed

Prototype actual bounded resident state and compare matched hosted edit pairs with current shipping behavior. Include checks, executed tests, body/signature/macro edits, negative diagnostics/recovery, cache eviction, cold starts and contention. Report compiler time and client-to-result separately, retained RAM per project, and output equivalence. Isolate frontend residency before attributing the full architecture gap to it. No production configuration or compiler default was changed for this estimate.
