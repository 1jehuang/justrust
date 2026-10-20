# Dispatch result availability: correctness results, 2026-10-15

## Status and scope

Implemented and correctness-validated on actual Rust source. The comprehensive default-disabled performance comparison is **blocked**: the completed 40-point A2 baseline failed eligibility because 39 points were noisy and seven had unexpected failing-test outcomes. B2 was not run. This is **not** a complete semantic dependency graph, an execution DAG, a critical path, or a useful-thread ceiling.

The per-requirement checks, observed information gain and explicit acceptance boundaries are mapped in `dispatch-acceptance-trace-20261015.md`. An independent recheck of the installed public CLI passed eight cases on unchanged original real-build inputs; its compact output is `bench/results/dispatch-result-provenance-20261015/public-cli-acceptance.json`. This is a consumer replay, not a new compiler benchmark.

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

The recovered baseline attempt also stopped with exit 130 before measured samples: its first pinned-workspace preparation `20261014-195647508-733465214` failed with E0463 for `proc_macro`, despite the std-only smoke test passing. The full original error log is retained under `dispatch-synth40/A-recovered/versus/logs/synth-suite-jr-ad23ba6b2733-pre-20261014-195647508-733465214.log`. Slim, path-redacted summaries of all six failed preparation runs, including original baseline run `20261014-193131390-502779134`, are committed in `data/runs/dispatch-benchmark-failures/2026-10.jsonl`. That recovered attempt recorded two foreign `rustc` entries and 7.1 other cores. The pinned recorder does not distinguish zombie entries, so its raw count is not proof of two live compilers. The genuine other-CPU observation still prevents calling this an uncontended result. All raw observations are preserved.

Earlier immutable snapshots contain files with the expected proc-macro names, but all 41 Rust std/runtime artifact bytes differ from the accepted timing baseline. Those are not certified matching donors, and neither candidate nor stage0 libraries were grafted onto the baseline. Reconstruction therefore uses new isolated coherent whole-toolchain source snapshots: baseline `884fd7ce705b20bc952469014b656113d43f7a26` and candidate `8b7398717f9d9313d5101c948f2ce8dc223ed5bf`. Both must explicitly build the entire `library` plus compiler/rustdoc/clippy using identical pinned bootstrap inputs and options, without keep-stage shortcuts. This is new paired evidence, not exact reproduction of the earlier accepted binary. A native proc-macro compile/load smoke and actual pinned-workspace preparation on both sides are required before another 40-point attempt.

The coherent A2/B2 full builds, native proc-macro compile/load consumers, std/libtest execution and actual pinned Desktop release preparations now pass on both sides. The exact source, cached bootstrap inputs, generated configuration, full-prefix inventories, six declared artifact identities and per-invocation metadata audits are recorded in `bench/results/native-dispatch-reconstruction-20261015.json`; redacted setup runs are in `data/runs/xps14-dispatch-setup/unknown.jsonl`. Both configurations have SHA-256 `e149e3a769b6f75028de03990ee6185ed15c9a22aa309047731315163ffc5a0f`. Each archive retains 66,682 unchanged inputs and 96 unchanged lockfiles. These are setup/correctness gates, not a comparison of compile speed.

The actual reconstructed driver hashes are A2 `1d7b6e73ac397ddcbc6fac4ca08040c95146b731c2fa8a623f8546ba3ec698cf` and B2 `1b9358beb897ec29e88a6a9abe056d1458378cf74418df5cfa14fc47d42f90b9`. They are not the earlier accepted drivers, even though the natural toolchain labels remain the same. Private homes, caches and targets stay isolated.

Before the reconstructed pair is measured, the benchmark's live-process discovery conservatively excludes only proven zombie/dead processes, continues counting live sleeping/blocked/stopped compilers, and aborts on unknown discovery/parse failures. Load and other-CPU thresholds are unchanged. The policy is recorded and must match across compared results. It does not rewrite old observations, change the frozen client, or turn its raw sampled zombie counts into zeroes.

### A2 baseline attempted, comparison blocked

