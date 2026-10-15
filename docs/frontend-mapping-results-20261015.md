# Frontend dependency mapping, 2026-10-15

## Question and scope

Improve the evidence needed to locate frontend parallelism bottlenecks on a real Rust compilation. This is not a measured speedup or a measurement of a 96-thread ceiling. A causal subgraph alone does not establish total CPU work W, weighted span S, or W/S. Missing ownership, non-query work, previous-session dependencies, resource delays and clock alignment remain relevant.

## Shipped consumer capability

Commit `7c5354279` adds explicit `justrust graph --large-capture` for regular-file or directory captures. The default remains 64 MiB and 100,000 read/binding records. The opt-in budget is 512 MiB and 1,000,000 read/binding records, with independent admission/wait/pool bounds unchanged. Manifest modes reject the option. Original-byte validation and fail-closed authority remain required.

Read coverage now reports `resolved_reads` separately from deduplicated `resolved_job_edges`, and mutually exclusive `unbound_read_reasons`. Later producer publication explains a gap but never retrospectively creates a dependency edge. Missing producer or consumer admission endpoints are classified as `job_endpoint_unavailable`.

The optional large query reader does not promote oversized resource-attachment authority. That separate parser remains strict and can report unknown attachment coverage. The collector's outer scan budget follows the explicit large policy, independently of those authority limits.

Validation of this consumer slice:

- Hosted check: `20261014-174118989-3846688`, passed.
- Hosted full tests: `20261014-174141889-3848874`, 596 passed, 3 ignored.
- Hosted clippy: `20261014-174153814-3849708`, no warnings.
- `cargo fmt --check` passed.
- `cargo install --path . --locked` completed, updating `/home/jeremy/.cargo/bin/justrust`.

These run IDs use the machine's local October 14 date. They ran during the October 15 UTC session.

## Real-build baseline

The acceptance harness compiles the unchanged, locked registry source of `itoa 1.0.18` to an rlib, using the accepted pre-change driver. It exercises cold and incremental builds at one and four frontend threads. It is a correctness/coverage diagnostic, not a general benchmark.

Baseline observations:

| Case | Admissions | Retained read records | Lost read records | Retained binding records | Lost binding records |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cold, 1 thread | 18,199 | 100,000 | 807,318 | 100,000 | 383,148 |
| Cold, 4 threads | 18,199 | 100,000 | 811,226 | 100,000 | 385,282 |
| Incremental, 1 thread | 1,833 | 31,514 | 0 | 19,596 | 0 |
| Incremental, 4 threads | 1,833 | 31,514 | 0 | 19,596 | 0 |

The four-thread incremental capture had 3,028 bound points and 16,568 `no_active` points. The cold serial capture had no pool authority. These are scoped observations, not whole-frontend dependency coverage percentages.

Cold OFF/ON/OFF full rlib bytes matched. Incremental archive bytes differed even in OFF/OFF controls because of randomized incremental object basenames in link metadata. Incremental rmeta and object payloads, linked consumer binary bytes, and integer-boundary execution matched. Do not claim full incremental archive-byte neutrality.

Raw baseline evidence is private under `$JCODE_SCRATCH_DIR/frontend-real-baseline-20261014/`. The updated harness consumes and closes an owned inherited descriptor before compiler entry, preserving identical compiler process argv/environment between OFF and ON. Older argument-controlled attempts are retained but do not establish that stronger neutrality property.

## Native changes and validation

The coherent compiler build, actual ownership fixture and native unit gates validate these scoped changes. The real-library large public-reader path additionally required the integration fix described below:

1. Associate an already-computed result with its producer before making the query cache entry visible. A deterministic native cache test checks that a concurrent hit cannot observe the entry before that association. Association is not proof of fresh provider execution or successful insertion.
2. Add programmatic, bounded read/binding capacity with unchanged 100,000 defaults and a hard 1,000,000 ceiling.
3. Own the actual interface invocation with a frontend root task when experimental frontend observation is enabled. Preserve opaque dispatch masking. A nested-restoring dispatch-mask marker prevents root entry inside generic dispatched work where `NoActiveTask` alone is ambiguous.
4. Retain exact synchronous cached-query and generic-dispatch witnesses, not inferred or fabricated ancestry.

## Coherent compiler and ownership acceptance

The final owned compiler and std build passed (`039553qbmc`, 597.66 seconds). This is a rebuild duration, not a speedup comparison. The preserved driver SHA-256 is `1e31fec19683ee1394cc5a3c3458f3353ba84079e63b1ed3f8681e8a7fbe0e05`. Matching private metadata and sysroot are preserved under `$JCODE_SCRATCH_DIR/native-frontend-mapping-accepted-20261015` before native unit tests can modify build outputs.

The actual frontend ownership fixture passed in 12.839 seconds with 33,316 expected and observed binding points. Entry task 1 advanced to actual resumed continuation 4. Synchronous cached requests `[974,976)` and reads `[733,735)` bound to continuation 4. Actual generic-dispatch cached requests `[1080,1082)` and reads `[822,824)` remained `no_active`. Masked root-entry refusal preserved task allocator and event state. Four registered and four resumed wait points were observed. Negative authority mutations and artifact-neutrality checks passed.

