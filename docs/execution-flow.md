# Native execution-flow diagnostics

`justrust graph --execution-flow-dag` explicitly admits native pool schema 5 with its schema-1 `execution_flow` sidecar. This opt-in is independent of query CPU rankings, cache-call provenance, and dispatch-result provenance. Those outputs still require their respective flags. Existing flags alone do not admit pool5, and the flow flag does not bypass pool4's query-CPU opt-in.

```sh
justrust graph CAPTURE --execution-flow-dag --large-capture --json
python3 bench/frontend_mapping.py --capture CAPTURE --execution-flow-dag \
  --large-capture --graph-output PRIVATE-GRAPH.json --output SUMMARY.json
```

The analyzer creates a mode-0600 exact public-validator stdout artifact with exclusive creation. Existing files are never overwritten. Inputs are snapshotted as original bytes before validation, and the graph artifact must be outside the input capture directory.

## What a node and edge mean

The opt-in native constructor records fresh execution invocation IDs at actual query entry. One admitted query job can execute more than once, so invocation IDs are not query job IDs. Query context/job identity and static query kind remain separately qualified. CPU nodes reference existing timing2 segment IDs rather than creating a second CPU accounting system. A real pool task remains orthogonal to query execution ownership.

Observed zero-cost control gates represent scope entry/exit and instrumented fork/task entry/task exit/join. Separate cut nodes represent unknown boundaries. Only the following native predecessor tokens produce edges:

- `strand`: successive recorded operations within the same admitted logical strand.
- `sync_call` / `sync_return`: paired actual synchronous query invocation and return, not whole-job parentage.
- `fork_branch`: explicit fork to the actual child task entry.
- `join_required`: actual terminal child lineage to the recorded join.
- `continuation`: explicit join continuation, or restoration after an unknown cut.

A cut is an unknown barrier, not a zero-work link. Generic dispatch, unobserved queries, foreign recorders, task switches, missing flow, and identity/TLS failures cannot smuggle helper execution into the caller's causal strand. Cross-recorder ancestry is not invented. IDs, append order, timestamp proximity, and worker chronology never create prerequisites.

## Authority and failure handling

The consumer checks bounded typed inventories, allocator identities, loss accounting, endpoints, scope lifecycles, call/return pairing, exact original task roles and terminal outcomes, acyclicity, and temporal direction. CPU-node outgoing edges require the CPU segment's end to precede the destination, not merely its start. Query CPU needs original admitted lifetimes and a reciprocal qualified clock bridge, including nonzero epoch offsets. Scope categories are descriptors. Public named CPU steps use only validated original timing-row names, and CPU-free gates do not expose scope category names as admitted authority.

Missing records, unmeasured CPU, cuts, and exhausted/TLS-failed identities are unknown. Unavailable measurements break known-chain propagation rather than contributing fabricated zero CPU. Explicit measured zero-CPU rows remain real measurements. Independent bounded-loss capture inventories can be partial without becoming malformed, but retained missing references cannot acquire authority from a sibling capture or a duplicate producer ID.

## Reading the report

The pool assessment's `execution_flow_dag` has scope `observed-query-and-join-control-flow`. It reports retained graph inventory, assigned/unassigned CPU coverage, independently qualified CPU partitions, and a `longest_known_chain_cpu_ns` witness with exact native node IDs and qualified CPU steps. The compact analyzer ranks named CPU segments and query kinds on that witness. It does not replace the witness with a category sum across unrelated invocations.

`records_complete` means the required retained observation inventories validated. It is **not** a complete compiler dependency graph. `flow_complete` and `full_pipeline_complete` remain false. `cpu_work_ns`, `execution_span_ns`, and `parallelism` remain null. A retained known CPU-chain cost is not wall-time critical path or theoretical span, and dividing partial work by partial chain cost would not bound true parallelism.

Result readiness, latch success/failure handoffs, cache publication, pre-attach work, uninstrumented compiler work, Cargo/process dependencies, and resource constraints are not inferred. These require separate native release-token boundaries. In particular, whole-job completion occurs too late to substitute for exact result visibility.

## Validation status

The final integrated client passed hosted check `20261015-060851299-1146107`, full test `20261015-060855169-1146356` (688 passed, 3 ignored), and warning-free all-target clippy `20261015-060912803-1145921`. The final module-specific targeted test and clippy runs are `20261015-060759656-1135797` and `20261015-060808166-1137502`. Dedicated exports are in `data/runs/execution-flow-20261015/`.

Actual coherent-compiler source acceptance, independent installed replay, and retained bottleneck results are separate gates. Unit tests alone do not establish that the real compiler emits a complete usable observed flow. No compile-time speedup, default-overhead nonregression, full critical path, or useful-thread ceiling is claimed by this diagnostic feature.
