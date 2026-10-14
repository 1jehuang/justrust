# Whole-result recheck, October 14, 2026 UTC

## What actually ran now

The cumulative October 13 result has18 H/T requirements. All18 are mapped individually, plus8 next-phase requirements, in `bench/results/cumulative-requirement-recheck-20261013.json`. The artifact records exact command timestamps, named passing cases,23 dated actual runtime rows,8 concrete edited-source/error samples, and every unexecuted current-host boundary. Aggregate test counts alone are not the mapping.

Between00:06:30 and00:08:55 UTC, the current public reporting interfaces, actual staged compiler component checks, full actual cloud API suite and correct deployed billing API suite were exercised. The initial unrelated backend checkout was discovered and excluded, rather than mistaken for hosted billing coverage.

| Public output or boundary | Actual fresh observation | Scope |
|---|---|---|
| `bench/versus.py report` on original42-row artifact | All3 workload wall ratios are `contended`; geomean is `not comparable`. Original2.05x reference and1.26x geomean claims are absent. | Real public reporting path, not only a synthetic unit fixture. |
| `bench/versus.py compare` on that original artifact | All6 stack/workload rows keep raw medians but suppress wall-percent verdicts as `contended`. | Real public compare path. Raw artifact SHA remains bd8040a8a5abc2ead6ee2d0894b7fa37bbd02408ed04cf75f507071497490b38. Existing VERSUS.md restored byte-identically from a backup. |
| `bench/suite.py compare` on original matched compiler pair | Reference5.32 to4.56s, -0.76s/-14% faster; body/no-op within noise. | Reproduction of an actual earlier24-core measured result, not a new96-core benchmark. |
| Whole committed-output audit | Before publication112 outputs/1275 cited IDs/1284 observations. After publication atae07ef1:114 outputs/1275 IDs/1315 observations,0missing/0mismatch. | Actual whole committed-output rerun includes all31 newly mapped runtime/sample rows. Offline correspondence, not new runtime acceptance. |
| Python public test command and formatting |38 individually named tests pass, including noisy/missing-provenance rejection, fallback rejection and output mismatch regressions. `cargo fmt -- --check` passes. | Exact current project interfaces. |
| Actual staged compiler | SHAefd82c516ba2b499e4a617cb97fbcbb65b8b1088e64b9c2654e98f7281d64e8a,18 interface checks,12 filter checks and40 deterministic span/body rounds pass. | Actual custom binary, direct local component exception. Not a substitute for hosted end-user acceptance. |
| Actual cloud project API tests |78 Node and11 Python pass, including authoritative queued visibility, refused charge and counted reclaiming. | Real project API entry points with fake upstream/SQLite boundaries. No destructive live outage or quota injection. |
| Actual deployed hosted-builds API worktree |439 full tests and13 targeted billing/identity/quota cases pass atfff73db. | Real existing deployed-source worktree, not copied functions or a switched peer workspace. |

## H/T requirements: individual outcomes, not fresh-runtime relabeling

