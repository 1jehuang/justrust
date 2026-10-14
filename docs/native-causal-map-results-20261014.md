# Native causal mapping acceptance, 2026-10-14

Status: validated native wait/read/join slice, not a full source-to-test graph.

The actual owned stage1 compiler and standard library rebuilt successfully with
`python3 x.py build --stage 1 -j 2 compiler/rustc library/std`.
The successful resumed bootstrap took 423.93 seconds. Earlier attempts exposed
and fixed a deprecated atomic allocator API and an unreachable public helper.
A server reload interrupted one attempt, whose compiled artifacts were reused.

The rebuilt compiler passed all four tests in
`tests/test_execution_graph_wait_native.py` and the existing admission test.
Retained evidence lives in the session scratch wait-native-integrated and
admission-native-integrated directories. The wait suite took 28.389 seconds.

Observed actual lifecycle counts on the contention fixture:

| Frontend threads | Registered/notified/condvar-returned/resumed waits |
| --- | --- |
| 1 | 0 |
| 4 | 331 each |
| 8 | 1253 each |

The alias-cycle fixture observed one cycle notification and one poisoned
notification, with unchanged capture-on/off diagnostics. Failed interfaces
remain explicitly incomplete. Equally warmed executable hashes and executed
outputs were identical with recording off/on/off for 1, 4, and 8 threads.
Parallel native pool receipts were required and strictly assessed independently.

The small incremental read fixture had zero lost events, 7081 tracked-new reads,
5870 resolved publishing-job to consumer-job edges, and executed output `42`.
Publishing-job provenance does not prove fresh provider execution.
The large contention fixture exceeded the bounded semantic-read recorder.
Its read assessment remains incomplete even though its independent wait
inventory is complete. No whole-graph completeness is inferred from that test.

The integrated client passed hosted check, all 540 tests (3 ignored), and
`justrust clippy --all-targets -- -D warnings`, then was installed locally.
Exported summaries are retained in data/runs/causal-map-20261014:

- check: 20261014-104125366-3435193
- full test: 20261014-104130902-3437189
- strict lint: 20261014-104152011-3434666

The installed client also decoded real standalone schema3 producer bytes with
one complete wait and one resolved read edge. This is a protocol check, not a
replacement for the native compiler tests above.

## Remaining limits and publication hold

Cargo/compiler binding acceptance is a separate gate. Publication is held while
its transport is checked for proc-macro observability and recording neutrality.
An untracked compiler argument must not change macro output without a cache-key
change. The binding cohort cannot be published merely because client tests pass.

Scope/scheduler/resource-grant coverage, pre-attachment work, query-to-task
binding, nonquery codegen/link/subprocess causality, and previous-session/feed
provenance are still incomplete. CPU work and execution span remain null.
No theoretical parallelism ceiling, architecture speedup, or default promotion
is claimed. Scoped overhead samples are diagnostics, not the comprehensive
session-weighted benchmark gate.

## Scoped recorder overhead diagnostic

Three sequential samples per configuration used the identical frozen fixture
(source SHA256 b3c73cbfc55f6150bb8e2386e97dde708a7bae6b3742e162047a91246842000a).
Median compile wall seconds:

| Compiler / recording | 1 frontend thread | 4 frontend threads |
| --- | ---: | ---: |
| Frozen admission baseline / off | 0.2777 | 0.1995 |
| Frozen admission baseline / on | 0.2388 | 0.1527 |
| Native causal candidate / off | 0.1617 | 0.1233 |
| Native causal candidate / on | 0.2862 | 0.3007 |

The baseline on/off inversion demonstrates noise and cache/order effects.
These samples have unknown quiet provenance and are explicitly ineligible for
speed claims. The candidate capture cost is material and motivates reducing
recording overhead before any default promotion. Raw samples, compiler hashes,
executed output, and fixture text are in bench/results/native-causal-map-20261014.
They measure this recorder fixture only, not general build performance.
