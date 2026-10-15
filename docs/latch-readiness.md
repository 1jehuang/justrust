# Observed latch-readiness provenance

Status, October 15, 2026: native and consumer implementation is shipped. Actual-source acceptance is in progress. This is not a complete execution DAG or a performance claim.

## Explicit opt-in

```sh
justrust graph CAPTURE --latch-readiness-dag --large-capture --json
```

The new flag admits pool schema 6 with `execution_flow.schema = 2`. The pool report exposes `latch_readiness_dag`, with scope `observed-query-join-and-latch-control-flow`. It extends the observed query/join flow with native successful latch handoffs. Timing schema 2 and query roots 5/6 are unchanged.

Feature admission and output remain independent:

- Pool 4 requires `--query-cpu-ownership`.
- Pool 5 requires `--execution-flow-dag`.
- Pool 6 requires `--latch-readiness-dag`.
- Combining flags admits their respective formats, without projecting a pool 6 capture into the older v1 report.
- Query CPU, dispatch and cache output still require their own flags.
- Public reciprocal clock authority remains same-epoch, offset zero. Nonzero offsets are refused.

The native constructor is `QueryAdmissionObserver::new_with_event_capacity_and_latch_readiness(path, capacity, dispatch)`. The fixture's seventh control line is `latch_readiness` or `off`; existing two through six-line controls retain their meaning.

## What a transfer establishes

A readiness row identifies the actual producer admission and a fresh readiness ID. Before acquiring the latch lock, native instrumentation prepares a prerequisite gate and an unknown visibility cut. A detached, row-bound `readiness_prepare` edge connects the actual saved prefix to that gate. It leaves the ordinary scope cursor and pending synchronous return unchanged. The prerequisite is a precursor, not the exact publication or notification time.

The existing latch mutex carries only already-prepared qualified scalar tokens. There is no new v2 recording, allocation or clock sampling under that lock. Existing legacy register/notification instrumentation is unchanged.

Each actual encounter has a fresh wait-attempt ID and the actual saved caller invocation. A successful transfer requires the exact same-latch published token, original producer authority, a successful cache lookup, and two row-bound obligations into its consumed gate:

1. The saved caller boundary through an unknown wake-to-consume cut.
2. The matching producer's unknown visibility cut.

Registered encounters additionally require the original qualified wait lifecycle. Already-ready encounters are explicitly recorded rather than invented wait registrations. Root formats do not expose wait-ID exhaustion authority, so registered encounters without an original ID cannot authorize a successful transfer.

Waiting and helper execution remain independent observed streams. Their tails are not substituted for caller or producer prerequisites. Direct query CPU ownership is preserved while the control-flow wait stream is separated.

## Partial records and conservative races

A consumer can observe a latch after its lock is released but before the producer's diagnostic row is marked published. Such an encounter is honestly `consumed_unbound`, not inferred successful provenance. Poison, cycle, unwind and unbound encounters never authorize success edges.

Independent bounded inventories cover readiness rows and wait attempts, with checked attempted IDs and explicit loss. Missing in-range records under proven loss are unknown. Foreign, duplicate and invalid identities cannot provide authority. Missing query authority withholds query costs, public query labels and readiness transfers, but does not erase independently validated task/opaque CPU measurements.

The report adds retained row/loss/active counts, `ready_records_complete`, `wait_records_complete`, `matched_readiness_transfers`, `unbound_wait_attempts`, and `unavailable_readiness_references`. Completeness is about retained qualified records, not coverage of all compiler prerequisites. Counts do not imply every encounter is matched.

All cuts remain unknown barriers. Full CPU work, complete execution span and parallelism remain null. Neither notification, admission completion, whole query lifetime, worker chronology nor helper completion is treated as a cache-publication or semantic-read edge.

## Validation evidence so far

- Native source-stable suite: 258 primary tests passed, 28 ignored, plus 28 doctests. Coherent stage1 compiler/sysroot build passed in 484.75 seconds.
- Final explicit hosted consumer check, 709 passing tests with 3 ignored, and all-target warning-free clippy passed. The three run summaries are committed under `data/runs/latch-readiness-20261015/`.
- Three genuine native partial/foreign packets passed twelve installed public CLI admission and authority checks without changing input bytes or the client. One task-bearing foreign packet retained exactly 1,762 ns of independent task CPU and a 1,084 ns native-edge chain, with no query labels or qualified readiness transfers.
- The [retained native failure/race packet replay](../bench/results/20261015-readiness-retained-public.json) passed sixteen public cases on four original packets. It covers registered and already-ready unbound races, cycle/poison/unwind, active-mode unwind and registered encounters without an original wait ID. These guard-test packets deliberately lack query-root authority, so they establish typed partial handling, not qualified scheduling transfers.
- The combined offline analyzer, original-byte oracle and descriptor-control suite passed all 137 tests.
- Actual cold/warm/mixed source captures and independent public acceptance are still pending. Do not describe this milestone as fully accepted until those receipts are committed.

The accepted predecessor is documented in [execution-flow-20261015.md](execution-flow-20261015.md). Paired performance promotion remains blocked by the separately documented synthetic baseline defects and noise, not replaced by these correctness checks.
