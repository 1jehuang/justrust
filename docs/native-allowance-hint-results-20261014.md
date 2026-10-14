# Native allowance advisory hint: evaluated results, 2026-10-14

**Scoped result:** the source-valid, experimental **unwired** hint prototype preserves admission authority and reduces focused idle cost versus the original ticketed protocol. It trades some idle advantage versus the prior fast candidate for improved measured multi-capacity behavior versus that candidate. It does **not** establish a robust natural-multi improvement versus original, blanket regression freedom, broad compiler speedup or default promotion. The frontend still selects legacy Proxy.

Exact evaluated hint SHA256: `f3849e80b7a1f72ee9472592d66a57a83477900d52354a772ac345c77f07def8`. Slim path-redacted evidence: `data/runs/allowance-hint-20261014/native-acceptance.json`. Native paths are relative to `scratch/allowance-hint-native-20261014` under the agent scratch directory; benchmark paths are repository-relative. Historical fastpath/prototype artifacts are unchanged and their samples are not pooled into this new cohort.

## Requirement → check → observed result

| Requirement | Check / observable evidence | Observed result and limit |
|---|---|---|
| Exact threeway sources and native compiler | Frozen archive hashes and coherent source/driver manifests | Original `96d80353…`, prior fast `1233aa16…`, hint `f3849e80…`; unchanged jobserver.rs `5f5d43e6…`. Complete hashes below and in slim JSON. No stale compiler-driver substitution. |
| Hint never authorizes admission | Relaxed AtomicBool selects only a speculative owning-State lock; actual used/pending checked before ID/count/grant | Stale TRUE with used=1 falls through to ordinary slot registration, remains Pending after spurious notification, and receives a named real release grant, not fabricated IdleImplicit. Source reviewer mouse cleared the narrow authority protocol, but ran no tests. |
| Stale FALSE loses optimization, not correctness | Ordinary path allocates Slot outside State and always rechecks actual used at protected registration | Forced FALSE at used=0 executes exactly one allocation/Ack hook and one request/grant/Ack, no Pending; legitimate IdleImplicit comes from State, never the hint. Both event-mode inventories match. |
| All actual zero crossings update under State | Fast and slow 0→1 reserve set hint FALSE; nontransfer 1→0 release sets TRUE | Helper requires used>0 before incrementing and cannot cross zero. Transfer and Ack preserve used. Relaxed hint does not publish State fields, define registration order or authorize capacity. Protected mutex/predicate remain authority. |
| Conservation under races, caps and stale observations | Three new hint tests plus prior fifteen ticket tests | Eighteen source tests total: original eleven, prior fast four, hint three. New race test runs 40 release/acquire races per event mode and verifies exact request/grant/Ack/release counts, used/pending and hint updates. This is bounded coverage, not exhaustive concurrency proof. |
| Preserve registered-Pending priority and native request identity | Prior deterministic allocation-gap, reservation-before-Ack, spurious-wake and exact source tests retained | FIFO claim is protected registration order, not API-call order, hint read or preliminary check. Live Arc/owning-State predicate, not optional telemetry IDs, authorizes pending requests. No holder/physical-token lineage inferred. |
| Production timing is not cfg(test) timing | Production MAIN release executables include exact archived module sources | Production immediate grant/Ack is one State critical section. Test before_ack seam unlocks after reservation, invokes hook outside State, then relocks. Benchmarks use non-cfg(test) binaries, not test execution timings. |
| Event storage independent of admission | Event-buffer-off/on stale tests, overflow/cap tests and benchmark inventories | Buffer-off retains optional IDs and independent counters. It is NOT zero-telemetry capture-off. Enabled storage caps at 4,096 rows with retained loss counters; capped rows do not certify a complete causal trace. |
| Actual native crate validates current hint source | Initial `106914oxke`, coherent `326345s2gi` native crate stage | Both PASS: 136 tests, 0 failed, 0 ignored; 5 doctests passed, 1 ignored. Initial crate-only wall 14.54 s; coherent build/compiler/crate cohort 206.75 s, coordinator task measurements. |
| Coherent actual compiler acceptance without metadata setup failure | `326345s2gi`: all five private/compiler fixtures before crate tests; hashes checked | Nine cases PASS: admission 1, wait 4, callback 1, compiler-binding 2, macro/argv 1. Unlike prior cohort sequencing failure, this cohort had no setup failure. These compiler fixtures still exercise **LEGACY frontend admission**, not TicketedProxy Registry/query wiring. |
| Recording and diagnostics neutrality remain bounded | Admission reuse/fail-open; warmed off/on/off bytes; fresh zero launches; failure/missing evidence/sinks; macro argv observations | All actual legacy compiler gates passed, with macro binding invisible off/on. Callback observed 33,320 expected/actual points, 3,874 bound, 12 registered/resumed wait pairs, binding/read loss zero, all_targets_bound=false. Counts are historical schedule-dependent observations, not resource-query linkage. |
| Matched threeway measurement instead of historical sample pooling | `705228qyre`, rotating mirrored six-position blocks, both event modes | Final 15 rounds, 30 paired blocks, 180 raw batches, **1,080 actual samples** across six variants, 180 per variant and 30 per timed case. Six is the samples-map key count, not sample count. No failures/timeouts. Same jobserver=0.1.34, compiler/harness/hot loop. |
| Exact work, capacity and administrative conservation | Every sample independently asserts completed pairs, external active ceiling and native counters | All 1,080 passed count/capacity/conservation/overflow checks. All 180 surplus samples acknowledge supplies=callbacks_completed=1. Forced pending uses 200 episodes, 1,600 pairs, exact registered pending seven/four and peak active one/four. These checks do not prove fairness or physical-token ownership. |
| Targeted idle result with opposing comparison visible | Within-block hint/original and hint/fast medians, not only aggregate medians | Hint/original idle ratio 0.3107 off / 0.3365 on, all 15 blocks faster per mode. Hint/fast idle ratio 1.2445 off / 1.1312 on, 13/15 and 14/15 blocks slower. Do not hide the cost relative to prior fast. |
| No robust natural-multi victory over original | Paired block directions compared with overall medians | Hint/fast multi ratios 0.9273 off (12/15 faster), 0.9216 on (15/15 faster). Hint/original approximately neutral: 0.9966 off (9/15 faster), 1.0186 on (8/15 slower), despite lower aggregate medians. No blanket regression-free/general contention conclusion. |
| Root regression verdict preserves first failure and recovery | `830752ekl2` full-suite original output and automatic isolated rerun; deliberate `887110wsq1` | Check/lint PASS. Full suite reported 559 passed/0 failed/3 ignored **after** one parallel bundle ELF-loader test hit ETXTBSY and automatic isolated rerun passed. Explicit deliberate isolated rerun then passed one test. This is recovered success, not pure first-pass success. |

