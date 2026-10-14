# Native causal mapping acceptance, 2026-10-14

Status: validated native wait/read/join and compiler-binding slice, not a full source-to-test graph.
Source cohort committed and pushed as `d3a5319b9`.

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

## Correctness regression fixed, remaining limits

Cargo/compiler binding acceptance is a separate gate. Its basic native harness
passed, but a real proc-macro probe then confirmed a recording-neutrality bug:
the private `-Zjr-query-binding-fd` argument was visible through `std::env::args`,
changing the generated program from `binding_visible=false` to `true` while
Cargo's metadata cache key was unchanged. Baseline failure task 85389090rs is
retained in compiler-argv-native-results-20261014 session scratch evidence.
The transport was replaced with a collision-checked reserved inherited descriptor
and no private argv or environment variable. Borrowed discovery duplicates only;
consuming descriptor admission is restricted to an explicit unsafe native process
entrypoint, before compiler threads. Safe embedded compiler entry does not probe.
After the final source freeze, the actual compiler/std/proc_macro rebuild passed
in 116.13 seconds. The repaired actual macro regression passed in 1.430 seconds,
requiring genuine compiler binding, identical complete proc-macro-visible argv,
identical generated program output, and identical executable SHA256 off/on.
The genuine native binding harness also passed in 7.678 seconds, including fresh
zero-launch inventories, missing entry/query/journal fail-closed controls, failed
compilation, and private/unwritable sink fail-open behavior. Admission acceptance
passed again in 0.847 seconds. The final wait/read/pool suite passed all four tests
in 31.672 seconds before a reload interrupted the following admission invocation;
only that interrupted remainder was restarted. No mixed-source build is evidence.

Final hosted client checks passed again:

- check: 20261014-111448439-3898857
- 540 tests, 3 ignored: 20261014-111451220-3899815
- strict all-target lint: 20261014-111503226-3897736
- owned pool API: 85 passed, 26 ignored: 20261014-110446989-3751543

Exact exported summaries are in data/runs/causal-map-20261014/final-acceptance.jsonl
and pool-acceptance.jsonl. The pool tests include permanent cap/u64-saturation
regressions for the bounded atomic allocator.

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
