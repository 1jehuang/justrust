# Mapping feedback closure and whole-result recheck, October 15, 2026

This records what the finished mapping actually did for the request to map dependencies and locate parallelization bottlenecks. It is an observed diagnostic improvement, not a compile-time speedup or a complete compiler dependency DAG.

## Concrete improvement on identical original inputs

The [same-original public coverage verification](../bench/results/20261015-mapping-coverage-public-acceptance.json) invoked the installed public CLI with query CPU, task DAG and readiness flags together, then independently checked task identities/edges/weights, every named query owner, the flow maximum-chain witness and CPU conservation against the original producer bytes.

| Same original cohort | CPU segments attachable to task DAG | All retained CPU segments assigned by invocation flow | Task-attached CPU, ns | All flow-assigned CPU, ns | Additional task-null CPU represented, ns |
|---|---:|---:|---:|---:|---:|
| Cold | 4,352 | 36,736 | 53,521,730 | 513,291,338 | 459,769,608 |
| Warm | 3,296 | 3,659 | 32,094,521 | 34,641,187 | 2,546,666 |
| Cold with dispatch | 4,278 | 36,934 | 59,784,609 | 522,313,801 | 462,529,192 |

For cold, the additional 32,384 represented segments are exactly 454,937,150 ns of query-owned CPU plus 4,832,458 ns of opaque CPU. They were deliberately excluded from the task-only graph because they had no genuine pool task, not because their measurements disappeared. Invocation scopes now attach them without inventing pool ancestry. All three partitions conserve the original CPU mass exactly, with zero unassigned retained segments. The existing task DAG's facts remain unchanged.

This is a direct answer to whether the mapping is better: it connects and names substantially more of the **same measured execution** and adds independently qualified latch handoffs. It does not say the compiler got faster. Six deliberately corrupted public results were rejected, including changed task weights, excluded mass, flow mass, query identity/name, witness labels and duplicate task identity.

## Final whole-result rerun

After all implementation and verifier corrections were pushed, the final frozen client and coherent compiler were exercised again without changing source, native metadata or installed binaries:

- Full 14-test harness: **14 passed, zero skipped, 508.147 seconds**, comprising seven actual-source tests and seven safety tests. It rebuilt and executed cold, warm incremental, dispatch, timing, query CPU, flow-v1 and readiness workloads, not merely parsed old fixtures.
- Combined offline mapping/oracle/control suites: **139 passed** in 0.165 seconds.
- Hosted check `20261015-074627086-1866661`, full test `20261015-074812985-1877054` (**709 passed, zero failed, 3 ignored**) and strict all-target clippy `20261015-074825060-1866628`: all passed. `justrust show` confirms remote on hosted. Exact run summaries are in `data/runs/mapping-recheck-20261015/`.
- [Fresh-capture public readiness replay](../bench/results/mapping-recheck-20261015/readiness-public.json): **34 cases passed** in 193.7 seconds. All 130 original files and the installed client were unchanged. The nonempty cold authority controls started from 154 actual matched encounters.
- [Fresh-capture public flow-v1 replay](../bench/results/mapping-recheck-20261015/flow-public.json): **25 cases passed** in 113.3 seconds. All 135 original files and the installed client were unchanged.
- [Fresh same-input coverage comparison](../bench/results/mapping-recheck-20261015/mapping-coverage-public.json): **3 combined public CLI cases passed** in 36.00 seconds. Fresh cold represented 36,862 retained segments / 677,724,779 ns in invocation flow versus 4,289 segments / 71,784,516 ns attached to tasks. The exact additional 32,573 segments / 605,940,263 ns comprise 600,688,890 ns query CPU and 5,251,373 ns opaque CPU. This independently repeats the same-byte information gain on newly compiled inputs.
- [Six fresh public-output corruption controls](../bench/results/mapping-recheck-20261015/coverage-mutation-controls.json): every changed task weight, excluded mass, flow mass, query identity, witness label and duplicate task was rejected against the untouched original warm capture.
- Previously accepted flow-v1 original-byte replay: **25 public cases passed** on the final installed client.
- [Genuine native retained failure/race packets](../bench/results/mapping-recheck-20261015/retained-failures.json): **16 public cases passed** on the final installed client, with all four original packets and the client unchanged.

The long check invocation spent 103.718 seconds synchronizing and 1.3 seconds in Cargo. The full test's slowest test took 9.98 seconds (`public_publish_fetch_allows_bundle_above_worker_cap`). These are observed validation costs, not claimed mapping speedups or reasons to bypass hosted execution.

## Requirement-to-check-to-observation matrix