The three new actual-source checks are `stale_true_hint_does_not_authorize_a_grant`, `stale_false_hint_uses_original_registration_and_rechecks_idle`, and `advisory_hint_release_acquire_races_conserve_allowance_in_both_event_modes`. The prior fifteen source tests remain unchanged.

## Final matched production-module measurements

`bench/allowance-tickets/fastpath/hint/results/threeway-015.json`, SHA256 `91f8b1391cb31320908a43036467b731ceb3e3cc5792ef24048a4a15bc1c39b3`. Task `705228qyre`: 112.76 s, October 14, 2026 21:01:45–21:03:37 UTC, after coherent native task ended at 20:58:53 UTC. Original raw 180 batch hashes, unrounded summaries and paired comparisons are retained in slim JSON. Preliminary threeway samples are separate and excluded.

Rounded aggregate median elapsed milliseconds, measured together in this cohort:

| Scenario / event storage | Original `96d803` | Prior fast `1233aa` | Hint `f3849e` |
|---|---:|---:|---:|
| Idle / off | 1.073451 | 0.273217 | 0.323714 |
| Idle / on | 1.205256 | 0.359495 | 0.398261 |
| Natural capacity 1 / off | 274.841013 | 288.706223 | 273.728690 |
| Natural capacity 1 / on | 274.421025 | 280.020776 | 280.244090 |
| Natural capacity 4 / off | 177.024627 | 190.520716 | 170.861729 |
| Natural capacity 4 / on | 185.876300 | 204.724416 | 176.426170 |
| Forced pending capacity 1 / off | 20.951721 | 21.442925 | 21.711507 |
| Forced pending capacity 1 / on | 37.331013 | 39.665145 | 38.414608 |
| Forced pending capacity 4 / off | 15.925953 | 16.283987 | 15.828140 |
| Forced pending capacity 4 / on | 24.316451 | 23.561635 | 22.966841 |

Median of within-block hint/comparator ratios is a different statistic from ratio of aggregate medians. Values below accompany actual slower-block counts out of 15, rather than treating small aggregate differences as robust wins:

| Scenario / storage | Hint/original paired ratio; slower blocks | Hint/fast paired ratio; slower blocks |
|---|---|---|
| Idle / off | 0.3107; 0/15 | **1.2445; 13/15** |
| Idle / on | 0.3365; 0/15 | **1.1312; 14/15** |
| Natural capacity 1 / off | **1.0294; 8/15** | 1.0022; 8/15 |
| Natural capacity 1 / on | **1.0388; 9/15** | 1.0247; 9/15 |
| Natural capacity 4 / off | 0.9966; 6/15 | 0.9273; 3/15 |
| Natural capacity 4 / on | **1.0186; 8/15** | 0.9216; 0/15 |
| Forced pending capacity 1 / off | **1.0295; 8/15** | 1.0182; 8/15 |
| Forced pending capacity 1 / on | 0.9972; 6/15 | 0.9318; 4/15 |
| Forced pending capacity 4 / off | 0.9250; 6/15 | 0.9940; 7/15 |
| Forced pending capacity 4 / on | 0.9787; 7/15 | 0.9999; 7/15 |

