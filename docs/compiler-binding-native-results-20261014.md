# Native compiler binding acceptance, October 14, 2026

## Delivered scope

Source cohort `d3a5319b9` integrates native Cargo compiler-launch declarations,
independent compiler entry receipts, existing schema-3 query receipts, and
`justrust graph DIR --compiler-manifest FILE --json` reconciliation. Coverage
is explicitly one selected BuildRunner/compiler authority, not an entire Cargo
command or compilation pipeline. The installed client was used for acceptance.
The native Cargo and compiler/std/proc_macro candidates were built locally in
isolated diagnostic locations. Default native toolchain snapshots were not
replaced by these candidate builds.

## Actual acceptance, not synthetic substitution

The final rebuilt pair passed these real workflows:

| Workflow | Observed result |
| --- | --- |
| Dirty native compile | 1 unit, 1 launch, 7 expected/observed journals, 1 independently bound entry and 1 complete query receipt |
| Fresh Cargo reuse | 1 unit, 0 launches, 3 expected/observed journals, no fabricated compiler entry or query receipt |
| Intentional Rust type error | Compilation failed normally; failed/interrupted query evidence did not count as complete |
| Removed entry/query/journal/declaration evidence | Required missing evidence never became a complete selected authority |
| Blocked or nonprivate output sink | Ordinary compilation continued; the nonprivate case reported loss and missing child evidence, not completeness |
| Warm capture off/on/off | Executable bytes identical, with normal fresh/incremental reuse retained |
| Real proc macro inspecting `std::env::args()` | Complete argument vectors and generated application output identical with capture off/on; executable SHA equal; genuine native entry receipt required |
| Native admission regression | Allocator reconciliation, serial/parallel capture, artifact reuse, failed interface, and fail-open sink passed |

The final macro, binding, and admission tests completed successfully in one
10.13-second harness invocation. Individual unittest durations were 1.430,
7.678, and 0.847 seconds. These are acceptance durations, **not speedup evidence**.
The separate native wait/read workflow also passed all four tests. The combined
client check, 540 tests, and strict all-target clippy passed.

Reproducible tests:

- `tests/test_execution_graph_compiler_native.py`
- `tests/test_execution_graph_compiler_argv_native.py`
- `tests/test_execution_graph_query_native.py`
- `tests/test_execution_graph_wait_native.py`

The first two require explicit `JUSTRUST_NATIVE_CARGO` and
`JUSTRUST_NATIVE_RUSTC` candidate paths. `JUSTRUST_NATIVE_CLIENT` can select the
installed client. `JUSTRUST_NATIVE_EVIDENCE_DIR` retains raw fixture evidence
outside the repository. No incompatible sysroot artifacts were substituted.

## Defect found and corrected before publication

The initial transport appended a private `-Zjr-query-binding-fd=...` argument
only at actual process execution. Although it was absent from Cargo's logical
arguments and stripped from a copied driver argument vector, a real proc macro
could still observe the original process argv. The regression produced
`binding_visible=false` without capture and `binding_visible=true` with capture,
changing generated code. That design was rejected.

The published transport uses no argument or environment additions. Its exact
capability marker is `jr-query-admission-binding: reserved-fd-v2,query-schema=3`.
A sealed bounded memfd and private directory descriptor carry anonymous binding
IDs. Child mapping reserves descriptor 198 atomically without overwriting
foreign descriptors. Borrowed probing validates duplicates without changing
original descriptors, offsets, or flags. Only an explicit unsafe native
process-main ownership boundary consumes a completely validated transport.
Foreign files, directories, pipes, malformed memfds, collisions, invalid
permissions, real exec, and allocation at the reserved slot have regressions.

## Limitations that remain explicit

- Every assessment retains `full_pipeline_complete: false` and null CPU-work
  and execution-span estimates. These receipts do not establish a thread ceiling.
- Only capability-verified, unwrapped native DefaultExecutor compiler launches
  are supported. Wrappers, custom executors, stock compilers, cache-only wrapper
  outcomes, and wrapper early replies are not elevated into native coverage.
- Backend/codegen, linking, test execution, resource internals, and whole-command
  launch inventory are not made complete by this selected-authority profile.
- Other producer sidecars are reported as unreconciled rather than silently
  included in selected compiler coverage. The successful fixture retained one
  such file.
- Event budgets, interruptions, missing seals, publication failures, and
  descriptor collisions yield incomplete/unknown coverage, not inferred work.
- General incremental rlib container byte identity is not claimed. Randomized
  incremental object basenames already vary without capture. Executable-byte
  neutrality was tested directly instead.
- Diagnostic capture has measurable overhead and remains opt-in. No broad
  benchmark, default promotion, speedup, or arbitrary proc-macro resource/timing
  invisibility is claimed.

Slim targeted hosted run exports are in
`data/runs/compiler-binding-20261014/2026-10.jsonl`. Combined client and native
wait/read/pool evidence is documented separately in
`docs/native-causal-map-results-20261014.md`.