The reconstructed A2 seed-1, 40-point, one-replicate attempt completed with harness exit 0 (`task5673007jqd`), from October 14, 2026 21:46:08 to 23:11:52 Pacific. Completion is not eligibility. Original `bench/results/native-dispatch-synth40-A2-20261015.json` is preserved byte-for-byte, SHA-256 `7b59851e5a660e9fdf18f2953184645ce230307f489ef8a122b228613ff601d3`. B2 has not been run. Both A2/B2 setup correctness gates above remain passed, not speed evidence.

The existing `bench/synth/run.py` eligibility gate reports **46 blockers: 39 noisy points and seven inconsistent success/test outcomes**. All 40 points have one sample, summary and no measurement-error entry. Only point 34 is quiet, leaving **zero comprehensively eligible points** under `_complete`, which deliberately does not salvage a subset. Measured foreign-rustc-before counts are all zero, but start load ranges from 3.82 to 20.09 and exceeds the unchanged 4-core noise threshold on 39 points. Zero sampled foreign rustc does not make a loaded run uncontended.

| Point | Saved run ID | Failed tests | Outcome |
|---:|---|---:|---|
| 6 | `20261014-222445927-1788897481` | 1 | Desktop bundled GitHub applet assertion |
| 11 | `20261014-223710077-1788897721` | 5 | Repository discovery, tool-description caps and feedback-hook assertion |
| 13 | `20261014-223949395-1788897209` | 2 | Test-harness detection and provider catalog assertion |
| 15 | `20261014-224343182-1788897308` | 10 | TUI ambient/onboarding, Git-repository assumptions, UI and restored-state assertions |
| 21 | `20261014-225033785-1788897888` | 8 | TUI ambient/onboarding, Git-repository assumptions, model/UI and restored-state assertions |
| 28 | `20261014-230016579-1788897555` | 1 | Desktop hint-chip bounds assertion; also retained rerun-suspect annotation |
| 37 | `20261014-230759120-1788897944` | 9 | TUI ambient/onboarding, Git-repository assumptions, model and restored-state assertions |

All seven planned success (`expect_fail=false`), compiled without a reported compile failure, then exited 101 with actual failing tests. All seven complete saved `output.jsonl` streams were read, not just summary verdicts. The audit retains their full-stream hashes, row counts, failed-test lines, result lines and panic contexts. These establish real test failures, not compiler causality or a complete root cause. The five deliberate compile-error points (16, 17, 18, 23, 31) instead have consistent `expect_fail=true`, exit 101, `compile_failed=true`, and zero failed tests. They are not the seven outcome problems. No expectations or observations were rewritten.

The invocation audit contains **86 identities: 40 preparation, six previous-command and 40 measured**. All 86 saved metadata hashes match and all 40 measured samples carry identity verification. Before/after declared-file proofs match and `toolchain_identity_comparison_issues(A2,A2)` is empty. This is self-consistency only, not proof of a matched pair or loaded process memory. The frozen measurement-plan SHA-256 remains `f6fd578fa3098e4d4960bb056f6dc8f8f41ee0828ca1d1bbd3a660dddc6d4b68`.

The documented export CLI was run from the repository root:

```sh
JUSTRUST_HOME=/home/jeremy/.jcode/scratch/dispatch-synth40-coherent/A2/home \
  justrust export --out data/runs --machine dispatch-synth40-A2
```

It exported 87 slim runs into the new isolated `data/runs/dispatch-synth40-A2/` shard: all 86 cited invocation IDs plus the pre-existing coherent A2 setup run. No cited identity run is missing. Unrelated `data/runs/xps14/` and `compiler/FINDINGS.md` were not edited. Original private logs and benchmark evidence remain in place. Reproduce the read-only audit with:

```sh
python3 bench/synth/audit_dispatch_a2.py \
  /home/jeremy/.jcode/scratch/dispatch-synth40-coherent/A2/home
```

The output is retained in `bench/results/native-dispatch-synth40-A2-audit-20261015.json`, including the exact existing suite blockers and eligibility implementation hash. **The comparison/default-promotion gate is blocked by the baseline itself.** No B benchmark or compiler build was launched for this audit. A future valid paired measurement needs controlled load and correct workload outcomes without dropping points or relabeling failures. No speedup, zero overhead, default-path nonregression, default compiler promotion or measured 96-core speedup is claimed.