Scope evidence: `$JCODE_SCRATCH_DIR/frontend-scope-final20261015/native-task-binding-o3dfj8n0`. Native crate tests passed with 205 tests passed, 28 ignored and zero failures across query_impl, session, thread_pool and six pool integration binaries (`688289q0yc`). The publication race regression passed again under an exact filter, one passed and one filtered (`8264447a07`). These are actual frozen-source native crate tests built by bootstrap using stage0, separate from the stage1 real-build acceptance. Logs are preserved under `$JCODE_SCRATCH_DIR/native-frontend-mapping-crate-tests-20261015/`. No native performance improvement is asserted here.

## Coverage comparison and acceptance-discovered boundary

`bench/results/frontend-mapping-20261015/baseline-warm.json` and `final-warm.json` contain validated, slim summaries of the same real incremental workload. Bound task points increased from 3,028 to 17,024 of 19,596, while unbound points decreased from 16,568 to 2,572. This is an ownership coverage improvement, not a speedup. Binding completeness remains false and the report preserves that issue.

Read semantics are unchanged: 7,892 dependency reads, 77 resolved reads, 15 distinct job edges, and 7,815 unbound reads. The unbound reasons are 6,157 `eval_always`, 1,653 `ignore`, and 5 `no_prior_producer`. Read records have no loss. These categories describe observed read tracking semantics, not an estimate that 99% of useful frontend work lacks dependencies.

The first final large-capture acceptance exposed a separate reader integration failure: the query reader accepted the opt-in budget, but resource-document scanning still rejected the 223,973,636-byte query with `resource aggregate budget exceeded`. The failed evidence remains under `$JCODE_SCRATCH_DIR/frontend-real-final-20261015/frontend-real-itoa-0whx1g9h`. Raw inspection found 911,210 retained read records and 485,256 expected/observed binding records, both with zero loss. Those raw counts do not substitute for successful public validation. The collector fix now permits the explicit 512 MiB scan while retaining strict individual authority parsers. The installed release consumer contains the final hard-bound guard and has SHA-256 `8168b750706578d56adcc611588bbe19c03cd999071c4fa598b2949fc2a47916`. Public validation and the analyzer accepted the same previously rejected original capture bytes. `final-cold-large.json` records 911,210 read-slice events, 485,256 binding points, zero read/binding loss and all 79 expected waits complete. Only 93,077 binding points are bound, so the report correctly retains incomplete graph binding.

Importantly, all 217,244 direct cold dependency reads are classified `disabled`, with zero resolved read-derived job edges. This cold workload does not have normal incremental dep-graph tracking enabled. Loss-free recording therefore does not mean a complete cold semantic dependency graph. Observing query-call dependencies independently of incremental dep-graph enablement remains required.

The complete unchanged acceptance suite then passed three tests in 19.791 seconds with no skips: descriptor safety, one/four-thread cold/incremental compilation, and opt-in large cold capture. Final cohorts are `$JCODE_SCRATCH_DIR/frontend-real-final-20261015/frontend-real-itoa-wj476lyq` and `frontend-real-itoa-00gtnd_g`. The fresh large run retained 910,398 read-slice events and 484,781 binding points with zero loss and 45 actual waits. Scheduling-dependent counts differ from the analyzed earlier capture and are not combined. Cold rlib and linked consumer bytes matched. Incremental payload/consumer gates passed with the documented OFF/OFF link-metadata instability retained.

Final hosted regression run `20261014-180011428-4084443` reported 602 passed and 3 ignored. It first encountered an unrelated parallel `Text file busy` failure in `executable_with_missing_elf_loader_is_rejected_without_compiling`, then passed that test when the runner retried it alone. This was not a clean first-attempt suite. Hosted clippy `20261014-180036654-4087415` and format/diff checks passed. The collector's focused hosted graph gate passed 150 tests, with default-limit and non-promotion regressions.

## Next measurement needed

The next useful instrumentation is disjoint per-thread CPU segments at actual task enter, fork suspension, continuation resume, task drop, and dispatch suspension/restoration. It needs explicit thread/clock identity, read-failure handling, and shared clock alignment with query events. Parent helping work must not overlap child attribution. Separately classify remaining opaque ownership and previous-session/non-query producer provenance without inferring edges.

Only after those gates pass should we compute a weighted causal span for the observed scope, identify the longest dependency chains, and evaluate graph-changing architectural alternatives. Report both work reduction and span reduction. The current capture does not answer whether one build can use 96 cores effectively.

## Interpretation limits

- A million-record mode fits the chosen small real workload, not the earlier roughly 30-million-event contention workload.
- Elapsed wait overlap is not CPU work. Registered-to-notified time and notified-to-resumed time describe different stages and must not be added across overlapping waits as wall time.
- Query and pool timestamps have separate epochs. No cross-epoch weighted critical path is claimed.
- Generic dispatched work must remain unbound unless an actual instrumented ownership relationship exists.
- Frontend source-level architectural redesign can change both total work and the graph. A measured critical path of the current design would be a diagnostic, not a fundamental language lower bound.
