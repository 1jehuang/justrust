# Native allowance idle fastpath: evaluated results, 2026-10-14

**Historical evidence-only checkpoint for candidate `1233aa16…`, not an overall accepted optimization or runtime/default promotion.** The matched production-module experiment shows substantially lower idle medians alongside natural multi-capacity and forced-pending regressions. Correct source behavior and passing tests do not imply overall performance acceptance. TicketedProxy remains unwired; the compiler frontend still selects legacy Proxy.

Slim, path-redacted provenance: `data/runs/allowance-fastpath-20261014/native-acceptance.json`. Native paths below are relative to `scratch/allowance-fastpath-native-retry-20261014` under the agent scratch directory. Benchmark paths are repository-relative. These hashes identify the exact evaluated frozen candidate, not whatever a subsequent live source experiment becomes.

## Requirement → check → observed result

| Requirement | Check / public evidence | Observed result and limit |
|---|---|---|
| Exact before/after runtime source | Archived production module hashes and coherent native build source manifests | Before `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18`; evaluated after `1233aa16d73ae11cf447ab5bac0abd2adf48efcb2873574fb08e4e9428f7e631`. jobserver.rs unchanged `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7`. |
| Slot-free idle path preserves grant authority | Owning State lock sees used=0, asserts pending=0, registers one request, reserves used=1, emits IdleImplicit grant and Ack | Production idle path performs no request-slot Arc allocation or subordinate predicate lock and does not unlock before Ack. Optional IDs and independent inventories remain active. |
| Allocation gap rechecks actual state | New `slow_registration_rechecks_idle_after_allocation_gap` test | Failed preliminary check changes no ID/count/registration. Allocate outside State, then register once and recheck under second lock. Release during gap yields one IdleImplicit grant/Ack with no fabricated Pending/helper request. |
| Registered Pending cannot be leapfrogged | New `allocation_gap_cannot_leapfrog_a_registered_pending_request` test | Release transfers to already registered recipient instead of making used=0. Paused unregistered caller later queues and cannot return on spurious notification. Fairness is registration order, not call-start or preliminary-check order. |
| Reserved idle grant survives reverse acknowledgment | New `idle_grant_is_reserved_before_outside_lock_ack_hook` test | Test-only pause after reservation permits another real helper recipient to Ack first without changing named IdleImplicit grant or used accounting. Production lacks this pause/unlock seam. |
| Event mode/cap/ID exhaustion never controls admission | New `slot_free_idle_path_preserves_event_modes_and_overflow` plus unchanged recorder tests | Idle path cannot reach allocation hook; event-buffer-off/on and exhausted optional IDs/counters preserve admission. Buffer-off is NOT fully zero-telemetry capture-off. Enabled records cap at 4,096, with explicit loss. |
| Existing request predicate and conservation retained | Existing eleven actual-source tests plus four new tests | Targeted request predicate/Ack and helper/local-transfer/retained-implicit behavior retained. Source reviewer mouse found no substantive delta blocker, but ran no builds/tests. Source clearance is not a performance verdict. |
| Actual native crate tests | `934537nw1f` and reordered coherent retry `3640775w60` | PASS: 133 tests, 0 failed, 0 ignored, 0 filtered; 5 doctests passed, 1 ignored. First native crate task wall 14.8 s. Retry independently repeats this gate after actual compiler fixtures. |
| Actual compiler compatibility without source/harness weakening | Retry `3640775w60`, five harness logs and source/driver manifests | PASS: admission 1, wait 4, callback 1, compiler binding 2, macro/argv 1, nine cases total. These exercise unchanged legacy frontend admission, NOT ticketed Registry/query integration. Combined retry 65 s, cached assembly about 2 s, coordinator wall measurements. |
| Explain failed first native sequence | Initial `173203c39a` callback setup traceback versus retry ordering | Source-exact first build about 79 s, native crate/admission/wait passed, then callback setup lacked full private metadata after x.py crate tests cleared stage0 sysroot metadata. Corrected order runs private compiler fixtures before crate tests. No runtime source fix or acceptance weakening. Initial overall exit remains failure. |
| Preserve schema4 evidence independently | Retry actual callback receipt and public graph | 33,319 expected/actual points, 3,874 bound, 11 registered/resumed wait pairs, zero binding/read loss, all_targets_bound=false. Historical schedule-dependent observation, not deterministic expected count or allowance-resource/query linkage. |
| Measure production critical section, not test seams | Hosted release MAIN executables include exact archived module sources | Benchmark binaries are not cfg(test)/test executables. cfg(test) explicitly unlocks after reservation for the Ack hook, then relocks; timing those tests would not measure the shipping one-critical-section idle path. |
| Matched before/after inputs and balanced ordering | `523798sl96`, 15 rotating ABBA/BAAB rounds, both event modes | PASS: 120 batches, 720 samples, 180/variant, 30 per timed scenario/variant; no failures/timeouts. Exact jobserver=0.1.34, compiler, harness, workload, inherited affinity and event modes matched. |
| Exact workload/capacity/conservation | Every sample's independently asserted work and external active counter, request/grant/Ack/release inventory | All 720 exit zero, completed pairs exact, peak active≤capacity, request=grant=Ack=pairs, releases=pairs+1 bootstrap, final used=pending=helper_errors=0, no counter overflow. All 120 surplus samples acknowledge supplies=callbacks_completed=1. These do not prove holder/physical-token lineage, fairness or full trace completeness. |
| Expose contention regressions, not only targeted idle result | Natural one/multi plus forced-pending supplementary cases, raw outliers and paired blocks retained | Idle medians lower; natural multi median higher in both event modes. Paired multi ratios 1.0902 disabled and 1.1655 enabled. Forced-pending-one also higher. No aggregate optimality, broad speed or promotion claim. |

