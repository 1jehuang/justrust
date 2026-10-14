# Native selected admission: requirement/check matrix, 2026-10-14

**Status: PASS, scoped native admission acceptance.** Final source-aligned task 853801iecj passed selected six, legacy nine, actual native crate suites and final source hashes. Default fixed-fixture ABBA control completed separately. This is not full graph authority, a speed/no-overhead claim, or default promotion. Earlier failures remain explicit below. Only the two owned evidence files were edited by this worker.

## Scope and evidence boundary

Programmatic factory selection in rustc_interface/interface.rs and util.rs installs FrontendAdmission before workers start. Its unit and contextual pairs retain one Arc<TicketedProxy>. Pool callbacks use paired contextual authority or unit fallback. Observer identity uses checked namespaces and immutable attachment binding, without new wire fields. Recording ON and OFF deliberately have different diagnostic carriers, qualified versus unit facts, while retaining the same selected TicketedProxy policy and event capacity zero. Equality of diagnostic facts is not an acceptance requirement.

No full mapping, resource wire, default promotion, or speed claim is authorized. Callback panic is not blocked-query panic coverage. Exhaustion fallback evidence from unit tests and the extracted latch with Span/QueryCycle stand-ins must not be labeled actual native compiler exhaustion.

## Requirement/check matrix

| ID | Requirement | Check | Result |
|---|---|---|---|
| explicit-selection | Config factory is selected programmatically before frontend workers start, default remains legacy, serial does not invoke factory | interface.rs/util.rs and smoke default/serial fixtures | PASS, scoped |
| same-controller | Unit and contextual pairs retain the same Arc<TicketedProxy>; no diagnostic carrier selects a policy | frontend_admission.rs inspection and native rustc_data_structures tests | PASS, scoped |
| qualified-pair | Release/reacquire use identical qualified wait identity and expected attachment, wrong/missing context falls back to unit pair | registry.rs/admission_dispatch_tests.rs, observer binding tests, actual selected fixture | PASS, scoped |
| actual-pending | Actual duplicate typeck query releases allowance; completed producer parks outside locks; reacquire becomes real TicketedProxy Pending; no return before native supply; grants/acks advance after supply | test_selected_actual_pending_and_macro_process_neutrality controller facts | PASS, scoped |
| carrier-not-policy | Recording ON observes qualified context; OFF observes unit facts; both explicitly select TicketedProxy eventcap0 | recording-off/on controller evidence; intentionally different diagnostic facts | PASS, scoped |
| neutral-artifact-process | Identical command/environment, executable bytes and proc-macro argv/environment output, no private mode arg | comparison.json and execute-off/on records | PASS, scoped |
| diagnostics | Selected-policy type error has matching nonzero exit and diagnostics after stripping controller facts | test_type_error_preserves_diagnostics_with_selected_policy | PASS, scoped |
| foreign-fd | Foreign reserved fd fails open and remains intact | test_foreign_reserved_descriptor_fails_open | PASS, scoped |
| callback-unwind | Callback panic remains nonzero and capture incomplete, not blocked-query panic coverage | test_smoke_callback_panic_unwinds | PASS, scoped |
| unchanged-wire | Existing reserved-fd-v2 transport, query schema4 and pool schema2 correlate; no resource receipt, full_pipeline_complete false | inspect_receipts and smoke receipts | PASS, scoped |
| exhaustion-boundary | Checked namespace/wait exhaustion falls back without ID reuse; representative extracted latch is not actual native compiler exhaustion | native session unit tests plus reported 51 representative extracted latch tests | PASS, scoped |
| legacy-regression | Query/wait/task-binding/compiler/compiler-argv gates remain passing | query.log, wait.log, task_binding.log, compiler.log, compiler_argv.log | PASS, scoped |
| frozen-native-crates | Native data structures/thread pool/session crate tests pass against same source cohort | main script x.py test and final source sha256 check | PASS, scoped |

## Reported supporting results, not independently rerun

