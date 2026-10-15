# Dispatch result availability: correctness results, 2026-10-15

## Status and scope

Implemented and correctness-validated on actual Rust source. The comprehensive default-disabled performance comparison is still pending. This is **not** a complete semantic dependency graph, an execution DAG, a critical path, or a useful-thread ceiling.

The design and measurement requirements are in `dispatch-result-provenance-plan-20261015.md`. Consumer commits are `f11a53fb5` and `1cf98db01`; native implementation and real-source acceptance are `8b7398717`. The installed client SHA-256 is `e346c429d0373a4341314e375cd403611755a8c1f43cee61ad3f674ffa1d9666`.

The explicit `justrust graph --dispatch-result-provenance` analysis accepts the new root-6/read-2 capture. Old strict/resource collector APIs intentionally do not accept root 6. Both cache-call and dispatch-result flags can be requested together, but their metrics and event relations remain separate. No resource attachment authority is inferred merely by adding the new root version.

## What was added

An opt-in nonincremental vtable entry retains the exact optional result index already returned by the existing scheduling algorithm. The actual lexical request records it at normal return, after any result conversion. The original execution function pointer ABI and the scheduling algorithm remain unchanged. This preserves behavior, not proof of zero default-path cost: the extra vtable field and selection branch still require measurement.

The terminal metadata says only `resolved_dispatch`. It cannot distinguish fresh provider execution, a cache recheck, or obtaining a waited-for cached result. A null index stays unresolved. Incremental dispatch, first-cache-hit paths and feeds do not acquire this metadata in this slice. Interrupted requests do not become successful normal returns.

The relation connects a unique actual prior result publication to the matching request terminal, with exact context, admitted endpoints and original stream positions. Publication may occur after request start. A producer completion marker need not precede consumption. A normal return can carry an error-valued result, so certification does not mean that the overall Rust program compiled successfully.

## Correctness gates

- Hosted check `20261014-190255435-340799` passed warning-free.
- Six public integration tests passed in `20261014-190300362-341711`, including explicit root/version gates and a real resource-scanner path that keeps root-6 resource authority incomplete.
- Full hosted test `20261014-190307525-343324`: **644 passed, three ignored, zero failed**, first attempt, 11 binaries. `justrust show` verified remote on hosted.
- Hosted clippy `20261014-190329579-347280` passed warning-free. Formatting and diff checks passed.
- Native compiler/session/thread-pool suites: **223 passed, 28 ignored, zero failed** in primary binaries. Nested child self-tests are not counted twice. All seven new native recorder/default-compatibility tests passed.
- Selected `tests/run-make/short-ice`: one passed, zero failed, 542 other run-make tests filtered. This was not the whole run-make suite.
- Actual-source suite: **six tests passed, no skips**, 159.107 seconds. This is correctness-test elapsed, not a compile speed comparison.

The actual suite uses untouched locked `itoa 1.0.18` source and real emitted rlibs, compiler threads, linking and executed consumers. Fixture controls arrive through owned fd198 and are consumed and closed before compiler entry. Legacy two-line controls and timing's three-line controls remain unchanged; dispatch is a distinct explicit fourth line. Compiler argv, environment, stdout and stderr match OFF/ON controls.

Within the candidate, cold OFF/ON/OFF whole archives, object payloads and linked/executed consumers matched exactly at one and four threads. Warm rmeta/object payloads and linked/executed consumers matched exactly. Warm whole archives did **not** match, also in independent OFF/OFF controls, consistent with the pre-existing randomized incremental link-metadata names. No warm whole-archive identity claim is made.

Old root-4 and root-5 original captures and frozen default public assessments replayed exactly. All original replay input hashes stayed unchanged. Native source-before/source-after hashes matched across the full tests.

## Actual new-mode observations

| Capture | Nonnull returned dispatch indices | Certified dispatch relations | First-cache requests |
|---|---:|---:|---:|
| Cold, one thread | 18,199 | 17,569 | 211,925 |
| Cold, four threads | 18,297 | 17,669 | 212,827 |
| Warm incremental, one thread | 0 | 0 | 5,713 |
| Warm incremental, four threads | 0 | 0 | 5,713 |
| Cold, four threads plus timing | 18,231 | 17,603 | 212,927 |

