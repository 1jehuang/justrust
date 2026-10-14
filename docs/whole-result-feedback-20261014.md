# Whole-result feedback and explicit remaining gates, 2026-10-14

## Observed improvements, not a performance claim

- Actual current graph/mapping/profile/phase public CLIs reprocessed the retained tiny, Desktop CHECK and optimized BUILD inputs. The optimized mapping again joins 2,695,071 providers to 2,809,610 nodes and retains 114,539 untimed nodes, 68 missing wait-read edges and two missing callers. This is usable invocation-level dependency/timing coverage beyond occupancy, not a complete execution DAG.
- The all-kind phase output is byte-identical to the committed result: 6,851,280 records, 584 groups, 517 Rust thread IDs, covered/exclusive elapsed 148,396,651,246 ns. Thread elapsed is not CPU work. CPU-work and execution-span remain null.
- The unchanged actual noisy one-point partial still makes the original reporter print a misleading 20.53 s expected wait. The current public reporter blocks that aggregate while retaining annotated raw detail. Clean, renamed, malformed and partial/full-counter controls are separately exercised, not advertised as new benchmarks.
- Exact original generated-script controls still incorrectly return success for payload 23 and 143. The final script returns 0/23/143 correctly. Six real production fork/exec stdin checks and 11 terminal/lifecycle checks pass; the detached default remains rejected because an actual private tmux teardown leaves its detached child alive.
- Four real compiled CPU regressions historically failed before the recording fix and passed afterward. All 21 original measurement summary hashes, exported outcomes and named CPU/summary cases were reverified locally. These are frozen e629/08d outcomes, not a rebuild or certification of the current mutable frontend or peer-dirty recorder source.

## Whole-result acceptance scope

One final bounded pass ran 41 named checks, including all 231 current benchmark tests with individual case output, all four actual capture mappings, the actual large phase CLI, default-limit/wrong-pair/strict-label/symlink/overwrite rejection, actual-report refusal and positive controls. Four additional local audits check six pinned compiler Git objects/17 literal anchors, the original optimized BUILD metadata, frozen real measurement outcomes and the saved resource-restoration response. All 54 benchmark Python/shell source hashes stayed unchanged.

No new Rust compilation, account-host request, SSH/provider request, global install, compiler flag/default change, cache mutation, EC2 start or performance experiment was made in this pass. Public tooling is exercised directly on real retained captures; synthetic input controls and synthetic launcher transport are labeled separately. Compiler/BUILD source, hardware, cache and contention epochs are not combined. A private export deliberately redacts arguments to `[build]`; an initial audit expected `--release` there and was corrected to verify the original raw private metadata/summary. The original export was not changed.

## Unmet requirements remain unmet

1. Complete continuation-level execution DAG, CPU-work W, execution span S and a core ceiling: blocked by missing task/synchronization/CPU/subprocess observations. Existing read dependencies and elapsed timers cannot establish them.
2. Clean comprehensive optimization gain: blocked. The exact original 37 records and 15 sample/outcome joins were re-audited; OFF remains 15/40, ON 0, seven noisy samples and three unexpected failures. Fresh public show/self-compare correctly refuse any aggregate or ratio. No samples were repaired, merged or reclassified.
3. Actual hosted asynchronous launcher lifecycle acceptance: not exercised by local synthetic transport. Correct payload exit preservation is observed, but a supervised headless mode is not accepted or promoted.
4. Archived workload runtime prerequisites and unexpected assertions: unresolved. The separate headless diagnostic still exited 101, not a fixed-workload or causal-suspension result.
5. Exact original legacy stop signal: unknown. Local SIGTTIN/timeout fixtures show a possible mechanism, not historical signal attribution.

The saved provider response proves the experimental EC2 was restored stopped at 10:27:09 UTC with its 500 GiB volume preserved. This final pass checks that receipt's integrity, not fresh AWS state. Repeating the same blocked experiment without repairing its prerequisites is not acceptance. All remaining gates are separate from the accepted diagnostic/report/stdin/status improvements.

## Every requirement mapped to an observation

