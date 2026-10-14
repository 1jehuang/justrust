# Native allowance-ticket prototype results, 2026-10-14

**Accepted only as an experimental, unwired A/P protocol prototype.** TicketedProxy is compiled and directly tested, but the frontend still selects legacy Proxy. Existing native compiler/query harnesses prove legacy-path neutrality, not TicketedProxy Registry/query integration. No speedup, default promotion, full mapping, thread ceiling or CPU work/span claim is made.

Slim, path-redacted evidence and original SHA256 values: `data/runs/allowance-tickets-20261014/native-acceptance.json`. Scratch paths are relative to `scratch/allowance-native-acceptance-20261014` under the agent scratch directory. Benchmark paths are repository-relative. Source inspection and actual test execution are separately attributed.

## Requirement → check → observed result

| Requirement | Check / observable evidence | Observed result and scope |
|---|---|---|
| Preserve existing compiler default | jobserver.rs only adds module/reexport prefix; frontend still constructs legacy Proxy | Baseline legacy body SHA256 `941b83d84a8c29b3f0d6267b12120787a765ee6f18055e7bf5666ddb23f8684b` unchanged. No frontend selection or promotion. |
| Exact recipient predicate, not notification authority | Native `ungranted_notification_does_not_return` and `actual_target_grants_survive_reverse_ack_order` | Selected request checks Granted and acknowledges under owning State. Spurious wrong-request notification cannot return. W2 acknowledges before paused W1 while named release/helper grants remain attached to the original requests. |
| Native identity independent of telemetry IDs | Stable Arc request slots, private targeted Condvar waiting on owning State; recorder exhaustion test | Optional telemetry IDs may exhaust without controlling admission. Slots allocated before State lock; intrusive queue avoids queue-growth allocation there. |
| Aggregate conservation includes reservations | Actual helper-first, local-first and race tests; matched harness counts | used includes granted-but-unacknowledged requests. Transfer and Ack preserve used; accepted helper supply increments, nontransfer release decrements. All 120 candidate benchmark samples have request=grant=ack=completed pairs, releases=pairs+1 bootstrap, final used=pending=0. |
| Exact operation source, unknown holder | Release/Helper/IdleImplicit grant and Ack events | Source names protected release operation or accepted helper callback, not releasing-thread ownership, held lease, physical jobserver token or a sibling process donor. |
| Real helper supply and surplus | `local_transfer_surplus_and_retained_implicit`, `helper_first_release_preserves_reserved_grant`, race test | Uses real Client and held Acquired handles. Local-first surplus is reacquired through Client, not inferred from Drop telemetry. All 30 candidate benchmark surplus samples have supplies=callbacks_completed=1 and available_after=1. Enabled surplus traces explicitly say granted:false. |
| Lock-safe token operations | Read-only source review plus real module tests | Raw token consume/surplus Drop and release_raw occur outside State. Callback captures State, not Proxy, avoiding helper→Proxy ownership cycle. Reviewer mouse found no substantive blocker in frozen narrow protocol; review itself ran no tests. |
| Checked native counts and failure-atomic queue | `checked_native_ceiling_does_not_mutate_pending_and_surplus_is_safe`, `queue_count_boundary_rejects_before_mutating_links` | Native usize boundary rejection precedes pending pop/grant or queue-link mutation. This is an invariant check, not recorder loss or a production small admission cap. |
| Event loss cannot grant or deny admission | `recorder_cap_and_identity_overflow_do_not_control_admission`, event-disabled/enabled inventory comparison | Native admission completes with capped rows/overflow. Event-buffer-off still performs optional identity allocation and independent counters. It is NOT fully zero-telemetry capture-off. Enabled buffer has 4,096 rows and retains loss counts after saturation. |
| Error is not a grant | `error_is_not_a_grant` | Direct State::error injection leaves request pending with no grant. This is NOT actual Client/helper IO-failure validation. |
| Bounded helper shutdown | `dropping_proxy_interrupts_outstanding_real_helper_request` | Drop with helper blocked acquiring after local transfer passes. Callback already in-flight while Proxy drops remains untested and is a separate future gate. |
| Actual native crate executes prototype | Corrected stage1 rustc_data_structures release-profile test task `341800bv8v` | PASS: 11 tests, 0 failed, 0 ignored, 118 filtered; 15.2 s coordinator wall time. Prior `3355241u1v` failed wrong-cwd preflight and is superseded, not a protocol failure or passing run. |
| Existing frontend semantics remain unchanged | Native build plus admission/wait/task-binding/compiler-binding/macro harnesses `8934935a2k` | PASS: five harnesses, nine tests total. 436.3 s combined task wall time; build log reports 6m25s for build alone. These tests run legacy Proxy, not ticketed Registry/query resource integration. |
| Historical real query points remain independently bounded | Actual callback receipt and schema4 consumer | 33,316 expected/actual points, zero binding/read loss, 8 registered/resumed wait pairs. all_targets_bound=false. Counts are actual schedule-dependent observations, not fixed deterministic invariants or allowance-resource linkage. |
| Matched dependency and sources | Archived real modules via include!, shared harness, exact jobserver=0.1.34 lock | Matched comparison has 15 rotating rounds, 45 batches, 180 samples, no failures/timeouts. Old 0.1.35 comparison is explicitly superseded and excluded from conclusions. |
| No oversubscription / exact work | 8 threads, capacities 1/4, 80,000 completed pairs; 10,000 immediate pairs | Every sample exits zero and completes exact operations; contention peak_active≤capacity. Baseline exposes external counts only and has no surplus callback acknowledgment oracle. |
| Root client regressions | Hosted fmt/check/full tests/all-target lint `382669zlwm` | PASS: 559 tests, 0 failed, 3 ignored. This gate is separate from native prototype tests and does not establish frontend wiring. |

