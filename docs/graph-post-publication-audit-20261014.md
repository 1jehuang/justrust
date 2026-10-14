# Post-publication native graph audit, October 14, 2026

The published source cohort and installed public CLI were exercised again,
not merely inspected. Whole-result requirement mapping is maintained in the
companion native causal-map audit. This document records the independent
checks and the failure this second pass actually found.

## Re-run observations

- Direct native compiler binding and real proc-macro argument/output/executable
  neutrality passed again against the published candidates and installed client.
- Native admitted-job allocation, serial/parallel lifecycle, artifact reuse,
  failed interface and fail-open sink acceptance passed again.
- Hosted whole-client check, 540 tests and strict all-target clippy passed.
- Native Cargo's 39 focused scheduler/owner/registry/capability tests passed.
- All 28 cargo-util tests and its strict library clippy passed.
- Owned thread-pool tests passed 85, with 26 existing ignored tests.
- The corrected entire actual wait/read workflow passed all four tests,
  including serial/parallel waits, warm executable bytes/output, semantic reads,
  failed compilation diagnostics and alias-cycle diagnostics.

Exact hosted run exports are in
`data/runs/graph-post-publication-20261014/2026-10.jsonl`:

| Check | Recorded run |
| --- | --- |
| Whole-client check | `20261014-112734653-4075647` |
| Whole-client 540 tests | `20261014-112736286-4075890` |
| Whole-client strict all-target lint | `20261014-112747957-4073137` |
| Cargo 39 focused native tests | `20261014-112926104-4098432` |
| cargo-util 28 tests | `20261014-112928797-4099086` |
| cargo-util strict library lint | `20261014-112931468-4099434` |
| Owned pool 85 tests | `20261014-112933122-4098427` |

## Failure retained, not hidden

The first independent actual wait/read suite passed three of four tests but
failed an exact diagnostic-byte assertion for a recursive alias fixture at eight
threads. Capture-off chose `First` as the cycle's reporting root, while
capture-on chose `Second`. Both reported E0391 and the same invalid cycle.
The failure was not discarded or mislabeled as a passing first attempt.

Before changing the harness, 48 actual controls were run on the same source,
compiler and fixture path, alternating off/on/off:

| Compiler threads | Capture | Samples | Reporting roots | Distinct diagnostic SHA values |
| --- | --- | --- | --- | --- |
| 1 | off | 16 | First: 16 | 1 |
| 1 | on | 8 | First: 8 | 1 |
| 8 | off | 16 | First: 9, Second: 7 | 2 |
| 8 | on | 8 | First: 6, Second: 2 | 2 |

Serial hashes matched across capture modes. The two parallel hashes also
matched across modes, and both variants appeared in capture-OFF controls.
This demonstrates that parallel cycle-root bytes are not a deterministic
baseline contract. It does not prove that capture has zero effect on scheduling
or the distribution of diagnostic variants.

All 48 anonymous control rows and raw diagnostic digests are retained in
`data/runs/graph-post-publication-20261014/cycle-controls.json`. Complete raw
stderr, commands and compiler receipts remain in the explicitly retained scratch
fixtures. No diagnostic text was normalized or suppressed.

## Stronger, accurately scoped diagnostic gate

`tests/test_execution_graph_wait_native.py` now tests both fixtures at one and
eight threads. It requires exact off/on bytes for serial type errors and serial
alias cycles, and retains exact parallel type-error bytes. Parallel alias cycles
must preserve E0391, both alias participants, the completed-cycle explanation,
the recursive-type restriction, the well-formedness context and compilation
failure. The test reports whether bytes matched separately from whether the
required diagnostic contract passed.

The entire four-test real native suite then passed, rather than only a filtered
replacement test. There was no compiler/runtime change for this correction.

## Coverage observations, including limits

The independent parallel fixture observed zero serial waits and nonzero
four-phase registered/notified/condvar-returned/resumed wait inventories at four
and eight threads. Its large read inventory exceeded the diagnostic budget:
loss was explicit, and public `structural_complete` was false while the scoped
wait inventory remained complete. This is correct refusal, not complete read
coverage. The small incremental fixture had zero read loss and 5,870 resolved
job edges, with executed output `42`.

All public assessments still reject whole-pipeline completeness and retain null
CPU-work/span. These checks demonstrate a more informative, fail-closed scoped
instrumentation path and a corrected neutrality defect, not a compile speedup,
96-core ceiling, universally stable parallel diagnostics or complete backend,
linker, test, cache-wrapper or whole-command coverage.