The following 82 rows reproduce every explicit requirement from the eight mapped documents. Named check IDs resolve to exact exit codes and full raw-to-redacted stdout/stderr hashes in the adjacent acceptance JSON. Named regression cases resolve to the final verbose whole-suite log. `RETAINED_ACTUAL_VERIFIED` means an original real compiler/provider workflow re-audited, not a new invocation. `REPRESENTATIVE_ONLY` does not certify hosted lifecycle. `BLOCKED` is a failed acceptance boundary, not completed optimization.

### compiler-query-graph-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Actual invocation IDs, not kind DOT or serialization ordinal | tiny-graph-default, tiny-graph-large, mapping-t1, mapping-t4, mapping-desktop-t4; test_out_of_order_actual_ids_and_holes, test_fixed_seed_sparse_dag_record_permutations | Actual t1/t4 nodes3379/edges8215, t4holes366; Desktop CHECK1052237/6496600. Default/large tiny graph stdout byte-identical; no serialization-ordinal identities. | FRESH_OBSERVED |
| Exact source/compiler format | pinned-compiler-source-object-audit, whole-current-tests; test_magic_version_and_trailer_rejected, test_footer_only_mutations_rejected, test_leb_corruption_rejected | Exact pinned compiler Git objects verified; named format/footer/LEB failure cases pass. Pinned337-kind capture labels unchanged. | FRESH_OBSERVED |
| Same-session graph/profile join | mapping-t1, mapping-t4, mapping-desktop-t4, wrong-real-pair-rejected; test_each_receipt_identity_mismatch_is_rejected, test_metadata_mismatch_is_rejected | All three actual CHECK-pair mapping metrics equal published results; wrong actual pair exits2 and creates no output. | FRESH_OBSERVED |
| No nested time double-count | whole-current-tests; test_nested_query_and_known_wait_not_double_counted, test_cross_thread_producer_is_not_caller, test_equal_labels_do_not_merge_numeric_ids, test_crossing_same_thread_query_intervals_rejected | Named nesting, producer/caller and numeric identity cases pass; invalid crossings reject, not reclassified as parallel work. | FRESH_OBSERVED |
| Missing coverage retained | mapping-desktop-t4, tiny-unmapped-strict-rejected, tiny-unmapped-explicit; test_unresolved_labels_and_unknown_nodes_retained | Desktop79132untimed/55missingwaitedges retained. Tinystrict exits2; explicit mode preserves41509records/21427unresolved label records. | FRESH_OBSERVED |
| CPU/span claims constrained | mapping-desktop-t4, mapping-desktop-release-t4, phase-large-real; test_noninterval_counters_never_become_elapsed | All actual outputs CPU/span remainnull. No frontier, instruction counter or threadelapsed substituted for CPU work. | FRESH_OBSERVED |
| Public failure semantics / packaging | query_graph-help, query_profile-help, query_mapping-help, whole-current-tests; test_public_cli_writes_bounded_readonly_mapping_and_refuses_overwrite, test_bounded_regular_input_and_exclusive_output | Current real CLIs accept supported inputs and preserve expected failure semantics; all named whole-current cases pass. | FRESH_OBSERVED |
| Filesystem/resource safety | mapping-existing-output-preserved, profile-symlink-rejected, whole-current-tests; test_receipt_read_cap_and_symlink_rejection, test_bounded_regular_input_and_exclusive_output | Existing t4mapping SHA unchanged after exit2 refusal; real input symlink rejected with no output. Named readcap/output safety cases pass. | FRESH_OBSERVED |
| Actual real workload, not only synthetic fixtures | mapping-desktop-t4, mapping-desktop-release-t4 | Real retained Desktop CHECK and optimized BUILD inputs analyzed through current public tools, not copied-source fixtures; all mapped published metrics reproduced. | FRESH_OBSERVED |
| Improvements are useful, not claimed performance gains | mapping-desktop-t4, mapping-desktop-release-t4 | Observed numeric provider->node and caller->producer/wait relationships are inspectable, an improvement over occupancy-only output. Complete execution DAG and speedup remainblocked. | FRESH_OBSERVED |

