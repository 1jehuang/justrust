# Observed cache-call provenance, 2026-10-15

## Why this is separate from semantic dependency reads

Cold nonincremental Rust builds can have complete diagnostic records while rustc's incremental dependency reads report `disabled`. That is not evidence of an absence of dependencies. The CPU timing milestone also found substantial cold observed CPU without a known task owner. Neither a complete event inventory nor a CPU sample proves a complete semantic dependency graph.

Existing native query-read schema 1 already records one mutex-ordered stream containing actual request starts, cache hits, result publications and request terminals. A cache hit carries the actual request ID and result index. A publication associates that index with a publishing job before the value becomes cache-visible. This supports a narrower useful mapping without another compiler change: **which previously published result an observed successful cache call obtained**.

This relation is not an incremental semantic dep-read, proof of fresh provider execution, or a whole-job execution edge. Default graph output and existing dep-read coverage metrics must remain unchanged.

## Opt-in contract

The explicit public analysis is `justrust graph --cache-call-provenance`, combinable with `--large-capture`. It applies to native query schemas 4/5 with independent admission inventories. Manifest reconciliation remains a different authority path. Default strict and explicit large limits stay unchanged.

The optional `cache_call_provenance` assessment has scope `observed-cache-result-availability-not-execution-dag`. A relation names the actual request, context, consumer job, result, publishing job and dependency-node index, plus zero-based original stream positions for request start, publication, cache hit and normal request terminal. Positions are not timestamps or CPU durations. The recorder's total stream order is used to establish publication precedence only. It must not become an edge between every neighboring event: recorder serialization is not a build dependency or a serial critical path.

Certification requires one unambiguous cache hit on a normal `Get`/`Value` or `Ensure`/`Ensured` request, exact matching request identities and endpoints, one unique publication in the same context/index namespace strictly before the hit, and admitted producer/consumer endpoints. A producer's admission completion need not precede the hit: publication precedes visibility, while the diagnostic completed marker can follow wakeup and racing consumers.

Feed calls, interrupted calls, green-ensure/cache-hit combinations, repeated hits, missing request lifecycle, absent consumer ownership, unavailable admissions, absent or later publication, and incomplete capture authority are explicit unresolved cases. Later publication never repairs an earlier hit. A missing root/nonquery consumer is not replaced with a guessed parent. Lost records cannot certify uniqueness by assuming no omitted competing fact existed.

Repeated requests for one cached result are distinct relation rows. A producer and consumer may be the same job, but the relation still connects distinct publication and hit events. It must not be rendered as an executable job self-loop. Similarly, combining admission parent-to-child edges with reverse result-to-parent job edges can create artificial cycles for ordinary synchronous work. No such union is authorized.

At most one relation is emitted per observed request, bounded by the existing validated event budget. The assessment's independent request/reason partition distinguishes unresolved requests from missing observed records. Read-record completeness is not call/semantic completeness.

## Storage and replay

The analyzer requires explicit `--cache-call-provenance` plus `--graph-output PATH`. The full public graph output, including relation rows, is retained as a new private local artifact with no overwrite. A compact committed report can retain counts, reasons, native receipt hashes, and the exact full graph artifact's size/hash without committing tens of MiB of relation rows. Normal analyzer operation must not add fields or create that artifact.

## Accepted validation and retained results

Consumer commit `4deee0558`, analyzer commit `92f2bd143`, and acceptance harness `515f9b479` are on main. The installed consumer SHA-256 is `ef643f146e01cb3c7abd96a02f65bb3c0ac6d15e5090e687adcaaa8692086c52`.

Hosted check `20261014-184331332-213278`, full test `20261014-184334962-213547` (630 passed, three ignored, zero failed), and warning-free clippy `20261014-184359637-215503` passed. Their slim run summaries are in `data/runs/cache-call/2026-10.jsonl`. The analyzer's 40 Python tests passed. The retained-input harness passed all nine tests, with no skips, in 18.996 seconds. This includes eight adversarial oracle tests and real cold/warm public graph validation, not a new compiler build or speed benchmark.