| Requirement | Observed result and latest check boundary |
|---|---|
| H1 plain hosted routing | Actual final plain check `20261013-164407681-4115213`, test `20261013-164409873-4115463`, strict lint `20261013-164412791-4115540` and repeated check `20261013-164413643-4115676` were hosted/nightly/exit0. A new96-core run is blocked by the owner lease, not called passed. |
| H2 recording/cache reuse | Those actual default rows each0compiled/56fresh. Named `sync_run_rejects_failed_writes_then_checks_latest_source_and_reuses_artifacts` passed in both actual tested compiler suites. No new host-reuse proof is inferred from a source diff. |
| H3 edited source reaches tests | Exact first body/test-edit IDs on each compiler are in the new artifact. Actual test-edit samples compiled1 unit and passed3 real FPS tests, across7 measured samples per compiler. Fresh rerun on newhost is blocked. |
| H4 errors/final verdict | Actual first error samples `20261013-163659667-4070699745` and `20261013-163716500-4071953373` retained real Cargo101/E0308 and saved full logs. Actual UI assertion output remained visible. No fresh hosted error injected during lease. |
| H5 permissions/isolation | Named actual `run_children_preserve_distinct_umasks_without_changing_agent_default` and `invalid_caller_umask_rejects_run_before_execution` passed. Real Desktop `refuses_foreign_launcher_and_writable_directories`, `symlinked_managed_root_is_refused`, and `extract_accepts_only_manifest_and_strips_dangerous_permissions` passed on both compilers, within16 unchanged package/security cases. Newhost rerun blocked. |
| H6 executable/RunSync/cancel | Named actual `replaced_agent_executable_still_checks_with_stable_recording_shims`, failed-write/latest-source/artifact reuse, and `cancel_interrupts_a_run_and_others_continue` passed in actual default and JR suites. They are individual observations, not merely360 aggregate tests. Newhost rerun blocked. |
| H7 quota/identity/admission | Fresh deployed-source tests individually passed exhausted-account readback rejection, rollback of reservation/charge/outbox, shared-token-only authorization denial, and no unbilled overage. Fresh cloud refused-charge and queued-before-success cases pass. Positive live admission remains dated actual hosted builds. Production negative injection is intentionally unverified. |
| H8 regional reclaim | Fresh actual cloud tests passed failed-stop-keeps-counted, combined AWS-stop/D1-compensation failure stays reclaiming, and confirmed-stop-frees-exactly-one-slot. Live fault injection not attempted. |
| H9 10x stock-local | Unmet. A fresh45-second quiet preflight00:07:17 to00:08:02 UTC saw load10.6 to10.8,0foreign rustc, so did NOT enter stock compilation. No fabricated replacement baseline. Previous42-row stock control remains rejected. |
| H10 published evidence | Actual current public audit passes all112 pre-publication outputs. Original raw input unchanged.23 explicit runtime rows and8 exact sample rows in this supplement must be checked against the committed ledger after publication. |
| H11 benchmark interfaces | Current38 named tests, public compare/report/audit and formatting pass. Actual matched compiler comparison reproduces only the14% reference gain. |
| T1 exact compiler/loading | Fresh local version and SHA match actual staged compiler. Earlier Ubuntu loader/hash and normally admitted selected-compiler builds are dated evidence. Current remote hash/loader probe blocked. |
| T2 interface behavior | Fresh18 checks individually include body/comment/panic edits keep interface, semantic edits change it, dependent links to changed body and runs19 1, signature change rejects, no sidecar without flag. Component coverage only. |
| T3 filtering/transitions | Fresh12 checks individually preserve unselected E0308, real warnings and env dependency. Actual previous3-test compiled harness and restored1313-test harness remain dated. Newhost full transition blocked. |
| T4 span/body stability | Fresh deterministic40-round actual compiler component stress passes. No new semantic-cache implementation. |
| T5 nondefault staging | Earlier actual selected JR check/test/lint and restored plain-nightly check/test/lint proved explicit selection and no default activation. Current installed client hash unchanged. Current remote stage/default check blocked, not inferred from inspection. |
| T6 real behavior | Actual unchanged full Desktop UI had1302pass/1samehint-chip failure/10ignored on both compilers. This is failure parity, not a clean promotion gate. Current real-project rerun blocked. |
| T7 useful loop improvement | Actual matched24-core reference5.32 to4.56s was14% faster,7 samples/2warmups each, with equal real test behavior. Body/no-op unchanged within noise. Different upstream revisions preclude isolated PGO causality. New96-core/staticCargo performance is unmeasured here. |

## Next-phase requirements remain unfinished

The owner confirmed00:01 UTC that the own host is now96-core m8azn.metal-24xl with static jr-cargo125bdc8c1. This is owner-reported state, not this session own runtime probe. The owner holds the host exclusively for a remote scaling sweep until explicit release, expected01:45 UTC. No hosted/status/SSH request was issued by this session during that sweep. The prior24-core/officialCargo result cannot certify this changed runtime epoch.

N1 stock control is blocked by the actual quiet gate. N2 scaling results are not received. N3 separate compiler/Cargo contribution is not yet measured on this epoch. N4 filter timing remains conditional, not a correctness-to-speed inference. N5 fresh API visibility/reclamation checks pass and unsafe deferred bookkeeping remains rejected. N6 compiler implementation gates remain intact, no new patch. N7 a new runtime improvement and fresh actual acceptance are not completed. N8 current reporting/audit paths pass and exact publication is the completed reporting result, not a new compile gain.

The previous00:50 retry conflicted with the owner lease. Only after confirming replacement was it cancelled. Scheduled continuation sched_5ec3312d resumes October14 at01:55 UTC, first requiring explicit owner release and a quiet laptop. No user response is needed. The 10x goal is NOT completed.

## Weak points found and corrected

The primary backend checkout is now a peer feature branch without hosted billing. Its372 passing API tests were excluded from H7, despite the successful aggregate. The real existing hosted-builds worktree then passed439, and its actual13 billing cases passed separately. No peer branch, source or index was changed.

The scratch recheck parser initially counted37 verbose case lines because one successful test interleaved benchmark progress before its `ok`. The actual public command passed38. The parser was corrected to preserve all38 named successful observations. This was a verification-helper error, not a failing product test.

No Rust client behavior, installed binary, live worker, quota, pricing or compiler default was changed in this phase. Current source inspection confirms no committed Rust/build-source change since701baa2, but that inspection is explicitly NOT a substitute for a newhost acceptance run. The concrete improvement completed here is truthful public reporting of noisy evidence. The real measured compiler/reference improvement remains dated actual evidence, while broader10x and current-epoch validation remain open.