### compiler-query-build-graph-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Actual optimized BUILD, not CHECK | actual-optimized-build-export-audit | Original raw meta/summary confirms hosted BUILD --release exit0 and UIopt3/linkemit, delivered library artifact; fresh audit is offline, not another build. | RETAINED_ACTUAL_VERIFIED |
| Same-session graph/profile identity | mapping-desktop-release-t4 | Original raw input hashes checked. Fresh mapping joins2695071providers to2809610nodes;0invented provider nodes. | FRESH_OBSERVED |
| Real public large mapping | mapping-desktop-release-t4 | Real large public mapping exit0, all published output fields except documented limitation prose unchanged; CPU/spannull. | FRESH_OBSERVED |
| Public graph and profile interfaces | release-graph-large, release-profile-large | Public large graph/profile summary exit0 on167MB/263MB actual inputs;6851280records and0unresolved labels. | FRESH_OBSERVED |
| No silent default resource expansion | release-graph-default-rejected, release-profile-default-rejected, whole-current-tests; test_large_capture_is_explicit_and_still_bounded, test_large_capture_requires_opt_in_and_preserves_record_api | Both actual default CLIs exit2, no profile output; explicit limits selected separately, malformed flag/count APIs tested. | FRESH_OBSERVED |
| No filtering or nested-time invention | release-profile-large, mapping-desktop-release-t4, whole-current-tests; test_streamed_records_match_materialized_mapping, test_nested_waits_partition_without_double_counting | All6851280events retained,2695071provider records mapped; streaming/materialized and nested-wait semantics pass named tests. | FRESH_OBSERVED |
| Whole package and boundaries | whole-current-tests; test_receipt_read_cap_and_symlink_rejection, test_limits_without_large_allocations, test_footer_only_mutations_rejected | Current231-test package ran verbosely; exact named resource/provenance/format failures map to these requirements, not only aggregate count. | FRESH_OBSERVED |
| Independent whole-result review | tiny-graph-default, tiny-graph-large, phase-independent-reference | Fresh public tiny parity and independent250-forest reference pass. Earlier independent real-release review retained, not described as a fresh second external review. | FRESH_OBSERVED |
| Exact exported records and privacy | actual-optimized-build-export-audit | Cited BUILD export and raw private meta/summary hashes verified. Only compact observations/log hashes published; full command/profile staysprivate. | RETAINED_ACTUAL_VERIFIED |
| Infrastructure and lifetime boundaries | actual-optimized-build-export-audit | Saved BUILD06:24 and host release06:41 evidence retained. This pass contains no host/start/install/capture call, so no new live-state or lifetime claim. | RETAINED_ACTUAL_VERIFIED |
| No performance or core-ceiling claim | actual-optimized-build-export-audit, mapping-desktop-release-t4 | Oneforeign rustc/107cachemiss epoch retained; no before/afterpair and no performance or core-ceiling claim. | FRESH_OBSERVED |

### compiler-phase-activities-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Account for real optimized BUILD, not only CHECK | actual-optimized-build-export-audit, phase-large-real | Actual optimized BUILD profile source identity matched; fresh activities JSON byte-identical to23d71f4e... committed output. | FRESH_OBSERVED |
| Avoid nested double counting across event kinds | phase-large-real | 6851280intervals/584groups/517RustIDs; exclusive=covered148396651246ns over15826062441ns window. No inclusive overlap sums used as CPU. | FRESH_OBSERVED |
| Validate mathematical partition independently | phase-independent-reference | Independent fixed-seed reference exercises250shuffled nested forests across3threads; group allocation and coverage pass. | FRESH_OBSERVED |
| Exercise whole implementation and integration boundaries | whole-current-tests; test_nested_cross_thread_adjacent_and_gaps, test_crossing_rejected_for_every_kind, test_limits_and_consumption, test_cli_privacy_forwarding_preservation_and_bounds | Named all-kind interval, crossing, consumption and public-IO methods pass over final implementation. | FRESH_OBSERVED |
| Preserve large-input default guard and small-input compatibility | phase-default-rejected, phase-large-real, whole-current-tests; test_cli_privacy_forwarding_preservation_and_bounds | Real large default exits2/nooutput; explicit large exits0. Named small-input public CLI compatibility passes. | FRESH_OBSERVED |
| Preserve failures and controlled opt-ins | whole-current-tests; test_symlinks_and_invalid_input, test_cli_privacy_forwarding_preservation_and_bounds | Named actual subprocess CLI cases retain invalid/missing-label rejection, explicit unmappedlabels, existingbytes and symlink targets. | FRESH_OBSERVED |
| Verify source semantics, not name-based CPU assumptions | pinned-compiler-source-object-audit | Six exact pinned compiler Gitobjects and17literal label anchors freshlyverified; coordinator/LLVMlabel naming is notCPUcausality. | FRESH_OBSERVED |
| Keep privacy and provenance | phase-large-real, actual-optimized-build-export-audit | Actual fresh activity output SHA matches committed23d71f4e...; exact inputhash checked. Captured command remainsprivate; no new BUILD record. | FRESH_OBSERVED |
| Leave unsupported conclusions unknown | phase-large-real | CPU/span remainnull;517RustIDs not517cores,148.396elapsed not129.698processCPU. No speedup/runnable-count/pureidle claim. | FRESH_OBSERVED |