The focused idle improvement versus original is consistent across the measured blocks. Recovery versus prior fast in natural multi is also visible. But paired comparison against original is approximately neutral/mixed for natural multi, and natural-one/pending-one tradeoffs remain. No robust natural-multi win versus original or blanket regression-free assertion is justified. Quiet status is unknown, affinity 0–15 inherited/unpinned on a heterogeneous CPU, loads recorded and outliers retained. These caveats do not erase observed regressions and do not establish their cause.

Natural workloads use eight threads, capacities one/four, 80,000 pairs; idle uses 10,000 pairs. Forced-pending timers include barrier/channel coordination and private snapshot cloning, not pure atomic/lock/allocation attribution. Event-buffer-off/on measures row-storage cost only; optional IDs/counters remain. Enabled long traces are lossy after saturation. Timed callback snapshots can be between supply and completion, whereas surplus cases explicitly wait for completed callbacks. No comprehensive session-weighted compiler/build improvement is measured here.

## Exact provenance and regression receipts

- Full sources: original `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18`; prior fast `1233aa16d73ae11cf447ab5bac0abd2adf48efcb2873574fb08e4e9428f7e631`; hint `f3849e80b7a1f72ee9472592d66a57a83477900d52354a772ac345c77f07def8`; jobserver.rs `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7`. Archives are under `bench/allowance-tickets/fastpath/hint/archive/{original,fast,hint}/`.
- Coherent actual native driver: `925c7892da0e7f00706326c9a77a3a20d887cf57706529b19f58e54575274bf9`; launcher: `95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba`. Source/driver manifests and five log hashes are in slim JSON, without publishing the 21.7 MB wait log wholesale.
- Actual legacy callback: `query/native-task-binding-ean5kw_j/capture/1792011500506452929-1570655-1.json`, SHA256 `34ceb39d295b31998f3a70c910b34b2d8ed9bb738e4c1548f32f7823b9996c08`. Artifact hash `5af443a848e719025d7c5c38ff489a67dbd59400d6fe328082844d331b67d717` is per-run evidence, not cross-cohort byte-stability authority.
- Shared benchmark harness: `de91c937a7b92b24817d5dce6067768be518b6b1514aaa99df150b862e29f21d`; matched lockfile: `cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236`. Benchmark compiler and executable hashes are separately retained from actual native compiler provenance. Same-stage off/on uses the same production executable. Local compiler binary pin plus hosted version stamp does not establish hosted ELF identity.
- Sheep's actual-source plus integration tests total original 12, fast 16, hint 19; hosted checks/release/clippy deny-warnings passed. These are 11/15/18 source tests plus one workflow, not native Registry integration. Reviewer mouse's clearance is source-only, not execution evidence.
- Root check `20261014-140351291-1636352`; recovered full suite `20261014-140406450-1639310`; all-target deny-warning lint `20261014-140417870-1636246`. Original failure: `toolchain::bundle::tests::executable_with_missing_elf_loader_is_rejected_without_compiling`, executing bundle rustdoc `--version`, Text file busy (os error 26). Automatic isolated rerun passed. Deliberate isolated task `887110wsq1`, run `20261014-140447126-1646208`, passed one test/0 failed/525 filtered in 2.11 s. Preserve this recovery history; do not silently call the full suite a clean first-pass run.
- Coordinator published 20 exact cited path-redacted summaries in `data/runs/allowance-hint-20261014/2026-10.jsonl`: four sloth, twelve fixture, four root/isolated. Export ownership is separate from this worker's two new files.

## Remaining gates and shipment boundary

Any scoped source shipment is only the targeted idle-path change in an **unwired** experimental protocol, with these contention tradeoffs explicitly retained. It does not change legacy frontend default or authorize general/default performance promotion. Real TicketedProxy Registry/query startup/sleep/cycle/reacquire/shutdown, fairness/reservation convoy and resource/query linkage are future gates, not supplied by nine legacy compiler fixtures.

Actual callback already in-flight teardown, actual Client helper IO/callback/raw-release failures rather than direct error-state injection, resource wire/reciprocal attach/checkpoint/seal and complete independent receipt authority remain unestablished. No held lease/thread/physical-token ownership, full map/full-pipeline graph, thread ceiling or CPU work/span is claimed. Comprehensive session-weighted and relevant cold/incremental/contention gates are required before broad performance/default conclusions.

This worker created only this new document and slim JSON. No builds, existing-file edits, historical-artifact modifications or commits were performed by this worker.