- Root018542e0h3 reports fmt/check, 559 tests, strict all-target lint PASS: 20261014-144019067-2003317 / 20261014-144020738-2003637 / 20261014-144032083-2003146. These do not establish native selected acceptance.
- Tiger reports 21 tests and strict lint PASS: 20261014-144112437-2012317 / 20261014-144113346-2012316.
- Spider reports no ordinary blocker and unchanged default methods. Hooks are outside ProxyState only, not necessarily outside query latch or Sleep locks. Hooks must not allocate, publish, do I/O, take locks, or try_lock. The fixture uses preallocated TLS Cell and atomic facts only.
- Poolwhale reports 106 hosted tests with 26 ignored, frozen strict lint: 20261014-143806667-1979505 / 20261014-143810229-1980586. Focused seven tests: 20261014-143805089-1979412. Ignored tests are not passes.
- Queryturtle reports 53 actual-module tests and 51 representative extracted-latch tests with Span/QueryCycle stand-ins, including exhaustion fallback (representative run 20261014-143808614-1980060). This does not establish actual native compiler exhaustion.

## Initial cohort identifiers (superseded)

These script, harness, fixture and source hashes identify the initial attempt only, not the final cohort. Final cohort identifiers are in Final source-aligned acceptance below.

Initially inspected repository HEAD: `545557a6077be358c25ea005e01e6049d6d46d47`. This is not a clean-tree identifier.

Immutable baseline driver: `925c7892da0e7f00706326c9a77a3a20d887cf57706529b19f58e54575274bf9` in scratch/native-selected-admission-baseline-20261014.

Gate script SHA256: `f54fcaae42770aeccda36b1f0316bda23360272d0d2511c59fba9eb9bfdbff47`.
Selected harness SHA256: `0a3704b0093cfe32a9c782ac3bc556b9e7a7bdb0c954be163827cfc38671b3ed`.
Controller fixture SHA256: `ac274851a39cbe62fdd8fdcba53073206b2fb2e783dd1c2d6c4a61b2e3a02b13`.
Smoke fixture SHA256: `ceef8e864220adc1b2cc513997d87815a638d773f53e558589fe68a8bea85643`.

Frozen source manifest read from selected-admission-native-20261014/source-before.sha256:

```text
f1e0541f6875226c258f4041ee3d4935fe4abc5b4019741b84e6e586996aebe6  toolchain/rustc/compiler/rustc_data_structures/src/jobserver.rs
f3849e80b7a1f72ee9472592d66a57a83477900d52354a772ac345c77f07def8  toolchain/rustc/compiler/rustc_data_structures/src/jobserver_ticketed.rs
b7f6f374cff0ff0dbf6e1cea8dce1f7080133cad306fbfb741f5a6c96b87510a  toolchain/rustc/compiler/rustc_data_structures/src/frontend_admission.rs
4861677858cf2603fb06fea75fbd755ddd244e7317a90740ebe4d5dee7210617  toolchain/rustc/compiler/rustc_interface/src/interface.rs
cc07809ed6a0ef9d63f0324e8e187c0872cb47bfcd04322a8f070f6d4b8899eb  toolchain/rustc/compiler/rustc_interface/src/util.rs
61542b53e47531be4600f156b3a9c670ac3a0db0d2bf5bcb84092df7576c0489  toolchain/rustc/compiler/rustc_driver_impl/src/lib.rs
e8ee13fbcdf12030a1a0b61cef7679b9d79a0a4625533ebbe91362455fd94081  toolchain/rustc/compiler/rustc_query_impl/src/execution.rs
87cf4d4adfa7cdbaf9586d79ec0385ca99c3c4f6f154e97ecd958b06b1c331e9  toolchain/rustc/compiler/rustc_session/src/query_admission.rs
1f9a50f8b010ec16c250053e19ee024f78ed0a624571a17aff7572ae7bb16e52  toolchain/rustc/compiler/rustc_thread_pool/src/admission.rs
014f5e9793e943227c1edc841efd57c2bd5f1df616b6e3c61ac2d36ab989f22b  toolchain/rustc/compiler/rustc_thread_pool/src/admission_dispatch_tests.rs
a0097e5e7f4fd6ff933e778564082b1574c4c445d2d7a6a247046a19dfeb9fa3  toolchain/rustc/compiler/rustc_thread_pool/src/lib.rs
d463edd0c618aef5aadd1baba39ead0dc877ccd70fc076cf2479932b5b29addb  toolchain/rustc/compiler/rustc_thread_pool/src/registry.rs

```

## Initial native attempt: FAILED, retained

Main task 113865w9vm reports compiler build success in 7m11s and overall pipeline failure after 496.6s. Independently inspected selected.log reports **6 tests, 3 passed, 3 failed, 0 skipped, 64.856s**. Passing tests were default, serial factory-not-called, and callback-panic smokes. Failed tests were actual Pending/process-neutrality, foreign-fd, and type-error checks. All three encountered the controller native client available() assertion, left 14 versus expected 0, before producer query completion, then insufficientexercise. No successful actual Pending/supply acceptance is established by this attempt.