| Actual retained capture | Requests | Cache hits | Certified relation rows | Unresolved requests | Distinct qualified publishing-job relations |
|---|---:|---:|---:|---:|---:|
| Cold | 231,751 | 213,125 | 154,017 | 77,734 | 45,663 |
| Warm | 8,038 | 5,713 | 3,580 | 4,458 | 2,460 |

Cold unresolved reasons are 322 feed requests, 18,306 requests without cache hits, 21,333 without context/consumer ownership, and 37,773 without prior publication. Warm reasons are respectively 307, 2,020, 27 and 2,104. Other reason counters are zero on these captures. Both have zero lost records and zero orphan cache hits. Those observations do not imply semantic completeness.

Every emitted row was independently checked against original single-stream positions, exact request/result identities, a unique prior publication, a successful terminal, and admitted endpoints. Default graph JSON exactly matches frozen assessments. Removing only the opt-in field exactly recovers default graph JSON. Default analyzer reports match the previously committed CPU reports byte for byte. All original capture and frozen-assessment hashes remained unchanged. The acceptance inventory before/after SHA-256 is `3af21a489e859f5ffa23d63f476564c02b9f2ad4763dea6f30d4f6f7d0c890d2`.

Compact reports are committed at `bench/results/cache-call-provenance-20261015/{cold,warm}.json`. Exact analyzer public graph stdout is retained outside the source captures in `/home/jeremy/.jcode/scratch/cache-call-provenance-analyzer-20261015-1BRD2k/`, with private directory/file modes 0700/0600:

- `cold-graph.json`: 60,796,922 bytes, SHA-256 `1078cccd14e26b238886a9231b9ae18b16ab8fd4be848bf4540ebde54bbdfa55`.
- `warm-graph.json`: 1,383,926 bytes, SHA-256 `98e4551a322273462be6546996824fe87402c7411260b14af9e8521137dfc09c`.

Separate acceptance command/stdout/stderr wrappers remain in `/home/jeremy/.jcode/scratch/cache-call-real-final-20261015/cache-call-acceptance-meth_ffw/`. Their stdout bytes can differ from analyzer stdout because analyzer snapshots rename input files for strict routing authority. Each artifact has its own hash, and relation facts independently match the same original streams. These large local artifacts are not committed or a durable hosted artifact store.

To reproduce with fresh output paths outside the capture directory:

```bash
python3 bench/frontend_mapping.py \
  --capture "$JCODE_SCRATCH_DIR/frontend-timing-real-final-20261015/frontend-real-itoa-56qsp7oz/timed-cold-t4-1-capture" \
  --justrust /home/jeremy/.cargo/bin/justrust \
  --large-capture --cache-call-provenance \
  --graph-output /private/new-output/cold-graph.json \
  --output /private/new-output/cold-report.json
# Use an existing private output directory and fresh filenames.
# For warm, select timed-warm-t4-1-capture and omit --large-capture.
```

Full graph artifacts are created with exclusive creation and no overwrite. Output paths resolving inside the input capture are rejected before scanning or invoking the native consumer. Invalid native authority still fails closed; preserving stdout as evidence does not certify a compact report.

Even a large resolved request count is not a work-weighted coverage percentage. Cache misses, race hits, waits, feeds, previous-session state and nonquery work still need explicit provenance. A few unresolved provider executions may dominate CPU. These are relations in the observed compiler implementation, not a proof that Rust language semantics require the same query boundaries or call structure. An architecture change can remove, merge or restructure them. This slice cannot establish a critical path, W/S, a 96-thread ceiling or an architecture-independent parallelism bound.

A later native follow-up, `dispatch-result-provenance-results-20261015.md`, records actual nonincremental dispatch return indices in a separate opt-in assessment. It improves result-availability mapping without classifying provider/race/wait routes, changing these retained cache-call counts, or completing the execution DAG.
