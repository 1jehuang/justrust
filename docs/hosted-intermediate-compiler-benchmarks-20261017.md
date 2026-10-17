# Hosted intermediate compiler benchmarks and diagnostics

Date: October 17, 2026. Revision `c6ce8e17a` (benchmark-runner corrections only
since local checkpoint `653f35fbf`; no compiler semantic changes).

## Environment and interpretation

The authenticated, charged experiment workflow verified account host type
`m8a.24xlarge`. Guest topology: AMD EPYC 9R45, 96 physical cores, one thread per
core, one socket, one NUMA node. Both native and stock compilers ran on this
same guest. These are shared-host, warm-machine, fresh-process/fresh-output
measurements, not exclusive/isolated or cold-cache timings.

Stock: rustc 1.99.0, commit b940084d7, LLVM 23.1.1. Native release binary SHA256:
`5c9067abe355d7490c0c9c4cc4fa4f10d24127567d4bada21168f7c3542710f2`.
Native debug was separately rebuilt and measured as a control. Earlier local
numbers used `target/debug/jr-rust`, not an optimized compiler. They remain
valid local-debug observations but should not be presented as hosted-release
performance. Hardware, compiler build profile, and stock version differ, so
cross-environment differences are not a controlled hardware speedup result.

The release ladder uses workers 1, 8, and 96, three repetitions each, alternating
configuration order. Stock builds use edition 2024, opt-level 0 and overflow
checks. Successful native and stock executables must print the same expected
literal and exit zero. Failed compiler invocations are retained, not timed as
successful compilation.

## Hosted release results

Median process wall milliseconds. Counts refer to generated arithmetic
functions plus main/fail. Same source generators as the local run.

| Shape | Functions | Stock BUILD | Native BUILD 1 worker | Native BUILD 8 workers | Native BUILD 96 workers |
|---|---:|---:|---:|---:|---:|
| Flat | 4 | 37.54 | 4.36 | 10.34 | 13.59 |
| Flat | 16 | 38.54 | 13.13 | 28.53 | 34.05 |
| Flat | 64 | 45.19 | 98.08 | 153.25 | 176.07 |
| Flat | 128 | 52.44 | resource failure | resource failure | resource failure |
| Flat | 256 | 66.87 | resource failure | resource failure | resource failure |
| Modules | 4 | 38.62 | 4.40 | 10.62 | 13.44 |
| Modules | 16 | 37.37 | 13.12 | 28.79 | 33.88 |
| Modules | 64 | 45.78 | 98.30 | 153.28 | 177.46 |
| Modules | 128 | 52.06 | resource failure | resource failure | resource failure |
| Modules | 256 | 66.29 | resource failure | resource failure | resource failure |

The optimized native compiler is quicker on the two tiny workloads, but about
2.17 times slower than stock on the flat 64-function build. Additional workers
hurt these measured jobs. No general compiler speedup or regression attribution
is claimed from these limited, uncontrolled samples.

All 30 stock builds succeeded. Of 180 native CHECK/BUILD runs, 108 succeeded
and 72 failed: every 128/256-function run at every worker count exhausted
resource limits and produced no executable. This confirms the size ceiling is
not merely local hardware or a debug-build artifact.

Hosted debug controls reproduce the same pass/fail boundary. Flat BUILD medians
at one worker: 4 functions 16.56 ms, 16 functions 52.89 ms, 64 functions 405.41 ms.
The debug versus release difference is substantial, but it does not remove the
underlying scaling defect.

## Correctness

All 33 unchanged repository example programs passed native CHECK and BUILD at
workers 1, 8, and 96 and matched stock runtime status/stdout/stderr. All 32 seeded
scalar and 32 loop/control-transfer differential cases matched stock. These
remain bounded programs and a curated gallery, not external Cargo ecosystem
coverage or completion of Full Rust.

## Detailed built-in diagnostics

Separate `--check --trace` invocations captured 4/64/128-function sources at
workers 1 and 96. They are not included in the performance table: verbose
formatting and writing of the 64-function CFG/proof graph is approximately
150 MB and takes about 8.6 seconds after compilation. Its engine time remains
roughly 94 ms at one worker. Trace serialization is a separate observability
cost, not evidence that compilation took 8.6 seconds.

For 64 functions, one worker:

- 310 graph nodes, 625 edges, 464 polls, zero dropped events.
- Engine elapsed 94.32 ms; summed recorded execution intervals 93.84 ms.
- Ownership/initialization analysis phases total about 75.61 ms.
- The main instance (index 65) alone accounts for 68.23 ms of those intervals.
- Source-template checking about 7.23 ms, lowering 6.84 ms, parsing 0.37 ms.
- 578 CFG blocks, 1,160 locals and move paths, 578 analysis worklist pops.

The 128-function trace reaches the main analysis (index 129), which runs about
48.27 ms before failing. At one worker the graph reports two failed nodes and
zero dropped events. The public failure is resource exhaustion, including
bounded diagnostic exhaustion; it is not unsupported source syntax.

At 96 workers, summed execution intervals include concurrent activity and
blocking inside an active task. For 64 functions they total about 5.12 seconds
while graph wall time is 162.95 ms. They are **not CPU seconds**, cannot be added
to wall time, and alone do not identify a particular lock as the culprit.

The report explicitly sets `resource_sampling_available: false`: allocator
samples and peak RSS are not implemented in this engine trace. The exact first
exhausted resource and allocation/call stack are therefore still unobserved.
Next diagnostic: instrument the first rejected shared-budget charge, recording
resource kind, usage, limit, function and stack. Repeated whole-CFG validation
inside analysis is a plausible static suspect, not a confirmed root cause.
Do not raise limits to mask it.

## Evidence and lifecycle

All files are under `experiments/results/hosted-intermediate-20261017/`, with a
SHA256 manifest. `raw-profiles.tar.gz` retains all six full traces, original
profile sources, trace hashes and the debug ladder. `phase-summary.json` is a
derived aggregation of running-to-suspended/task-done intervals by graph key.
It is checked against the report totals and does not treat queue/wait sums as
wall time. `release-ladder.jsonl` retains all 210 measured samples.

- `compiler-intermediate-hosted-20261017-0150`: charged run ended with a benchmark
  script argument-shadowing error after successfully building the compiler.
  The script was corrected in c6ce8e17a; no compiler workaround was made.
- `compiler-intermediate-hosted-20261017-0151`: completed release ladder, gallery,
  scalar corpus and part of loops before the local controlling daemon reloaded.
  Remote process absence was verified, then the normal authenticated API
  recorded an abandoned terminal event. Partial loop evidence is preserved.
- `compiler-intermediate-hosted-20261017-0152`: charged debug control and all six
  traces completed, exit zero, lifecycle recorded successfully.
- `compiler-intermediate-hosted-20261017-0154`: charged rerun completed all 32 loop
  cases, exit zero, lifecycle recorded successfully.

No run bypassed subscription admission, accounting or tenant boundaries. The
host's stable compiler installation and resource caps were not changed.