The coordinator reports actual native client capacity 16 rather than fixture-assumed capacity 2. This is a fixture-capacity diagnosis, not an established native protocol defect. Parrot owns a fixture-only correction holding real native Acquired surplus tokens before workers start, without fake capacity. The failed initial records must remain retained and distinguished from any rerun.

Retained initial selected.log SHA256: `64a49d5142a738aebaefa3860142615961f3d7b1189bbd7d3cd5ce3f69157368`. Rebuilt stage1 rustc SHA256: `95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba`. Rebuilt driver SHA256: `2c049aed60aa1edda713cf79c19d679a8496e957d698aa0811b9d4405231e6db`. All 12 frozen source checks independently passed again after this attempt. At that stage, legacy nine were running separately as task 638142k8uo and native crate tests were pending. These gates were later resolved by final task 853801iecj.

## Earlier corrected selected retry and legacy gates, before final cohort

Hare reports corrected selected suite **6 PASS in 4.851s** under scratch/native-selected-retry-final-20261014. Independently inspected comparison.json confirms OFF and ON actual query release, producer completion/parking, query-ready reacquire, real Pending, returned_before_supply=false, returned_after_supply=true, event_capacity=0. Both reserve 14 real native Acquired surplus tokens. Grants and acknowledgments advance from 3 to 4. OFF is unqualified, ON qualified. Executable SHA256 is `9019ae0368157341e3bcc8a05d3f2f13b4ddd2d916c0bbc3d68bde5ac1404f61`, with identical environment and observed proc-macro output. All six private-artifact manifests identify unchanged driver `2c049aed60aa1edda713cf79c19d679a8496e957d698aa0811b9d4405231e6db`. Corrected fixture SHA256: `b837c4f0f92cf341ffe1978dedc374de2a377243b896bbf64977601d1fb5db81`. Corrected harness SHA256: `fc87b68e39318996869876762eec3c2bf3f6d1b21dc403067baa4a705662c684`. Earlier hashes above identify the initial attempt, not this corrected retry.

An intermediate full retry is reported to have failed a foreign-descriptor payload-length assertion, pread 26 versus 27 bytes, fixed to len(payload). Exact retained case is `$SCRATCH/native-selected-retry-full-20261014/native-selected-admission-zh56hx5b`. Independently inspected foreign-fd.json exits zero with SHA256 `dabe8e748b784658da79ca5e4c2342eddf91526b024af46c7b99af9c2f32ab97`: the native foreign-fd fixture passed. The failure was the subsequent Python parent assertion in retained harness-source.py line299, reading 26 bytes against a 27-byte payload. The suite failure was visible in retained foreground tool output, not a separate suite stderr file. This was a harness assertion bug, not a failing native foreign-fd result. Preserve this failed attempt too.

Independently inspected legacy logs pass nine tests: query 1, wait 4, task_binding 1, compiler 2, compiler_argv 1. Main task 638142k8uo reports 49.58s overall. These are correctness facts, not speed measurements. At that stage, actual three native crate tests were running as task 738655l84x and ABBA controls were planned afterward. Those historical blockers were resolved by final task 853801iecj and control task 023050nddl.

## Native crate failure, test-only correction, and final source-aligned rerun

Hare reports first actual three-crate task 738655l84x failed the compiler internal `rustc::default_hash_types` lint on a test-only HashSet in pool admission.rs. Standalone clippy did not catch it. Main changed only cfg(test) uniqueness checking to Vec clone/sort_unstable/dedup. Current code was inspected. The admission.rs hash therefore changed from `1f9a50f8b010ec16c250053e19ee024f78ed0a624571a17aff7572ae7bb16e52` to `1ac5e63ea13e51f1439a2954741546f31e8ee810b8621a4730b0b1948a144855`. The original manifest above remains an initial-cohort record, not the final source manifest. No production-code change is reported.

At that stage, retry 822511an4g was reported PASS in 26.26s: data structures 139, pool 106 with 26 ignored, session count not yet inspected, integration and doctests passed. That report alone did not finalize acceptance. Main then ran final task 853801iecj with a fresh source-aligned cached assembly, selected six, legacy nine, and three native crates under scratch/selected-admission-native-final-20261014. That final pass and subsequent ABBA task 023050nddl resolved the freeze blockers, as detailed below. The separately owned exported 11-run 2026-10.jsonl was not edited by this worker.