### scheduler-legacy-attempt-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Sufficient effective scratch capacity | legacy-real-outcome-export-manifest-join | Actual saved guest capacity permitted15points on500GiB; fresh manifest audit verifies recorded guest output. No new guest capacity probe or bound on full-workload peak. | RETAINED_ACTUAL_VERIFIED |
| Exact compiler/Cargo/CLI and flags | legacy-real-outcome-export-manifest-join | 37exact exportIDs and15outcomes rejoined;35completed boundary records preserve actualprivateCLI/Cargo/rustc/jobs24 flags. Earlier private raw identity audit retained. | RETAINED_ACTUAL_VERIFIED |
| Preserve generator, seed, points and weights | legacy-real-outcome-export-manifest-join | Existing frozen seed1/40points/weights/generator notchanged or filtered. Partial remains15/40, no substituted points. | RETAINED_ACTUAL_VERIFIED |
| Use real public build/check/test interfaces | legacy-real-outcome-export-manifest-join | Real saved calls include releaseBUILD/CHECK/fullandfilteredTEST.35completed calls and preservedinterrupted/diagnostic records, not another actual compile run. | RETAINED_ACTUAL_VERIFIED |
| Match outcome to measurement | legacy-real-outcome-export-manifest-join | All15sample execwall/exit/compilefailure/testsfailed match exactcommands;3normal-success plans failed, not expectederror successes. | FRESH_OBSERVED |
| Clean full paired comparison | legacy-current-public-show, legacy-current-public-compare, legacy-real-outcome-export-manifest-join | OFF15/40,ON0,7noisy/3unexpected failures. Valid full paired expected-session gain remainsunobserved. | BLOCKED |
| End-user report refuses invalid claims | legacy-current-public-show, legacy-current-public-compare | Fresh current public show refuses weightedaggregate; selfcompare refuses ratio, not an OFF/ON comparison. | FRESH_OBSERVED |
| Full harness regression check | whole-current-tests | Current231named tests pass over final reporter/runner/launcher modules; frozen199 historical package acceptance separate. No compiler performance acceptance. | FRESH_OBSERVED |
| Preserve interrupted execution | legacy-real-outcome-export-manifest-join | Interrupted16 remains separate from15samples, no fabricated after-exec or benchmarkwall. Diagnostic remains separate; compressed5nonzero outputs allparse. | FRESH_OBSERVED |
| Bound lifetime and credit burn | retained-resource-restoration-receipt-audit | Savedearly ownedshutdown and originalstopped restoration receipt preserved. No provider call in this pass, historical bound notnewAWSstate. | RETAINED_ACTUAL_VERIFIED |
| Restore initial resource state | retained-resource-restoration-receipt-audit | Savedprovider response confirms stopped10:27:09 and500GiBpreserved/no termination. Freshlocalhash audit only, no newEC2request. | RETAINED_ACTUAL_VERIFIED |
| Keep tenant security/defaults intact | retained-resource-restoration-receipt-audit, legacy-real-outcome-export-manifest-join | Actualattempt usedseparatedeveloper resource; savednormaltenantchangedfalse. No hostedcredentials/security/live mutation in freshscripts; legacybootstrap notnormaltenantproof. | RETAINED_ACTUAL_VERIFIED |
| Preserve honest CPU/scaling scope | legacy-real-outcome-export-manifest-join, mapping-desktop-release-t4 | No comparative speedup,W/S or96coreceiling. SampledCPUlowerbound and processCPU notquerywork. | FRESH_OBSERVED |
| Preserve public privacy and auditability | legacy-real-outcome-export-manifest-join | All39legacy manifestentries rechecked,37exports rejoined and5fullnonzeroJSONL files parsed. Publicpath/addressredaction retained, failures notdiscarded. | FRESH_OBSERVED |

