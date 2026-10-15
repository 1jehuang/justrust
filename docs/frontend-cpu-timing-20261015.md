# Frontend exclusive CPU timing, 2026-10-15

## Purpose and boundary

The dependency mapping milestone established actual query/task bindings and honest loss accounting, but task start/finish elapsed time is not CPU work. A task can wait, execute nested work, help another query, or be temporarily replaced by generic pool dispatch. Inclusive guard CPU deltas would double count that work or attribute it to the wrong task.

This diagnostic mode records **observed exclusive thread CPU segments**, not total frontend CPU, a weighted semantic dependency graph, a critical path, or a theoretical thread ceiling. The separate cold query-call provenance gap remains documented in `cold-query-provenance-gap-20261015.md`.

## Native protocol

Timing is explicitly enabled through `QueryAdmissionObserver::new_with_event_capacity_and_timing`. Default constructors retain query schema 4 and pool schemas 1/2 without timing fields. The native fixture carries its opt-in through an owned inherited descriptor consumed before compiler entry, not compiler arguments or environment visible to compilation.

Opt-in query schema 5 and pool schema 3 endorse the same four-field clock descriptor: query capture ID, pool capture ID, pool observer ID, and `pool_epoch_in_query_ns: 0`. The pool receives the query's actual `Instant` origin. Matching nearby timestamps or just matching old capture IDs cannot authorize a clock conversion. The strict consumer requires reciprocal descriptors and validated binding/lifecycle authority. A pool-only CPU assessment does not independently claim query clock alignment.

One thread-local cursor is shared across observers on each worker. Entry, restoration, generic dispatch, forks, and continuations close the previous exclusive segment before starting a replacement. Timed work entering a different or unobserved registry cannot continue charging its source task. Opaque dispatch is separately labeled rather than inventing task ancestry. Thread IDs and segment IDs have independent checked allocation and loss accounting.

Linux thread CPU samples use `CLOCK_THREAD_CPUTIME_ID`. A start wall sample precedes the start CPU sample, and an end wall sample follows the end CPU sample. Each delta is taken between samples on the same thread. A task aggregate is a sum of those deltas, never a subtraction of counters from different workers. Existing worker-local `Fork`/`TaskGuard` ownership is preserved. Cross-worker child execution is not proof of same-task migration.

Instrumentation and uncovered intervals remain outside the measured scope. Default no-CPU-counter behavior is supported by source inspection showing no recorder/cursor creation and legacy receipt replay, not a separate native counter-interposition test.

## Consumer safety

The consumer checks original typed JSON, exact versions and required nulls, reciprocal qualified identities, independent allocator inventories, lifecycle containment, per-thread wall/counter nonoverlap, and per-task wall nonoverlap across threads. Two threads cannot both charge overlapping execution to one task. Opaque work can execute concurrently.

Clock failures, explicit regressions, wall failures, active segments, loss, and identity exhaustion remain visible. Ownership-integrity failure, including the native sticky TLS failure flag, suppresses numeric aggregates. Explicit counter regression or an incompatible CPU/wall envelope yields unknown aggregate CPU, not a repaired or clipped number. The clocks have no declared exact cross-clock error tolerance, so such an envelope is not silently accepted as quantitative CPU evidence. Ordinary retained-row loss can retain a clearly scoped measured sum while completeness remains false.

`observed_cpu` is separate from `cpu_work_ns` and `execution_span_ns`. The latter stay null, as does theoretical parallelism. Even complete retained CPU segments do not prove complete semantic/control dependency coverage.

## Client validation

The strict consumer and graph integration passed hosted check `20261014-181930580-10323`, 169 targeted graph tests `20261014-181933786-10498`, warning-free clippy `20261014-181940956-11523`, and a clean first full suite `20261014-181945238-10320`: 621 passed, 3 ignored, no failures or retries. `justrust show` confirms the full suite ran remote on hosted. Slim summaries are in `data/runs/frontend-timing/2026-10.jsonl`.

The analyzer passed 28 Python tests, including an exact legacy report byte comparison. Those analyzer tests use a mocked validator and are not native acceptance. Client commits are `67f1ef504` and `03d84b93c`.

## Native and real-workload acceptance

The coherent compiler build passed in 575.82 seconds. Earlier integration attempts were retained: deprecated atomic method warnings were denied, a clock test seam lacked `RefUnwindSafe`, and its production trait object did not satisfy rustc's private dynamic-thread markers. The final implementation uses a concrete production clock and a test-only dynamic fake clock, without unsafe marker assertions.