## Final source-aligned acceptance

Task **853801iecj PASS, 158.80s**. Selected suite **6 PASS, 4.199s**, legacy **9 PASS**. Actual native crate unit suites: data structures **139 PASS**, session **69 PASS**, thread pool **106 PASS / 26 ignored**. Two nested session protocol subprocess invocations each pass one filtered test and are not additional session unit tests. Integration suite pass counts are 1, 1, 1, 3, 1. Doctest suite results are 1 pass/2 ignored, 5 pass/1 ignored, 1 pass/0 ignored, 27 pass/0 ignored. No failing suite. All 12 final source manifest hashes independently match. Full final task output was read to obtain exact counts.

Final launcher SHA256: `95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba`. Final driver SHA256: `f31f9744df6448383327439ad6875bc09531d3b4030cf5b8e934bb80dc1d56c6`. Final admission.rs SHA256: `1ac5e63ea13e51f1439a2954741546f31e8ee810b8621a4730b0b1948a144855`. Final selected evidence is $SCRATCH/selected-admission-native-final-20261014/selected/native-selected-admission-fbslf3n1/comparison.json. OFF/ON both demonstrate real release, completed/parked producer, real Pending and no return until native supply. Grants and acknowledgments advance 3 to 4, real surplus reserve is 14, eventcap0 stays disabled, OFF unit facts versus ON qualified facts. Artifact SHA256 `bcbad7a28882a9133b62e3ad75d0e145927f6c7620685071f39af22aba4ca3cf` matches OFF/ON, as do proc-macro output and environment. Final source/log hashes and receipt hashes are in native-acceptance.json.

## Public consumer boundary

Actual `justrust graph --json` on the final selected capture exits zero. Exact parsed output is embedded in native-acceptance.json with a path-redacted command. Query schema4 is structural_complete=true, wait_structural_complete=true, one wait, read record/endpoints complete and loss0 but semantic_complete=false. Task binding records/endpoints/validity are true, expected30001/bound2422/unbound27579/loss0, while all_targets_bound and graph_binding_complete remain false. Pool schema2 is structural_complete=true with 12 nodes/12 edges. Entry schema1 is structural_complete=false because --compiler-manifest reconciliation was not supplied, leaving compiler-launch-authority opaque. CPU and span remain null and full_pipeline_complete=false everywhere. In-memory qualified context is deliberately not serialized. This does not establish resource authority or complete mapping.

## Default fixed-fixture ABBA control, not a performance claim

Task 023050nddl PASS, 19.7s. Committed evidence bench/results/native-selected-admission-20261014/paired.json SHA256 `5566cbf60adba4d03249d911d121c5e2436bbe9c79b8cbdb7b8768c0e54d6869`. Four ABBA rounds, 64 samples, eight samples per compiler/thread/capture cell. Baseline is immutable driver925c7892..., candidate is final driverf31f9744.... Medians before/after (ms):

| Threads | Capture | Before | After |
|---|---|---:|---:|
| 1 | OFF | 129.680 | 126.132 |
| 1 | ON | 250.791 | 219.553 |
| 4 | OFF | 93.872 | 93.361 |
| 4 | ON | 211.351 | 198.412 |

Quiet provenance is unknown. Timing followed native work (reported gate end21:56:52, timing start21:57:03), not overlapping bootstrap. Captured reads lose approximately 1.0 million events and task bindings approximately 0.5 million, with records/all-target completeness false. Admission loss is zero. This measures capped default capture cost on one fixture, not complete graph construction or selected-policy performance. All 64 executions have identical output, and artifact bytes are stable within each compiler/thread across capture OFF/ON. Artifacts need not match between compilers. No speed, no-overhead, general performance, rustdoc/clippy coverage, broad suite, or default-promotion claim is made.

## Retained failure and audit trail

Retain initial capacity-assumption failure (6/3/3), intermediate foreign-payload-length failure, native internal lint failure, and adapter manifest setup failure `20261014-143523131-1953212` (exit101, zero tests passed). The last is present among exactly 11 exported supporting records in the separately owned 2026-10.jsonl. The intermediate raw native result and buggy parent assertion are retained at the exact path above; the parent suite failure remains in foreground tool history. Native lint failure remains in task738655l84x history. These are not erased by the final pass. Final acceptance is limited to this source-aligned correctness cohort, with actual compiler exhaustion, blocked-query panic, full graph/resource authority, and performance promotion explicitly unclaimed.