### benchmark-stdin-boundary-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| An open inherited pipe cannot hold a benchmark on stdin | stdin-current-real-exec; test_open_pipe_becomes_empty_non_tty_stdin | Executed child seesEOF/nonTTY withparentpipe heldopen; samePGID/env/stderr/resource schema preserved. | FRESH_OBSERVED |
| Noncontrolling PTY input is also noninteractive | stdin-current-real-exec; test_pty_becomes_empty_non_tty_stdin | Executed child seesEOF/nonTTY while both noncontrollingPTY endpoints stayopen. | FRESH_OBSERVED |
| The temporary `/dev/null` descriptor does not leak through exec | stdin-current-real-exec; test_temporary_null_descriptor_is_closed | Executed child seesEBADF on deliberately inheritable temporaryfd. | FRESH_OBSERVED |
| Opening `/dev/null` directly as fd 0 works despite Python's close-on-exec default | stdin-current-real-exec; test_null_open_returning_zero_keeps_stdin_open | Forcedopen returns0; child canreadEOF despite Python defaultCLOEXEC. | FRESH_OBSERVED |
| Existing context and output survive | stdin-current-real-exec; test_open_pipe_becomes_empty_non_tty_stdin, test_pty_becomes_empty_non_tty_stdin | Actualexec child matches driverPGID/envsentinel andstderrmarker; no processgroup change. | FRESH_OBSERVED |
| Nonzero and signal exits are not rewritten | stdin-current-real-exec; test_real_exec_failure_status_and_output_are_preserved, test_signal_exit_is_preserved | Actual23/stderr and-15signal statuses preserved, not rewritten. | FRESH_OBSERVED |
| Existing Python benchmark integrations do not regress | whole-current-tests | Currentwhole231named cases pass, including all6execution regressions andreport/graph/launcher integrations. | FRESH_OBSERVED |
| Verification does not overwrite unrelated changes or claim old samples use the new runner | stdin-current-real-exec, whole-current-tests | All54current benchmarksource hashesunchanged throughpass; oldsamples remainineligible. ControllingTTY unaffected, no oldcontext remapping. | FRESH_OBSERVED |

### benchmark-terminal-boundary-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Preserve a payload's failure status after logging | terminal-original-red, terminal-current-real-exec; test_public_start_preserves_failure23 | Exactoriginal29c7446 generatedpayload failure23returned0; currentreturns23. RetainedEXIT agrees. Original87example remainshistorical. | FRESH_OBSERVED |
| Retain ordinary success and failure cases | terminal-current-real-exec; test_public_start_preserves_success0, test_public_start_preserves_failure23, test_public_start_preserves_signal143 | RealgeneratedBash scriptstatuses0/23/143 matchEXITlog. Transportsynthetic/asynchronous realdispatch notcertified. | FRESH_OBSERVED |
| Separate stdin from controlling terminal | terminal-current-real-exec; test_devnull_still_sigttin | Actualfixture fd0null+/dev/tty read receivesSIGTTIN21. Stdinintegration notTTYisolation. | FRESH_OBSERVED |
| Test the real timeout grouping boundary | terminal-current-real-exec; test_gnu_timeout_background_group | ActualGNUtimeout backgroundpayload stops inownedPTY; noforeground option orchildtimeout weakening. | FRESH_OBSERVED |
| Verify a detached recipe's natural lifecycle and inherited context | terminal-current-real-exec; test_setsid_wait_exit23_inherited_jobserver, test_private_tmux_natural_session_lifetime | Realsetsid normal23 andinheritedpipe preserved; private tmuxnaturallifetime passes. NotactualCargo jobserverproof. | FRESH_OBSERVED |
| Check the likely unsafe early-teardown edge | terminal-current-real-exec; test_diagnostic_rejected_setsid_tmux_kill_leaks | Actualprivatekill-session leaves detachedchildalive; candidateREJECTED. Stablepidfd verifiesandcleans exactownedchild withfdclosed/nochildremaining. | FRESH_OBSERVED |
| Preserve existing launcher/evidence interfaces | shell-syntax, whole-current-tests; test_diagnostic_public_start_still_has_tty | Publicgeneratedscript retainsordinaryTTY/default;11focused0skips+231namedwhole passon54unchangedsourceblobs. | FRESH_OBSERVED |
| Avoid misattribution and accidental live changes | terminal-current-real-exec | No realSSH/auth/host actionorheadlessdefault promotion; localtransporttests nothosted lifecycleacceptance. Fullremote supervisedlauncher remainsblocked. | REPRESENTATIVE_ONLY |

