# Local no-op recording phase attribution, 2026-10-30

This is a controlled local diagnostic, not a stock comparison, default-routing
promotion or comprehensive workload speed claim. The command is
`justrust clippy --all-targets -- -D warnings`, with `JUSTRUST_REMOTE=off`.
Native Cargo still resolves and validates all inputs on every invocation. The
local toolchain is `jr-ac20de5e7921`, unlike the hosted nightly toolchain, so
local and hosted observations are not a matched performance comparison.

## Opt-in bounded phase trace

`JUSTRUST_RECORD_PHASE_TRACE=1` prints fixed-name wall-clock phase durations
in milliseconds. It measures local lifecycle startup, toolchain selection,
slot acquisition, preparation, Cargo process wall, pipe draining/scope finish,
sampler shutdown, Git join, report persistence/lifecycle completion, and
cleanup. These are successive intervals, not compiler CPU times. The trace
variable is removed from Cargo's environment and does not change build flags,
cache identity, routing or correctness decisions. The disabled path performs
no trace I/O. The trace has a fixed number of messages per run, with no paths,
credentials or source text in its output.

## First unchanged samples

After warming the new recording source, five unchanged strict Clippy commands
reported 62 fresh units each. These used the existing local daemon path.

| Run | Process wall ms | Toolchain ms | Slot ms | Prepare ms | Cargo ms | Sampler ms | Git join ms | Finish ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `20261029-215710978-3166849` | 894 | 0.161 | 9.458 | 15.898 | 56.600 | 8.753 | 65.936 | 0.563 |
| `20261029-215711848-3166947` | 877 | 0.116 | 11.351 | 17.371 | 41.006 | 6.965 | 88.425 | 0.423 |
| `20261029-215712784-3167104` | 979 | 0.136 | 12.803 | 16.353 | 79.681 | 12.857 | 85.585 | 0.785 |
| `20261029-215714047-3167402` | 1212 | 0.159 | 11.749 | 14.595 | 61.594 | 7.911 | 60.704 | 0.560 |
| `20261029-215715089-3167617` | 1065 | 0.149 | 13.695 | 17.729 | 53.129 | 9.627 | 85.367 | 0.581 |

Pipe draining/scope finish was below 0.4 ms and cleanup below 0.03 ms.
These phases explain only about 0.16–0.21 s, not the whole process wall.
An additional lifecycle-start trace measured about 1.4 ms, ruling it out as
the missing majority in that sample.

## Daemon isolation

With the additional controlled factor `JUSTRUST_BUILDD=0`, native Cargo still
reported 62 fresh units. Process walls were 425, 373 and 409 ms, with Cargo
walls 200, 109 and 160 ms under concurrent compiler load. Git join added 118,
162 and 162 ms. IDs are `20261029-215847302-3184954`,
`20261029-215847730-3185123`, and `20261029-215848104-3185206`.

This isolates substantial overhead outside `record()`. The daemon unconditionally
computes `source_roots` and recursively computes `source_stamp` before starting
the client child. This checkout had about 188,444 files under `toolchain`,
including compiler build outputs, versus 68,384 Git-tracked files overall.
The coordinator owns investigation of that path. Ignored directories must not
simply be excluded, since ignored files can be actual compilation inputs.

A later concurrent baseline attempt is deliberately excluded from comparison:
it joined another agent's identical command for 65.5 s and then 0.5 s, changed
slots, and rebuilt the recording source. This demonstrates why measurements
must be serialized and warmed, not attributed to the phase scheduling change.

## Safe scheduling change

The unchanged observational `git_info` thread now starts at `record()` entry,
instead of after toolchain, slot, machine-information and initial metadata
setup. Its result is still joined and persisted before returning. This overlaps
independent Git work with preflight instead of serializing both ahead of a
short Cargo run. Git metadata never controls Cargo, cache keys or freshness.
No source/result reuse, file exclusion, flag or hosted-default change is made.

Both retained release binaries were then alternated on the same warmed,
frozen checkout, with `JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0` and the trace
switch. A first invocation that rebuilt the changed source was discarded.
Every measured invocation reported zero compiled and 62 fresh units. Shell
wall times include the entire process, not only Cargo.

| Pair | Before run | Before process ms | Before Git join ms | After run | After process ms | After Git join ms |
| --- | --- | ---: | ---: | --- | ---: | ---: |
| 1 | `20261029-220623313-3262252` | 589 | 74.968 | `20261029-220623887-3262440` | 343 | 58.793 |
| 2 | `20261029-220624232-3262536` | 494 | 146.254 | `20261029-220624735-3262666` | 708 | 170.074 |
| 3 | `20261029-220625446-3262906` | 414 | 105.641 | `20261029-220625844-3263013` | 366 | 112.995 |
| 4 | `20261029-220626210-3263174` | 670 | 327.895 | `20261029-220626887-3263505` | 341 | 57.605 |
| 5 | `20261029-220627247-3263604` | 392 | 78.431 | `20261029-220627611-3263767` | 315 | 46.154 |
| 6 | `20261029-220627935-3263881` | 358 | 61.575 | `20261029-220628295-3263976` | 431 | 66.917 |
| 7 | `20261029-220628723-3264106` | 391 | 55.191 | `20261029-220629117-3264289` | 359 | 66.660 |

These diagnostic medians are process 414/359 ms, Git join 78.431/66.660 ms,
and Cargo 195.320/158.717 ms, before/after respectively. Concurrent compiler
work caused substantial variation, and the Cargo difference alone is larger
than the Git-join difference. These samples therefore do **not** establish a
causal or general speedup. `bench/suite.py` was not run, so there is no suite
speedup claim or default promotion. The scheduling change is small and preserves
the same operations, results and failure handling. The dominant daemon-path
cost remains a separate investigation, not a problem solved by this change.

## Correctness validation of the trace

Hosted `justrust check`, full `justrust test`, and
`justrust clippy --all-targets -- -D warnings` all exited zero. The full test
run `20261029-220319083-3228241` reported 838 passed, zero failed, and ten
ignored. Strict all-target Clippy was `20261029-220338353-3226723`.
`justrust show` confirmed the tests ran remotely on hosted. After the Git
scheduling move, the final combined candidate also passed hosted check
`20261029-220813809-3277042`, the full 838-test suite
`20261029-220827074-3278391`, and strict all-target Clippy
`20261029-220836466-3277041`. `cargo fmt --check` and the scoped
`git diff --check` passed. Installation and run-summary export remain owned by
the coordinator.
