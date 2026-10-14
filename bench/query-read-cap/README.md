# Focused query-read cap diagnostic, 2026-10-14

This is an actual-module standalone diagnostic, **not a whole-compiler benchmark,
not the session-weighted broad gate, and not evidence for default promotion or a
general/native compile-time speedup**. No configuration defaults were promoted.
The native compiler acceptance requirement is separate from these unit tests.

## What is measured

`harness.rs` compiles the owned `query_reads.rs` directly through `#[path]`, not a
reimplementation. Feature `baseline` selects the byte-identical pre-edit snapshot
`baseline-query_reads.rs` (SHA256 `6c8eebbc2796dff1f6a55a2f3f60867122283c8b43e65ddb62cd937d829efad7`).
The original nine tests and initial baseline timings completed before any owned
source edit. The snapshot retains the source's existing licensing.

Both binaries used justrust's pinned `jr-ac20de5e7921` compiler,
`rustc 1.101.0-nightly (ac20de5e7 2026-10-11)`, LLVM 23.1.3. The system `rustc
1.98.1` metadata is explicitly labeled `system_rustc`, not the builder. The build
commands, hashes, compiler version, and common observed compiler options are in
`provenance.json`. Six cited slim build/test run records, exported to scratch and
filtered to this task only, are preserved in `build-runs.jsonl`.

Each capped case pre-fills 100,000 real `result_produced` events, then times
30,000,000 rejected events through the public API. `read` records reads only.
`mixed` cycles request start, cache hit, bound read, result, and request finish.
`accepted` times 100,000 mixed events on an initially empty recorder. Timing
includes the barrier release and worker joins but excludes prefill, serialization,
and assertions. Every sample asserts the exact loss count, independent request,
read, and result expected counts, accepted event count, and completeness verdict.

The Dell XPS14 host's initial sweeps allow all 16 CPUs and are not interleaved.
The follow-up runs three explicit ABBA rounds per case, with six samples per
binary, immutable copied binaries, CPU0 for one-thread cases, and CPU0..7 for
eight-thread cases. These are CPU-affinity IDs only, not asserted P/E core classes.
**No quiet-system guarantee**: other work was active. Fixed affinity is not CPU
isolation, fixed frequency, or thermal control.

## Observations, not a broad speed claim

Initial separate sweeps (`before.json`, `after.json`), three samples per case:

| Case | Before median seconds | After median seconds |
|---|---:|---:|
| capped reads, 1 thread | 0.914685 | 0.363507 |
| capped mixed, 1 thread | 0.699351 | 0.925979 |
| capped mixed, 8 threads | 5.425539 | 3.187312 |
| accepted mixed, 1 thread, 100k | 0.012026 | 0.005100 |

The initial mixed one-thread result was a regression, not discarded. It motivated
an explicit alternating fixed-affinity follow-up (`paired.json`):

| Case | Before median [min, max] seconds | After median [min, max] seconds |
|---|---:|---:|
| capped reads, 1 thread | 0.665844 [0.539419, 0.732509] | 0.380712 [0.328605, 0.552349] |
| capped mixed, 1 thread | 0.687678 [0.646928, 0.754316] | 0.389319 [0.368956, 0.441447] |
| capped mixed, 8 threads | 5.117650 [4.790810, 5.509319] | 2.100425 [1.953250, 2.609734] |
| accepted mixed, 1 thread, 100k | 0.004889699 [0.004629720, 0.005411912] | 0.004872339 [0.004517234, 0.005913258] |

The one-thread regression did not reproduce under ABBA with fixed affinity.
Scheduling, differing core-frequency limits, and contention are plausible
explanations, not proven causal attribution. The accepted-event controls overlap. These measurements
justify investigating the opt-in saturated path, not a native/general speed claim.
No `bench/suite.py` comprehensive comparison or default-promotion gate ran.

## Correctness and limitations

- The monotonic `capped` flag is published only after accepting event 100,000,
  while holding the existing mutex. It never reserves event positions or moves
  ID allocation. Requests, reads, and results still allocate independent IDs in
  the public call paths, including rejected events and RAII interruption.
- Accepted events still append under the same mutex. No FIFO or global ID order
  was promised by the old mutex, and no such guarantee is added. Their existing
  append/serialization order is unchanged. Boundary races retain exactly 100k
  events, with the pre-existing prefix unchanged.
- A stale relaxed `capped=false` takes the original lock and rechecks length. A
  `true` never reads the vector. Mutex ordering still protects accepted contents.
  There is no new buffer or per-thread storage. Vector capacity no longer grows
  after capping. Existing allocator over-allocation is retained, not an exact
  100k-slot allocation guarantee.
- Allocation reservation failures still increment loss and allow later attempts.
  They do not seal the observer. This policy is verified by source inspection,
  **not allocator-failure injection**. Process-level OOM behavior is unchanged.
- Ordinary unpoisoned loss is exactly one per rejected event. The existing poison
  bookkeeping is deliberately retained: one extra per poisoned observation and
  one publication marker. Five already-poisoned capped rejections produce ten
  stored losses and eleven serialized losses.
- `is_poisoned` can race a concurrently panicking mutex holder. Unlike the old
  lock path, the fast path does not wait for that holder, so identical historical
  poison-observation overcounts are **not** guaranteed during that race. Each
  actual rejection is still counted once, and publication detects poison and
  fails incomplete.
- The loss overflow latch uses one `fetch_add` and a rare overflow branch, not a
  CAS retry loop. If the u64 loss counter wraps, the latch permanently prevents
  `records_complete=true`, even if retained inventory matches and serialized
  `lost_events` is zero. Counts are exact only while representable. No widened
  count or new JSON schema is introduced. ID counters retain their existing u64
  wrapping behavior, with forced-wrap tests failing inventory completeness.