### synthetic-report-eligibility-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Preserve raw point observations and annotate noisy or unknown noise provenance. | report-current-real-partial-show, whole-current-tests; test_invalid_coverage_and_noise | Actualnoisy20.53detail preserved withannotation; partial expectedwaitaggregate removed, no inconvenientpointdrop. | FRESH_OBSERVED |
| Emit comprehensive expected-wait aggregates and comparative ratios only when every requested point and replicate is present, all noise markers explicitly say false, outcomes match the planned successful or expected-compile-error command, and timing summaries agree with the actual samples. | report-current-real-partial-compare, report-clean-show, whole-current-tests; test_exit_outcome_provenance, test_invalid_timing_and_stale_summary, test_clean_and_expected_compile_error | Actualpartialcomparerefused; cleancontrolsretain2.00report andexpectedE0308behavior; invalidoutcome/stale/boolwall casesblocked. | FRESH_OBSERVED |
| Block malformed identities, missing outcome evidence, duplicate or out-of-range point IDs, measurement errors, suspect harness artifacts, and incomplete runs. Do not silently drop inconvenient points and call the surviving subset comprehensive. | report-malformed, whole-current-tests; test_malformed_public_payloads, test_empty_results_blocked, test_remaining_schema | Malformedpublicdata returnsblocked/nottraceback; typedcoverage/pointidentity/errorcounter checks pass. | FRESH_OBSERVED |
| Compare identical generator, seed, sampling information, point plans, source snapshots, execution mode and machine context. Toolchain or optimization versions can intentionally differ, but changed commands or incompatible source/machine context are not a matched compiler comparison. | whole-current-tests; test_incompatible_identity, test_public_environment_identity | Matchedplan/generator/snapshot/hostCPU/mode exactcontract tests pass; wrongcontextsblocked, no compilerclaim fromfixture. | FRESH_OBSERVED |
| Require complete valid counter coverage across all required replicates before emitting instruction ratios. Missing counters stay unknown rather than disappearing from the median. Otherwise valid wall comparisons may remain available without an instruction claim. | report-missing-counter, report-full-counter, whole-current-tests; test_instruction_counter_coverage | Missingrawreplicate counter suppressesinstrratio butretainsvalidwall; completecounter preservesratio, allnullstaysunknown. | FRESH_OBSERVED |
| Preserve positional stack renaming by mapping sample, statistic and error keys through the original stack identities, not dictionary value order. Do not mutate loaded input results. | report-clean-renamed, whole-current-tests; test_public_positional_remap_noise_and_clean, test_clean_replicates_and_stats_key_order | Renamed/reordered cleanstacksretain+0%report, mapping followsoriginalkeysanddoesnotmutateloadedinputs. | FRESH_OBSERVED |
| Retain valid complete clean reports and expected compile-error workloads. Malformed report data should produce an actionable blocked message rather than a traceback or empty-mean arithmetic. | report-clean-show, report-faster-control, report-malformed, whole-current-tests; test_clean_and_expected_compile_error | Completeclean2.00 andsynthetic-50%mathretained; malformeddata blocked. Controls notrealperformancebenchmarks. | FRESH_OBSERVED |

### measurement-acceptance-20261014.md

