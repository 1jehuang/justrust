# Observed frontend wait hotspots, October 15, 2026

## Scope and authority

This is a **single-capture diagnostic**, not a compiler bottleneck verdict, CPU
profile, execution span, or parallelism ceiling. Evidence is in
`bench/results/frontend-mapping-20261015/wait-hotspots.json`, derived from the
same original receipt as `final-cold-large.json`. The original query receipt's
SHA-256 is `a6c5b4d92e7b95de5a99edf360893763f015820f853234d76add4ab10570ec81`
(223,973,636 bytes). This is the original 79-wait cold capture, not the separate
fresh full-suite rerun with 45 waits. Capture-local numeric IDs are diagnostic
labels, not stable query identities. No raw query names, source paths, or query
keys are included.

The installed public `justrust graph --json --large-capture` consumer revalidated
both original receipts: query structural/wait coverage complete, pool structural
coverage complete, and neither assessment had issues. Task bindings are valid
but incomplete, with `task-binding targets declared incomplete` retained. The
previous consumer aggregate-budget rejection remains a historical failure, not
a result silently relabeled as success. Acceptance here uses the repaired
consumer on the same original bytes.

## What the wait intervals show

All 79 waits have notification reason `completed`, covering 48 consumer jobs and
58 waitee jobs. Stages use **only the query receipt's epoch**. Summed elapsed
intervals overlap across waiters and include waiting, scheduling and other work.
Neither their sum nor union measures CPU work or time recoverable by an
optimization.

| Observed interval | Sum of elapsed intervals, ms | Union of intervals, ms | p50, us | p90, us | p95, us | Maximum, ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Registered to notified | 17.820341 | 16.008386 | 14.917 | 282.623 | 425.052 | 6.784256 |
| Notified to condvar returned | 0.580556 | 0.443262 | 5.465 | 14.052 | 18.747 | 0.023626 |
| Condvar returned to resumed | 1.821053 | 1.392027 | 15.288 | 45.773 | 61.103 | 0.103311 |
| Notified to resumed, containing the preceding two stages | 2.401609 | 1.730277 | 22.129 | 53.960 | 71.236 | 0.110736 |
| Registered to resumed, total | 20.221950 | 17.608946 | 42.156 | 327.511 | 487.828 | 6.887278 |

Quantiles are nearest-rank: sort the 79 durations and select index
`ceil(p * 79) - 1`, zero-based. Intervals are half-open. The total has a peak of
three overlapping waits. Unions were independently recomputed by merging sorted
intervals and compared with the analyzer's endpoint sweep. The three disjoint
stage sums equal the total sum. Their unions must **not** be added: intervals
from different waiters can overlap across stage classes. The notified-to-resumed
row already includes the two stages above it.

Registered-to-notified contributes approximately 88.1% of the **summed wait
interval durations**. That is a useful place to investigate notification
readiness, not evidence that 88.1% of runtime is producer computation.

## Narrow hotspots without query names

| Wait ID | Total elapsed, ms | Registered to notified, ms | Condvar returned to resumed, ms |
| --- | ---: | ---: | ---: |
| 71 | 6.887278 | 6.784256 | 0.079841 |
| 70 | 4.087154 | 4.018171 | 0.061103 |
| 78 | 2.372554 | 2.318594 | 0.045773 |

These three account for 13.346986 ms, approximately 66.0% of summed wait
interval durations. Their registration and resumption task targets are all
`no_active`, so the evidence cannot locate them on a validated pool task or
attribute their elapsed durations to provider CPU. Query kind/category is absent,
so calling these type-checking, trait-solving or another semantic wait type
would be invented. The only recorded notification category here is `completed`.

There is also a shared-waitee example: job 2059 has waits 28, 29 and 30 from
consumers 2047, 2049 and 2043. Their total durations are 471.500, 482.035 and
487.828 us. The sum is 1.441363 ms, while their union is only 0.497013 ms. This
is observed fanout to one waitee, not three additive wall-time bottlenecks.

## Actual provenance chains and missing links

Using only validated wait endpoint edges, the longest observed job chain has
two edges: `6898 --wait 72--> 6901 --wait 73--> 6881`. Wait 72 runs from
76.502891 to 76.579933 ms and wait 73 from 76.512786 to 76.636671 ms in the
query epoch. The waits overlap: their summed elapsed duration is 200.927 us,
while their union is 133.780 us. This is a chain in the **observed wait-only
graph**, not the execution critical path. Admission parent relationships are
descriptive and are not added as semantic edges.

Of the 79 waits, 76 have `no_active` at both task-binding endpoints. Only waits
75 and 76 bind both endpoints to pool task 1, and wait 79 to task 7. Valid pool
control provenance separately contains `2 -> {3,4} -> 5` and
`1 -> {6,7}`, `7 -> {9,10} -> 11`, `{6,11} -> 8`. These are continuation
relationships, not semantic query dependencies. Pool and query timestamps use
separate epochs and have not been subtracted or merged.

All 217,244 observed direct reads are explicitly `disabled`: no tracked reads
and no resolved read-provenance edges. Read event loss is zero, which establishes
record coverage, not semantic dependency coverage. Neither a cold disabled read
nor a publishing-job record proves a fresh provider execution dependency.

### Completion marker boundary

For every wait, the recorded waitee `completed` marker is **later** than its
`notified` timestamp, by 1.631 to 9.288 us. This agrees with the current hook
placement: `QueryWaitToken::notify` records immediately before notification,
and `ActiveJobGuard::complete` performs latch signaling through
`drop_and_maybe_poison` before `admission.complete()` records job completion.
Relevant sources are `toolchain/rustc/compiler/rustc_session/src/query_admission.rs`
and `toolchain/rustc/compiler/rustc_query_impl/src/execution.rs`.

Therefore the admission completion marker is not the instant provider execution
ends. Do not subtract it from notification to invent a production or wakeup
latency. Likewise, condvar-returned to resumed can include allowance reacquisition,
descheduling and instrumentation, not just a resource-grant cost.

## Next telemetry needed to turn observations into bottleneck diagnosis

1. **Cover the unbound wait sites natively.** Preserve attachment, observer,
   task/continuation identity and explicit unavailability reasons through query
   execution, without inventing roots or binding nonquery work by proximity.
   The three longest waits presently lack task authority.
2. **Privacy-safe query categories and actual provider boundaries.** Add bounded
   query-kind/category IDs without query keys, symbols or paths. Distinguish
   request start, cache/ensure/feed outcome, fresh provider execution, result
   readiness/publication, latch notification, and terminal bookkeeping. Preserve
   exact clock definitions and causal IDs.
3. **Read semantics independent of cold depgraph tracking policy.** Obtain
   consumer-to-result/provider provenance even when depgraph reads are disabled,
   without changing compilation semantics or inferring edges from request nesting.
   Preserve duplicates, missing context and opaque outcomes explicitly.
4. **Separate scheduler/resource states.** Native request/wait/grant/release and
   runnable/dispatch events can distinguish readiness, resource reacquisition
   and descheduling. They need allocator-backed identities and loss accounting.
   Add a shared epoch or validated clock mapping before combining timelines.
5. **Actual CPU telemetry and controlled tests.** Per-thread CPU clocks and
   validated execution ownership can distinguish CPU consumption from elapsed
   intervals. Parallel work during waits, nonquery work and compiler phases still
   need coverage. Measure recording overhead and replay/compile neutrality, then
   compare repeated controlled runs and the comprehensive session-weighted
   workload before claiming a bottleneck removal or speedup.

No CPU work, full execution span, theoretical parallelism, optimization ceiling,
or measured speedup is claimed by this evidence.