| Requirement / changed public output | Whole-result check | Observed result |
|---|---|---|
| Preserve Rust compilation, linking, diagnostics and incremental behavior while recording | Actual-source OFF/ON/OFF compile/link/run and archive/payload checks in the full 14-test harness | All seven real-source tests passed; cold archives and supported warm payload comparisons passed, and linked consumer behavior stayed correct. Randomized `lib.rmeta-link` bytes remain explicitly excluded from byte-equality claims. |
| Map genuine native task prerequisites | Public `--observed-task-dag`, independent `native_task_oracle` and same-input coverage verifier | Exact task/node/edge/weight inventory passed for all three same-input cohorts: cold 11 tasks/12 edges, warm 14/16, mixed 11/12. Task-null CPU remains excluded from task weights rather than fabricated into tasks. |
| Attribute exclusive query CPU to actual admitted owners, not inherited worker TLS | Public `--query-cpu-ownership`, independent admission/timing/clock qualification and exact original query rows | Exact query context/job/name/cost ordering passed, with 18,199 cold and mixed named owners and 1,833 warm owners. Query sums reconcile with flow query CPU. |
| Connect actual invocation flow and fork/join control, including repeated query jobs | Fresh real flow-v1 captures, public flow flag, original scope/segment/edge oracle and maximum-chain witness check | Fresh native harness passed: 18,199 cold scopes and 1,838 warm scopes, with every retained CPU segment assigned; public replay status is recorded above. No whole-job contraction or worker chronological edges are substituted. |
| Qualify actual latch handoffs rather than infer completion/publication edges | Fresh readiness captures, original wait lifecycle and exact row-bound two-obligation oracle | Fresh cold recorded 154 encounters, including 148 registered and 6 already-ready, all qualified. Fresh mixed recorded 150, including 139 registered and 11 already-ready, all qualified. Warm recorded zero. Earlier accepted cold's real one-unbound race remains retained and incomplete, not rewritten by this second observation. |
| Localize bottlenecks using real chain costs, not inclusive category sums | Actual analyzer named-chain ranking checked against original CPU segments and validated native witness | Fresh cold known chain 37.560985 ms includes `typeck_root` 32.316304 ms. Warm known chain 15.988678 ms includes `resolver_for_lowering_raw` 5.989231 ms and `lower_to_hir` 2.538333 ms. These repeat the qualitative cold type-checking / warm resolver-HIR localization, without claiming stable serial costs. |
| Keep explicit feature admission and older public behavior | Default/old-flag refusal, independent/combined flag equality, fresh root5/root6 handling and legacy replay | Existing whole-result native tests and 25 legacy public cases passed; fresh public cases are recorded above. Pool6 is not admitted by old CPU/flow flags. Additive flags do not fabricate a v1 projection for v2. |
| Public analyzer must preserve complete authoritative graph output safely | Actual analyzer invocations, original-byte neutral snapshots, exact full stdout artifact equality, mode/hash/size and collision refusal | The full native harness passed actual analyzer checks for cold/warm/mixed. Fresh independent public checks are recorded above; no fields or resource attachments are stripped to manufacture equality. |
| Missing, mismatched or colliding authority must not invent query costs, labels or readiness transfers | Copied nonempty-cold missing-query, clock-mismatch, duplicate-pool and cross-producer controls | All four copied cold authority controls passed: missing/mismatched query authority withheld every query weight, query name and qualified readiness transfer while retaining only independently valid nonquery CPU. Duplicate/cross-producer identity controls suppressed causal authority. Controls require a positive original wait/transfer inventory, avoiding vacuous warm-zero assertions. |
| Honest native failure/loss must remain partial, not reject or invent success | Four exact native guard packets covering unbound races, cycle/poison/unwind, active unwind and registered-null wait ID; 16 installed-CLI cases | Readiness mode accepted all four as typed partial; all 12 old-mode admissions refused. Missing original query authority yielded zero qualified readiness transfers, and no failure row acquired success edges. |
| Never claim full span, useful-thread ceiling or speedup from incomplete facts | Every public assessment/oracle asserts null full work/span/parallelism and false full-pipeline/flow completeness | Fresh native checks passed; retained/legacy public checks passed. Visibility, wake-to-consume, cache publication, I/O, scheduler and pre-attach gaps remain unknown. A2 defects/noise still block a paired performance claim. |

## Scope and artifact identity

Frozen installed client SHA256: `c477b59037551e26babe623103e1d52449717c3789c04917089798e1bfa203e0`. It is the same client used by the accepted milestone, the new same-input coverage comparison and this final whole-result rerun. Native implementation and runtime/private metadata hashes are recorded in [the original acceptance](latch-readiness-20261015.md).

The fresh captures have different observed CPU costs and wait counts from the earlier captures. These are separate instrumented executions, not matched performance samples. The same-input coverage table above is the only numerical before/after diagnostic-coverage comparison here. Full real-world parallelism remains an open engineering target, not a completed theorem.