## Matched focused cost, not a speed or default gate

Source `bench/allowance-tickets/results/comparison-034.json`, SHA256 `cf6ba0ac4b1e67b9b31d553335f0b77f292aacbb9f09b8f063976373f9868cd9`. Exact 45 batch paths/hashes, provenance, assertions and unrounded nanosecond summary are retained in slim JSON and original comparison. Commands and runner receipts are in `bench/allowance-tickets/README.md`.

| Variant | Capacity 1 median ms | Capacity 4 median ms | Immediate median ms |
|---|---:|---:|---:|
| Matched legacy baseline | 286.793 | 165.949 | 0.277 |
| Ticketed, event-buffer-off | 314.302 | 201.659 | 1.244 |
| Ticketed, event-buffer-on | 311.409 | 204.504 | 1.308 |

Every candidate median is larger than baseline in this focused run. No faster/default promotion conclusion is supported. Event-buffer-off/on isolates row-buffer cost only, since both retain IDs/counters. Tiny work sections, OS scheduling noise and large outliers are retained. Timed multi-contention snapshots can observe supply before callback completion and are not quiescent seal evidence. Baseline surplus setup/availability cannot certify callback completion; candidate surplus assertions can. Fairness, reservation convoy and optimal policy are not measured by throughput/count success.

Benchmark compiler is nightly commit `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, separate from rebuilt native stage1. Local pinned compiler binary SHA256 and matching hosted artifact version stamp are provenance, not proof of identical hosted ELF bytes. Baseline artifact SHA256: `e7a7fd459d3275c2f6997f664396e45d2e2547471e816cd88fc7191f679daf40`; both candidate event modes use `4c46fb86ee8c79c4d3d0cd03b58df23336be25d5d2e7ce366624777ee06ab543`.

## Frozen source and actual native evidence

| Evidence | SHA256 |
|---|---|
| jobserver.rs including module/reexport prefix | `5f5d43e64ced6353dfefcbf12e8cb6b135c7aa4e43135985b2c74b11aadd5fc7` |
| jobserver_ticketed.rs | `96d803532813f0cd7e27c704ca5ed84eaf8a145995e3111b354586a095c0cc18` |
| Matched lockfile | `cb87ed9dad272415aca309ec0fd12b26e43b380d7a7185c8db1b08ab36c76236` |
| Shared actual-module benchmark harness | `5436794490ce3a0502dcf52ba38e765f5992962d27bc2167f17e572457efe439` |
| Actual native stage1 driver | `94f8d6443482ad714acbf728b815032a8869976da2c7612761db7c7a7b43ad0f` |
| Actual native launcher | `95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba` |
| Legacy callback receipt | `7b1787cb8d40a9e5042564488b320ac82651569f0f51f98af534048cd6965d66` |

Compiler/source manifests: `source-before.sha256`, `compiler.sha256`. Actual callback: `query/native-task-binding-n7np45uo/capture/1792009310636653698-1165393-1.json`. Huge combined native output is identified as `task-output/8934935a2k.output` with original hash in slim JSON, not republished wholesale. Source reviewer mouse inspected pinned jobserver 0.1.34 helper callback/error/Drop behavior and cleared only the narrow source protocol. Sheep's actual-module benchmark is separately attributed from main's actual native crate/compiler execution.

Root exact run IDs: check `20261014-132303169-1180201`, full tests `20261014-132304295-1180347`, all-target lint `20261014-132315425-1180084`. Candidate benchmark hosted receipts: check `20261014-131523267-1085919`, test `20261014-131525176-1086123` (12 tests: 11 included-module tests plus integration workflow), release `20261014-131527584-1086391`. Coordinator owns export subset publication, not this document worker.

Complete native rustc_data_structures gate `5016119dkd` also passed: **129 tests, 0 failed, 0 ignored, 0 filtered**, plus **5 doctests passed, 1 ignored, 0 failed**, 11.9 s coordinator wall time. This is distinct from the focused 11-test gate. Coordinator published 20 exact cited path-redacted summaries in `data/runs/allowance-tickets-20261014/2026-10.jsonl` (nine sloth, eight sheep, three root), outside this worker's ownership.

## Remaining acceptance gates

1. Callback already in-flight teardown owning Acquired/State while Proxy drops, distinct from blocked-helper cancellation.
2. Actual Client/helper IO failure and raw-release outcome handling, distinct from direct error-transition injection.
3. Explicit ticketed frontend/Registry wiring, startup/idle/termination/external installation, query cycle/error/reacquire/shutdown, reservation convoy and fairness. A descheduled selected recipient can retain a reserved grant; no revoke/steal policy is established.
4. Separate resource wire format, reciprocal namespace attachment/checkpoint/seal, independent complete resource receipt authority and explicit query-wait→release/request/grant linkage. Existing schema4 points are not such linkage.
5. Comprehensive session-weighted before/after build measurements and relevant cold/incremental/contention correctness gates before any performance or default decision. Focused module costs are informative, not that gate.

No held-lease/physical-token ownership, complete resource map, full-pipeline graph, thread ceiling, CPU work/span or compiler speedup is established. Publication here creates only this document and its slim acceptance JSON, with no builds, existing-file edits or commits by this worker.
