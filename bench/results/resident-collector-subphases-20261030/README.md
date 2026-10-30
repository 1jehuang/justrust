# v26 actual HIR collector inventory evidence

Accepted source commit: `3b13e250b`, against v25 `90164925c`.
This bundle contains copied frozen bytes and root-owned gate results only. The
collector worker ran no new native build, test or runtime workload to package it.

## Scope and observed result

The 16-category bounded caller-owned inventory records actual calls and cumulative
nanoseconds, explicitly inclusive. Inventory rows are not timeline events. Existing
five actual IDE phase intervals remain timeline events on their actual threads.
Do not add nested module/impl/body totals, infer exclusive costs by subtraction, or
claim a speedup from these observations.

Root native gates passed: collector IDE 6, HIR helper 2, existing outer API 5,
focused resident 2/3/1/1/3, full engine 351. Raw per-test output journals accompany
the filtered summaries. Root also independently passed the previous v25 evidence
validator/mutation checks and all 700 benchmark regression tests in 10.498 seconds.
The raw regression log is retained, not interpreted as native workload performance.

Snapshot-before/after maps cover 2444 nested workspace files. The live original
hash inventory is unchanged, the copied pre-resolution inventory equals the
original, and the immutable private selected inventory is unchanged. Its only
difference is Cargo.lock. The actual selected private resolved lock is included.
This does not modify or bless any new live workspace lock resolution.

Positive telemetry admission passed all six profile rows with zero reported
transport drops. Every row includes a normal edit collection selecting all 373
bodies. This is an attribution clue, not a caching prescription. Replicate 0 also
includes another 231-body collection, retained separately. `inventory-groups.json`
keeps emitted collection groups in log order, restarting at `module_inclusive`.
It does not assign a group to a trace thread, synthesize events, or sum duplicates.
Nested inclusive module and expression durations can exceed the outer wall time.

For the unique 373-selected-body inventory group in each of the six rows, observed
category cumulative-duration medians are below. These are six enabled-instrumentation
observations, not disabled-runtime performance estimates or exclusive compiler phase
costs. The wide ranges and recursive nesting preclude an additive wall-time budget.

| Actual category suffix | Calls per group | Median cumulative ms | Observed range ms |
| --- | ---: | ---: | ---: |
| `item_case` | 331 | 0.894 | 0.578 to 1.087 |
| `signature_generic_inclusive` | 445 | 1.314 | 0.937 to 1.777 |
| `signature_fields_inclusive` | 169 | 0.079 | 0.048 to 0.093 |
| `wellformed` | 331 | 0.436 | 0.292 to 0.676 |
| `impl_checks_inclusive` | 115 | 0.338 | 0.214 to 0.517 |
| `selected_body_inclusive` | 373 | 246.962 | 192.429 to 688.549 |
| `body_expr_inclusive` | 373 | 145.628 | 121.231 to 418.567 |
| `body_inference_inclusive` | 373 | 8.230 | 5.599 to 31.553 |
| `missing_unsafe` | 373 | 9.460 | 6.790 to 13.153 |
| `body_validation` | 373 | 16.645 | 11.465 to 17.870 |
| `body_borrowck` | 373 | 49.639 | 43.438 to 196.703 |

The selected-body expression-store category includes block-module recursion, not
just expression scanning. Thus these observations motivate investigating the actual
recursive body walk, not declaring an inference-only bottleneck or choosing a cache.
The full grouped rows retain the other inclusive categories and zero-call omissions.

Disabled strict RSB17/RSA23 engine median: 99.526 ms to 106.166 ms, +6.640 ms
(+6.67% raw). Reference-normalized ratio: 0.901716 to 0.915673 (+1.55% relative).
The requested 10% median floor was not exceeded. These uncontrolled descriptive
scoped runs have unknown noise and provide no significance or speedup verdict.
They are not a comprehensive session-weighted workload/default-promotion gate.

## Contents

- `source-ready/`: exact initial source-only handoff, including full originals,
  frozen files, source patch and readiness. Its doc is the original pre-acceptance
  version. Root's atomic source commit contains the later bounded acceptance doc.
- `build/`: frozen compiled-source inputs, source hashes and check/build results.
- `native/`: raw run journals, gate summaries, 2444-file before/after inventories,
  selected private Cargo.lock, source checks and positive test admission.
- `disabled-runtime/{rsb17,rsa23}/`: flat raw runtime outputs and strict suite
  manifests/metadata. Private runtime caches and binaries are deliberately omitted.
- `profile/`: six actual traces and worker deltas, raw outputs, identity state and
  the original pinned fixture bytes.
- `root/`: root orchestration scripts, completed gate exits, native admission,
  strict runtime comparison, positive profile admission and 700-regression log.
- `SHA256SUMS`: exact local evidence coverage, excluding its own hash and derived
  `validation.json`. No external absolute path is needed by offline validation.

Absolute paths in copied root provenance are historical identifiers. The offline
validator does not execute copied scripts, require those paths, or spawn tools.
It checks hashes, actual raw native results, private lock/source continuity,
strict runtime artifact hashes, raw counters and bounded collection grouping.
It does not independently prove compiler semantics or statistical significance.

## Replay

From the repository root, with no native workloads:

```sh
python3 bench/results/resident-collector-subphases-20261030/validate.py
python3 bench/results/resident-collector-subphases-20261030/test_validate.py
```

The local-only reconstruction command is `python3
bench/results/resident-collector-subphases-20261030/collect.py`. It requires the
original root scratch artifacts, refuses changes to previously copied payloads,
and performs copies/derived grouping/hash generation only. It does not build,
resolve dependencies or launch an engine. Replay of validation does not require
running collection.