Every certified row was independently checked against original request start, terminal index, context, actual returned index, unique publication before terminal and admitted endpoints. First-cache requests retained null dispatch metadata. All new-mode read/binding inventories were loss-free; ordinary default 100,000-row cold captures still honestly report their existing truncation.

Counts varying between serial and parallel runs do not classify individual routes. For example, subtracting admitted jobs from dispatch resolutions does not establish a count of blocking waits or cache races. Warm zeroes describe this deliberately nonincremental-only feature, not an absence of incremental query work.

The mixed timing capture retained 379 measured exclusive CPU segments and a reciprocal shared-clock bridge. Its scoped CPU sum is not comparable to earlier correctness cohorts as an overhead benchmark: workloads were not run under the matched performance protocol and other correctness jobs could run concurrently. Full CPU work, span and theoretical parallelism remain unknown.

## Retained evidence

Immutable candidate runtime plus matching private metadata:

`/home/jeremy/.jcode/scratch/dispatch-result-native-accepted-20261015/`

Its launcher SHA-256 is `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`. The actual driver SHA-256 is `8553d8b5e4d498c451f7cc8ef3f41a50cd91b538e40a1a692d7b474662d67223`. The launcher is identical to the baseline while the driver differs, demonstrating why launcher or version identity alone is insufficient.

Native logs and unchanged source manifests:

`/home/jeremy/.jcode/scratch/native-dispatch-result-tests-20261015/{full.log,short-ice.log,source-before.sha256,source-after.sha256}`

Fresh actual-source evidence root:

`/home/jeremy/.jcode/scratch/dispatch-result-real-final-20261015/`

New-mode cohort `frontend-real-itoa-1ov3ms2y` retains `dispatch-cold-t1-1`, `dispatch-cold-t4-1`, `dispatch-warm-t1-1`, `dispatch-warm-t4-1`, and `dispatch-with-timing` capture directories, public graph outputs, oracle results and artifact/member/consumer hashes. Original failed attempts are retained rather than overwritten.

Five compact reports are in `bench/results/dispatch-result-provenance-20261015/{cold-t1,cold-t4,warm-t1,warm-t4,with-timing}.json`. Each records exact original input hashes, full external graph-output SHA-256/byte count, separate cache-call and dispatch-result counters, unresolved reasons and explicit unknown full-pipeline CPU/span/parallelism. On the same cold four-thread capture, cache-call certification covers 153,723 requests and dispatch-result certification covers 17,669 of 231,444 observed requests. These are availability observations, not a semantic dependency-coverage percentage or a parallelism bound.

## Performance gate still open

The baseline compiler and matched rustdoc/clippy helpers were preserved before candidate native edits. The first privately registered benchmark prefix was incomplete: its standard-library artifacts were omitted during sysroot assembly. Its first five preparation builds failed with E0463 before any measured samples, and the run was stopped with exit 130. Original logs and partial results remain retained under `dispatch-synth40/A`; they are not a performance baseline. The run also observed unrelated compiler activity, so it was not a quiet measurement.

A fresh `A-recovered` prefix was assembled from the original baseline prefix and 41 missing std/runtime files from the matching immutable accepted timing snapshot, without borrowing candidate libraries or mutating the original evidence. The donor/destination recovery inventory SHA-256 is `5b43d9de001fff1d56423d82cb1657074a7502735dc4386f452d77bbac50cf75`. Both registered prefixes passed an actual `std::collections::BTreeMap` compile, link and executed-output smoke test, and selected sysroot paths matched their registrations. These are preparation gates, not measured workload results. The intended comparison uses isolated homes, caches and targets, a single pinned client, the same donor Cargo and explicit attested compiler/driver/helper selections. Natural IDs are `jr-ad23ba6b2733` and `jr-c515f6b3476d`; they are not forcibly renamed to conceal different builds.

The new benchmark identity protocol checks declared prefix files and recorded compiler/Cargo selection before and after every invocation, including preparation calls. It does not prove loaded process memory, and hashing warms OS page cache. Both compared runs must use the same attestation protocol. A cold target directory is not a cold OS-cache measurement.

Matched full seed-1, 40-point synthetic runs and `bench/suite.py compare` remain pending a fresh run with the recovered baseline. No speedup, zero overhead, default-path nonregression, default compiler promotion or measured 96-core speedup is claimed before valid results exist and their command/change/noise breakdowns are inspected.
