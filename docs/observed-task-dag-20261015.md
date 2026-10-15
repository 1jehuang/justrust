# Observed fork/join task DAG acceptance, 2026-10-15

The explicit `--observed-task-dag` flag now retains the native instrumented fork/join topology and attaches independently authorized exclusive CPU segments to genuine tasks. It does not turn query nesting, timestamps, worker chronology, or job completion into execution dependencies.

## Accepted real captures

The installed client `dd0f06a0dd6c5d74e30cc5f5900e13956591973156977366b0450da46e44ec23` replayed the original accepted named-query CPU captures. `bench/verify_observed_task_dag_public.py` independently reconstructed every task, typed edge, CPU weight, connected component, maximum retained CPU-chain cost and valid witness. Actual analyzer reports and exact mode-0600 companion artifacts passed, including collision preservation. A copied pool-only capture retained genuine local topology and non-query weights while withholding query authority. Six original input files and the client binary remained unchanged.

| Capture | Tasks / edges | Attached CPU | Longest recorded fork/join CPU chain | Excluded task-null query CPU | Excluded opaque CPU |
| --- | --- | --- | --- | --- | --- |
| Cold | 11 / 12 | 54.787070 ms | 45.866307 ms | 496.824661 ms | 4.015578 ms |
| Warm | 14 / 16 | 33.195573 ms | 31.137137 ms | 1.754247 ms | 1.047598 ms |
| Mixed dispatch | 11 / 12 | 53.203158 ms | 44.665384 ms | 437.084668 ms | 3.942291 ms |

These are attribution facts from one cohort, not speed comparisons. The cold task DAG accounts for only about 10% of the observed exclusive CPU. The native logical query execution-flow milestone is therefore still necessary, not an optional polish step.

## Correctness and limits

- Query-labeled weights require the original admitted query inventory, reciprocal capture identity, clock bridge and segment lifetime checks. Missing or duplicate authority cannot produce a numeric query weight through the DAG path.
- Unique local task topology does not require a query file. Duplicate pool or cross-producer capture identities do invalidate local authority.
- Missing CPU is not silently a full measured zero. Partial `known_cpu_chain_ns` is only the sum of retained known task weights on explicit edges. The complete-chain field is unavailable when its required inventory is incomplete. Components without any eligible measurement have no numeric chain estimate.
- `issues` records validation/coverage failures. Unconditional scope limitations are separate. Full CPU work, execution span, parallelism ratios and theoretical ceilings remain unavailable.
- A longest observed task chain is not a complete compiler critical path, and its ratio to partial work would not bound true parallelism.

Hosted check `20261015-055202324-1010213`, full test `20261015-055126359-1004947` (664 passed, 3 ignored), and strict all-target clippy `20261015-055205643-1010112` passed. Their exports are in `data/runs/task-dag-20261015/`. Analyzer/oracle regression gates passed 78 tests. Compact actual receipts are in `bench/results/task-dag-20261015/`; full private stdout and original captures remain outside the repository.

The comprehensive A2 benchmark remains unsuitable for paired promotion: unresolved pinned-workload/environment issues and noisy samples are documented separately. No speedup or default-overhead claim is made here.
