# Actual latch-readiness mapping acceptance, October 15, 2026

The complete actual-source suite passed all 14 tests, with no skips, in 475.022 seconds. It compiled, linked and executed the locked `itoa` source with an immutable coherent compiler, matching private metadata and frozen installed consumer. This is a correctness and provenance acceptance, not a compile-time benchmark.

## Retained observed graph

| Fact | Cold | Warm incremental | Cold with dispatch |
|---|---:|---:|---:|
| Native nodes / edges | 74,024 / 73,538 | 7,412 / 7,238 | 74,506 / 74,028 |
| Actual invocation scopes | 18,199 | 1,838 | 18,199 |
| Assigned measured CPU segments | 36,736 | 3,659 | 36,934 |
| Readiness rows | 80 | 0 | 130 |
| Actual wait encounters | 139 | 0 | 180 |
| Registered encounters | 134 | 0 | 170 |
| Already-ready encounters | 5 | 0 | 10 |
| Qualified transfers | 138 | 0 | 180 |
| Unbound encounters | 1 | 0 | 0 |
| Retained known CPU, ms | 513.291338 | 34.641187 | 522.313801 |
| Longest known CPU chain, ms | 20.787135 | 16.342947 | 24.054486 |
| Explicit unknown cuts | 511 | 49 | 662 |
| Readiness inventory qualified | yes | yes | yes |
| Wait / aggregate records complete | **no** | yes | yes |

Every retained CPU segment is assigned exactly once in these cohorts. There are no missing/unavailable CPU references or lost native node/edge/scope/readiness/wait rows. That does not make their full execution DAG complete.

The cold run genuinely encountered the conservative post-unlock publication-finalization race. Its unbound encounter is preserved, not filtered, and `wait_records_complete` and `records_complete` correctly remain false. The other 138 transfers are independently qualified, including all 134 registered encounters. Warm zero means zero actual recorded encounters in this cohort, not a claim about all incremental workloads.

## Actionable localization, not a parallelism ceiling

On the cold retained known chain, `typeck_root` contributes 17.556408 ms, followed by `impl_super_outlives` at 0.810641 ms and `clauses_of` at 0.796283 ms. In the dispatch cohort, `typeck_root` contributes 20.241784 ms of the 24.054486 ms known chain. The warm chain instead includes `resolver_for_lowering_raw` at 5.991975 ms and `lower_to_hir` at 2.470884 ms.

These are costs on validated native-edge witnesses, not category sums across unrelated invocations. They identify type checking and resolver/HIR work as concrete investigation locations. They do not prove those operations cannot be partitioned, predict a speedup, or establish a useful-thread limit. These instrumentation cohorts are not paired performance comparisons with each other or earlier captures.

## Acceptance checks

- Original-byte independent oracle checks every retained node/edge/scope, CPU partition, row-bound readiness obligation, original registered wait lifecycle, public query label and maximum known-chain cost/witness.
- Actual analyzer reports check named rankings and an exact mode-0600 companion public-validator stdout artifact with recorded size/hash and collision refusal.
- Cold OFF/ON/OFF archive bytes match. Warm OFF/ON/OFF metadata/object payloads and linked consumer behavior match. Randomized incremental `lib.rmeta-link` bytes are retained but not falsely claimed identical.
- Root5 cold/warm and root6 dispatch are exercised with explicit readiness admission. Old flags still refuse pool6. Unsupported nonzero reciprocal epochs are hard refusals.
- The original query, timing, dispatch, named CPU and flow-v1 tests all passed alongside the new readiness tests.
- Separate genuine native guard packets cover failure, unwind, missing original wait ID and unbound races through sixteen public cases. Those deliberately lack query-root authority and remain typed partial reports, not invented qualified transfers.
- Native source-stable tests passed 258 primary tests with 28 ignored, plus 28 doctests. Final hosted client tests passed 709 with 3 ignored, with check and strict all-target clippy green. Final hosted run summaries are committed under `data/runs/latch-readiness-20261015/`. Combined offline mapping/oracle/control tests passed 139.

Expanded independent installed-client replay is a separate gate whose receipt will be published after completion. Compact actual analyzer reports are under `bench/results/latch-readiness-20261015/`; raw captures and full witnesses remain private.

## Reproducible artifact identity

- Native implementation: `834da6ee4`. Later native changes are test-only export hooks, with the production source prefix unchanged.
- Client central / strict readiness: `3bb4cd539` / `a422e4bd3`.
- Frozen client SHA256: `c477b59037551e26babe623103e1d52449717c3789c04917089798e1bfa203e0`.
- Runtime driver SHA256: `12c7a06ecf881b7ca2a39921a1bebb5a75cc94ba38de200abce0dba93762a42e`.
- Matching private driver metadata SHA256: `27da8d1ee34aa3044633e900188dd9ffeaca76570beffe310e96f91c29f02bcd`.
- Coherent compiler/sysroot build passed in 484.75 seconds. The actual driver compiled successfully against copied private metadata before the full suite. All frozen artifact hashes still matched afterward.

The first full fourteen-test attempt is retained honestly: 13 passed and one independent-oracle check failed after 375.169 seconds. The missing oracle role was the exact row-owned `query_wait` cut with neutral task followed by genuine same-scope CPU with a qualified task. Native and strict consumer already handled it correctly. The narrow correction added adversarial controls, preserved default v1 behavior and replayed the unchanged original packet before the successful fresh full suite. No compiler/client relaxation or input rewriting was used.

All full work/span/parallelism fields remain null. Visibility and wake-to-consume gaps remain unknown barriers. Cache publication, semantic reads, uninstrumented scheduler prerequisites, I/O and pre-attach boundaries remain outside this map. Paired performance promotion remains blocked by the documented A2 baseline defects and noise. See [the protocol and authority rules](latch-readiness.md) and [the overall parallelism map](parallelism-map.md).