## Final matched focused measurements

Source: `bench/allowance-tickets/fastpath/results/abba-015-retry.json`, SHA256 `22829467a07bffa91cd91eea23a84bbfe922d999431b26e91f9562d8e8b3eae5`. Task `523798sl96` completed in 76.48 s after native retry had finished. Original 120 batch paths/hashes, exact summaries and paired-block ratios are in slim JSON. Runner methodology and build receipts are in the benchmark README.

Median elapsed milliseconds, before → evaluated after:

| Scenario | Event-buffer-off | Event-buffer-on |
|---|---:|---:|
| Idle, 10,000 pairs | 1.073910 → 0.266289 | 1.176994 → 0.383702 |
| Natural capacity 1, 80,000 pairs | 287.338010 → 277.143257 | 280.705362 → 279.864320 |
| Natural capacity 4, 80,000 pairs | **174.861898 → 189.085572** | **170.185051 → 214.829179** |
| Forced-pending capacity 1, 1,600 pairs | **20.361399 → 22.166133** | **39.655131 → 41.391242** |
| Forced-pending capacity 4, 1,600 pairs | 16.419868 → 16.718671 | 24.025003 → 24.051120 |

Natural multi rises about 8% off and 26% on by ratio of overall medians. Median within-block after/before ratios are separately 1.0902 and 1.1655, not the same statistic. Sheep reports 11/15 disabled and 14/15 enabled multi blocks slower. Forced-pending-one medians rise about 9%/4%. These mixed observations reject an overall-win/default-promotion conclusion for this candidate. They do not isolate a causal explanation from noisy cases.

Natural scenarios use eight workers, capacities 1/4 and 10,000 pairs each. Forced-pending scenarios use 200 episodes of eight pairs with actual registered Pending requests confirmed before gates release, at least seven/four Pending per episode. Those timers include barriers, channels and private snapshot cloning and are not pure lock/allocation attribution. The contended path's additional preliminary State lock is an architectural risk that must remain visible rather than being averaged away behind idle results.

Quiet status is unknown. Affinity 0–15 is unchanged/inherited, not actively pinned; CPU is heterogeneous and load is recorded per sample. Raw outliers are retained. Event-buffer-off retains IDs/counters, so off/on isolates event storage only. Enabled long traces are capped/lossy. Timed snapshots may capture helper supply before callback completion; surplus fixtures explicitly wait for completion. None of this establishes comprehensive session-weighted build performance.

Initial balanced runner `4717919awt` stopped on relative-output-path bookkeeping after one six-sample batch. Resolving the output path fixed only runner bookkeeping; workload, native sources and executables were unchanged. That batch and 48 preliminary samples remain outside the final balanced 720-sample comparison, not silently mixed or overwritten.

## Exact artifact and native provenance

- Native evaluated stage1 driver: `1e876cc55afa97e8520bec40f5ad348fb3ee3c2a6f40f8ba39544ce6a52fbc8c`; launcher: `95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba`. `compiler.sha256` and `source-before.sha256` qualify the native retry.
- Benchmark before production executable: `86e76145438916c201a449afa189d67118bae506db9926265c4dbcd23a08e394`; evaluated after: `b5fc3065499e8d523694b25a60020a9ab8dd350c1db8cccac1cc6c309fb915e1`. Each stage uses the same executable for both event modes.
- Shared benchmark harness: `eca652fed0bb108abd6d14d18ffaeb5253afccc61e233b78a4a0537f1414a927`; lockfile: `cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236`. Benchmark compiler nightly commit `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6` is distinct from actual rebuilt native stage1. Local compiler binary pin plus hosted artifact version stamp does not prove hosted ELF identity.
- Actual legacy callback: `query/native-task-binding-693d4uh9/capture/1792010397206114139-1362290-1.json`, SHA256 `7ccce6d7bd31c45627da4712adae68c22d0682a950575f7461054c374ef30fac`. Five native log hashes are retained without copying the 21.7 MB wait output.
- Sloth reports all 15 include-source tests passing in debug/release with warning-free clippy. Exact cited runs: `20261014-133045116-1255662`, `20261014-133045248-1255761`, `20261014-133045538-1256117`, `20261014-133045816-1256247`. Actual native 133-test execution is independently attributed to main, not the read-only reviewer or include-source harness.

New root client gate `682331ltmb` passed in 16.23 s: fmt check, hosted check `20261014-134442741-1418120`, full tests `20261014-134444010-1418389` (**559 passed, 0 failed, 3 ignored**), and all-target deny-warning lint `20261014-134457381-1417956`. This is client regression evidence, not ticketed frontend integration or performance-promotion acceptance. Coordinator owns cited run export publication and excludes the live sibling source from this historical evidence-only checkpoint.

## Remaining gates and next hypothesis

Callback already in-flight teardown, actual helper Client IO/callback/raw-release failures, ticketed frontend Registry/query startup/sleep/cycle/reacquire/shutdown and reservation convoy/fairness remain future gates. Resource wire format, reciprocal attachment/checkpoint/seal, complete independent resource receipt authority and explicit query-wait linkage are not implemented or established here. Existing legacy compiler tests do not substitute for them.

An advisory idle-hint experiment was separately authorized after this mixed result. It is a hypothesis outside this frozen evaluated candidate and has no acceptance or performance evidence in this document. Re-measure its exact sources and native correctness before making any comparison. Comprehensive session-weighted and relevant cold/incremental/contention gates are still required before any broad performance or default decision.

No holder/lease/thread/physical-token ownership, full mapping/full-pipeline graph, thread ceiling, CPU work/span or live runtime promotion is established. This worker created only this new results document and slim JSON, with no builds, existing-file edits or commits.
