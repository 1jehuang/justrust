# Actual native execution-flow acceptance, 2026-10-15

The opt-in [execution-flow protocol](execution-flow.md) now has actual-source acceptance, not just mocked or native-unit coverage. All 12 real compiler/fixture safety tests passed in 346.023 seconds against one immutable coherent compiler snapshot and a frozen installed client. This duration is the acceptance suite's runtime, not a compile-time benchmark.

## Accepted cohort

| Capture | Nodes / edges | Invocation scopes | Assigned CPU segments | Known retained CPU | Longest known CPU-chain witness | Explicit cuts |
| --- | --- | --- | --- | --- | --- | --- |
| Cold | 73,038 / 72,550 | 18,199 | 36,443 | 900.142702 ms | 44.280185 ms | 175 |
| Warm | 7,449 / 7,264 | 1,838 | 3,684 | 33.382171 ms | 15.772376 ms | 61 |
| Mixed dispatch | 73,072 / 72,590 | 18,199 | 36,470 | 682.336129 ms | 31.061217 ms | 182 |

The warm capture admitted 1,833 query jobs but recorded 1,838 execution scopes. This directly exercises the distinction between actual invocation identity and whole-job identity, including repeated execution of an admitted job. Every retained CPU segment was assigned exactly once, with no unassigned or unavailable measurements in these positive captures. This accounts for observed timing inventory, not all compiler work or dependencies.

The independent original-byte oracle verified CPU references, original admission lifetimes and original timing-row names, task roles, actual edges, endpoint times, DAG costs and valid chain witnesses. Public flow-only replay and combined query/dispatch replay agreed. The actual analyzer retained exact mode-0600 public stdout artifacts and correctly refused output collisions. On the cold retained chain, `typeck_root` contributes 37.521239 ms. The warm chain includes `resolver_for_lowering_raw` at 5.900361 ms and `lower_to_hir` at 2.510948 ms. These are actionable observed-chain locations, not aggregate categories substituted for prerequisites, and not a proof that their internals cannot be parallelized.

Compact reports are in `bench/results/execution-flow-20261015/`. Full raw captures, exact public artifacts, command logs and compiler metadata remain in private retained scratch storage. The accepted cohort is `execution-flow-real-final3-20261015/frontend-real-itoa-dspxnlim`.

## Positive and negative gates

- Real locked itoa source compiled, linked and executed boundary-value consumers. OFF/ON/OFF controls preserved argv, environment, diagnostics, cold archive bytes and warm rmeta/object/consumer payloads. The known independently randomized incremental `lib.rmeta-link` member is not claimed byte-identical.
- Actual cold/warm root5 and mixed root6 captures passed mandatory positive CPU and complete-retained-inventory checks. An all-unknown report cannot satisfy acceptance.
- A genuine native capacity-7 capture produced missing in-range terminal flow nodes. The installed consumer retained honest partial authority, validated the retained witness and did not turn cuts or lost endpoints into zero-cost proof.
- The actual native timing-allocator-exhaustion packet replayed with all causal costs unavailable and an empty witness. Default, query-CPU-only and task-DAG-only modes correctly refused pool5.
- Reciprocal copied nonzero epoch declarations were rejected. The frozen public clock protocol requires offset zero. Internal checked-offset unit tests do not advertise a new public clock protocol.
- Legacy query/read/binding/dispatch/timing captures and exact frozen warm analyzer output remained compatible. Installed-client regression replays also passed named CPU, task DAG and native exhaustion controls.

Two prior full attempts are retained as failures, not relabeled accepted. The first exposed a harness-only combined dispatch replay that omitted the explicit pool5 flag. The second exposed an incorrect harness expectation that a reciprocal nonzero clock declaration should be accepted despite the frozen same-epoch protocol. Both were corrected with regressions. Neither fix broadened producer/consumer authority or weakened the positive original-capture gate. A standalone genuine bounded-loss diagnostic also passed before the final rerun.

## Toolchain and correctness evidence

The native source-stable suite passed 247 primary tests with 28 ignored, plus 28 doctests. The coherent compiler/sysroot build then passed in 318.87 seconds, followed by isolated runtime and matching metadata copies. Runtime driver SHA256 is `2de2d0f71df3fb05f1825f7b619bc827be94e2dc2a8c4acf6c40257afe33fed9`, matching private driver metadata SHA256 is `bc8f193fb5860fc9159807cf803d3401cc23192121a3821445d748e9a3d97ffa`, and installed client SHA256 is `02c4c8cd0f396a47066be9d1143723ab8a2e4d9e44de4b5e9b0653dffcd90717`.

The integrated client passed 688 hosted tests with 3 ignored, hosted check and warning-free all-target clippy. Exact run IDs and exports are linked in `execution-flow.md` and `data/runs/execution-flow-20261015/`. Python analyzer/oracle/safety regression gates passed 109 tests.

## What is still unknown

Explicit cuts are dependency gaps, not missing CPU replaced by zero. Latch readiness, already-ready encounters, successful post-wait consumption, cache publication, generic scheduler prerequisites, process/Cargo edges and uninstrumented work are not silently reconstructed. The successful latch handoff is the next bounded native milestone. Its exact same-latch token must precede a successful consumer lookup, with unknown visibility/wake intervals preserved and poison/cycle/unwind excluded from success edges.

Full CPU work, execution span, parallelism and a useful-thread ceiling remain unavailable. No speedup or default-overhead nonregression is claimed. The comprehensive paired A2 performance gate remains blocked by its separately documented pinned-workload/environment defects and noisy samples.