- Receipt publication remains a quiescent root-drop operation. Concurrent
  publication with active producers is not made snapshot-consistent by this
  change. Tests are real threaded executions, not exhaustive memory-model checks.

## Permanent tests and gates

All 15 tests in the real candidate module pass, including its nine original tests.
The six additional tests are:

1. `exact_cap_is_complete_and_preserves_accepted_order`
2. `capped_events_do_not_lock_and_still_allocate_independent_ids`
3. `racing_cap_retains_prefix_and_counts_each_rejected_event_once`
4. `poison_after_cap_preserves_rejection_and_poison_loss_counts`
5. `identifier_wrap_is_independent_and_fails_inventory`
6. `loss_counter_wrap_cannot_restore_complete_retained_inventory`

The lock-bypass test deliberately holds the event mutex while another thread
makes capped calls, including an interrupted request. Its timeout releases the
lock before asserting, so a regression cannot deadlock the test suite. The
uncapped poison test additionally asserts that admission continues and the
observer has not sealed.

| Gate | Run id | Result |
|---|---|---|
| baseline release build | `20261014-113819165-11391` | pass |
| baseline actual-module tests | `20261014-113828457-13347` | 9 pass |
| candidate check, including tests | `20261014-114350238-75927` | pass |
| candidate actual-module tests | `20261014-114351226-76289` | 15 pass |
| candidate all-target clippy, `-D warnings` | `20261014-114352517-77001` | pass |
| candidate release build | `20261014-114353474-77168` | pass |

Only the owned module was formatted. Final frozen source SHA256:
`38e52413ab64a050b76830c8d274556d39309e0f9dca84d2ddea6627a1302cde`.

### Native compiler acceptance

The coordinator alone rebuilt the actual compiler, std, and proc_macro with
`python3 x.py build --stage 1 -j 2 compiler/rustc library/std library/proc_macro`.
The combined build and acceptance task `773938x4fq` passed with exit 0 in 436.06s.
It incorporated this frozen recorder and separately owned pool dispatch fixes,
so its before/after timing is not isolated recorder attribution.

Seven real native acceptance tests passed:

| Actual compiler gate | Tests | Reported test seconds |
|---|---:|---:|
| `tests/test_execution_graph_query_native.py -v` | 1 | 0.831 |
| `tests/test_execution_graph_wait_native.py -v` | 4 | 26.794 |
| `tests/test_execution_graph_compiler_native.py -v` | 1 | 7.972 |
| `tests/test_execution_graph_compiler_argv_native.py -v` | 1 | 1.614 |

This exercised the rebuilt stage1 compiler on real serial/4-thread/8-thread
queries, waits, saturated read recording, incremental reads, equally warmed
capture off/on/off artifact identity, executed output, error/cycle diagnostics,
Cargo/compiler binding, proc-macro argv privacy, and failed/fail-open sinks.
The worker observed the task completion and rechecked the frozen source hash.
Native driver SHA256 is
`270af337d3643eda83e3f14660bd7e6ea35fc6bd170c088a70aeb9121758f90e`.
The launcher is unchanged from baseline, while the driver hash changed.

The path-redacted native baseline and candidate observations are preserved by
the coordinator in `data/runs/query-read-cap-20261014/native-{baseline,candidate}.json`.
The source-matched candidate's serial/4-thread/8-thread saturated receipts report
30,157,293 / 30,205,928 / 30,342,869 lost events, with 100,000 retained events and
`records_complete=false`. The uncapped small incremental case retains 53,938
events, zero loss, complete record inventory, 13,465 requests, 13,575 reads,
2,426 results, 7,081 tracked-new reads, and 5,870 resolved producer edges. It
executes to `42`. Semantic and full-pipeline completeness remain false, not
silently upgraded by the cap fix.

These are actual integrated compiler acceptance gates, not a substitution by the
standalone harness. They are still not a broad performance gate. This worker
did not bootstrap, commit, push, or install anything.

## Reproduce the focused diagnostic

Run from the repository root. Local mode is intentional for a controlled benchmark,
not a hosted fallback workaround. Preserve each binary before the next build.
Use the same pinned compiler for both and supply its real path to `run.py`.
The build emits the run id needed for each invocation's `--build-run-id`.

```sh
JUSTRUST_REMOTE=off justrust build --release --manifest-path bench/query-read-cap/Cargo.toml --features baseline
cp bench/query-read-cap/target/release/query-read-cap-bench "$JCODE_SCRATCH_DIR/query-read-cap-baseline"
JUSTRUST_REMOTE=off justrust build --release --manifest-path bench/query-read-cap/Cargo.toml
cp bench/query-read-cap/target/release/query-read-cap-bench "$JCODE_SCRATCH_DIR/query-read-cap-after"
python3 bench/query-read-cap/paired.py \
  --before "$JCODE_SCRATCH_DIR/query-read-cap-baseline" \
  --after "$JCODE_SCRATCH_DIR/query-read-cap-after" \
  --output "$JCODE_SCRATCH_DIR/query-read-cap-paired-repro.json" --rounds 3
python3 bench/query-read-cap/verify.py
JUSTRUST_REMOTE=off justrust check --manifest-path bench/query-read-cap/Cargo.toml --tests
JUSTRUST_REMOTE=off justrust test --manifest-path bench/query-read-cap/Cargo.toml
JUSTRUST_REMOTE=off justrust clippy --manifest-path bench/query-read-cap/Cargo.toml --all-targets -- -D warnings
```

`run.py --help` describes initial per-binary sweep reproduction. Reproducing the
one-thread CPU0 and eight-thread CPU0..7 affinity assumptions requires availability
of those CPU IDs or an explicitly recorded change to the paired helper policy.