| Requirement | Exact checks / named cases | Observed result | State |
|---|---|---|---|
| Preserve survivor CPU after a different child exits | frozen-real-measurement-export-audit | Actualcompiled record::tests::sampled_cpu_preserves_survivor_work_when_other_child_exits failedbefore/passafter; exactoriginalsummarySHA verified. | RETAINED_ACTUAL_VERIFIED |
| Do not recount a process's lifetime after one missing snapshot | frozen-real-measurement-export-audit | Actualcompiled record::tests::sampled_cpu_missing_snapshot_does_not_count_lifetime_again failedbefore/passafter; exactoriginalsummarySHA verified. | RETAINED_ACTUAL_VERIFIED |
| Distinguish PID reuse and overlapping process roots | frozen-real-measurement-export-audit | Actualcompiled record::tests::sampled_cpu_distinguishes_pid_reuse_and_overlapping_roots failedbefore/passafter; exactoriginalsummarySHA verified. | RETAINED_ACTUAL_VERIFIED |
| Do not turn a regressing counter into a later spike | frozen-real-measurement-export-audit | Actualcompiled record::tests::sampled_cpu_counter_regression_does_not_create_later_spike failedbefore/passafter; exactoriginalsummarySHA verified. | RETAINED_ACTUAL_VERIFIED |
| Correct success despite intentional nested compiler failures | frozen-real-measurement-export-audit | Named summary::tests::successful_test_run_ignores_nested_compile_failure actualcompiled pass inclunit/output cases;21rawsummaryhashesverified. | RETAINED_ACTUAL_VERIFIED |
| Preserve genuine compiler failures | frozen-real-measurement-export-audit | Named unsuccessful_build_keeps_unit_and_output_compile_failure_verdicts actualcompiled pass; realouterE0308/101compile_failedtrue retained. | RETAINED_ACTUAL_VERIFIED |
| Preserve runtime test-failure classification | frozen-real-measurement-export-audit | Named runtime_test_failure_is_not_a_compile_failure actualcompiled pass. Frozenoutcomes verified, notcurrentpeer-dirtysource certification. | RETAINED_ACTUAL_VERIFIED |
| Exercise the installed CLI with a real nested compiler failure | frozen-real-measurement-export-audit | Realretained expectednestedE0308 output withouter2tests/exit0/compile_failedfalse rejoined tooriginalsummary+export. | RETAINED_ACTUAL_VERIFIED |
| Dogfood normal hosted routing and packaging | frozen-real-measurement-export-audit | Retained exacte629client hostedcheck/404tests/strictlint/recheck passed; frozen08d identity separate fromlater0a19/currentmutableglobal. | RETAINED_ACTUAL_VERIFIED |
| Preserve unchanged Cargo artifacts | frozen-real-measurement-export-audit | Allfour installede629acceptance records0compiled/62fresh verified; notactualfreshRustinvocations now. | RETAINED_ACTUAL_VERIFIED |
| Validate a real reference body edit | frozen-real-measurement-export-audit | RealretainedDesktop bodyedit3tests/1compiled/745fresh recordjoinedexactly. No new referenceedit thispass. | RETAINED_ACTUAL_VERIFIED |
| Preserve unchanged Desktop reuse | frozen-real-measurement-export-audit | RealretainedunchangedDesktoprepeat3tests/0compiled/746fresh recordverified. | RETAINED_ACTUAL_VERIFIED |
| Restore unrelated repository bytes | frozen-real-measurement-export-audit | SavedbackuprestorationSHA24de30f7... andrestored3testrecord retained; no change toanotherrepo now. | RETAINED_ACTUAL_VERIFIED |
| Recording must not invalidate artifacts | frozen-real-measurement-export-audit | Saved actualrecorded/unrecorded/repeatedFresh/rmetabyteparity verifiedinacceptance; no fabricatedDirty-free performancegain. | RETAINED_ACTUAL_VERIFIED |
| Retain failed experiments and harness mistakes | frozen-real-measurement-export-audit | Original397successbadverdict and4compiledredfailures remainunaltered; unsupportedcargo--local error retained. Frozenhistoricalrawhashesall21match. | RETAINED_ACTUAL_VERIFIED |

## Packaging and privacy

Raw private compiler profiles, command receipts and operator paths are not copied into the repository. The public evidence manifest covers every published payload by byte count and SHA256. CLI logs retain complete output except explicit path/address replacements; original hashes remain alongside published hashes. No error, failed assertion, missing coverage or noisy point was intentionally removed. The original exported compiler/measurement records remain in their prior committed namespaces. This is a documentation/evidence publication only, so no Rust reinstall is appropriate.