Before native tests could rebuild private metadata, stage1 and the matching full stage0 sysroot were copied into the immutable local snapshot `native-frontend-timing-accepted-20261015`. Its driver SHA-256 is `ec9f912a22ee163833d3e7e08566d49322ca03939bb4fb46d4d01e5d34b02f36`. The installed acceptance client SHA-256 is `a22a1bd38f4b0ded1d70d0aabe05734412e8599bf47f264594e253ddd1fe32d1`.

Actual native tests passed in 101.07 seconds: 216 passed, 28 ignored, no failures across the selected session, query implementation, thread pool and integration binaries. The 10 new timing tests passed, including fake-clock failure/exhaustion controls, a real CPU-target versus sleep test, nested foreign observer restoration, dispatch unwind, fork continuation, and real generic dispatch on an observer-free registry. Source hashes before and after the run matched. Logs remain in the local scratch directory `native-cpu-timing-20261015`.

The exact real-source suite passed four tests in 46.088 seconds with no skips. It compiled untouched locked registry `itoa 1.0.18` using the immutable runtime and matching private metadata. Default serial/four-thread cold and incremental captures, explicit large captures, opt-in timed cold/warm captures, and foreign descriptor safety all passed. The mandatory legacy query4 replay preserved original raw hashes and every old assessment field. A unilateral clock descriptor mutation was rejected or remained unaligned without changing the independent pool CPU facts.

OFF/ON/OFF cold whole-rlib and linked-consumer bytes matched. Warm rmeta/object payload and linked-consumer bytes/output matched. As in the earlier milestone, warm whole archives/link metadata are unstable even under OFF/OFF controls, so no whole incremental archive byte-neutrality claim is made.

The retained timed cohort is `frontend-timing-real-final-20261015/frontend-real-itoa-56qsp7oz` under the local scratch directory. Committed reports are `bench/results/frontend-timing-20261015/cold.json` and `warm.json`, in commit `fe81b7c2d`. Native instrumentation and its real-source harness are in `9bb5f86ea`. These are observations from one actual four-thread capture per mode, not benchmark before/after speedup evidence.

Replay the cold analysis on this machine with:

```sh
python3 bench/frontend_mapping.py \
  --capture "$JCODE_SCRATCH_DIR/frontend-timing-real-final-20261015/frontend-real-itoa-56qsp7oz/timed-cold-t4-1-capture" \
  --justrust /home/jeremy/.cargo/bin/justrust \
  --large-capture --output "$JCODE_SCRATCH_DIR/replayed-timed-cold.json"
```

Use `timed-warm-t4-1-capture` and omit `--large-capture` for the strict-budget warm replay. The original multi-hundred-MiB receipts remain local retained evidence, not repository fixtures.

| Scope | Measured segments | Observed threads | Observed CPU | Task-owned CPU | Opaque CPU |
|---|---:|---:|---:|---:|---:|
| Cold | 360 | 3 | 198.098740 ms | 66.142860 ms | 131.955880 ms |
| Warm incremental | 131 | 3 | 38.733401 ms | 37.068743 ms | 1.664658 ms |

Both captures had zero failed CPU segments, no timing issues, and validated reciprocal clock alignment. This is complete retained **segment** accounting, not complete frontend execution coverage. Cold bindings still include 392,883 `no_active` points out of 485,881 total, despite zero read/binding record loss. Its 217,520 direct dependency reads remain disabled and resolve to zero publishing-job edges. Warm bindings have 2,572 `no_active` points out of 19,596. No missing task ancestry is synthesized.

## What this tells us next

About two thirds of the **observed cold CPU** is opaque rather than assigned to a known pool task. That is an actionable attribution gap: improving generic-dispatch task ownership and actual query-call provenance is more justified than claiming a low thread ceiling from the small known task graph. It does not prove that two thirds of all frontend CPU is opaque, or that this work can safely be parallelized.

Warm observed CPU is mostly task-owned, but task ownership alone does not establish the semantic prerequisites or complete control graph needed for a critical path. The next substantive graph change is the explicit request/result dispatch protocol described in `cold-query-provenance-gap-20261015.md`, followed by representative larger-crate captures and CPU-weighted dependency coverage. Architecture experiments must compare actual work and latency under the same workload, not optimize the completeness percentage or equate pool occupancy with speed.
