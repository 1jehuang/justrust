# Intermediate native compiler benchmarks, October 17, 2026

## Scope and identity

**These are local-machine measurements of a debug-built compiler, not the
96-core hosted release results.** See
`hosted-intermediate-compiler-benchmarks-20261017.md` for the requested hosted
comparison and detailed phase diagnostics.

This is a progress checkpoint, not a Full Rust completion claim or a general
speedup claim. Revision `b4b5028ace7d3640b931dec604a978f7bd482251`, native binary
SHA-256 `50677dcc0cff723228bef7c43af26bcc29a2c09f48d8ffc291e77f73e54a1b91`.
No compiler code, language admission, or resource cap was changed for this run.

The actual `jr-rust` CLI was tested against `/usr/bin/rustc`. Raw receipts retain
source hashes, compile diagnostics, runtime output, and statuses. Timing samples
are fresh processes/fresh output on a warm, uncontrolled machine, three repeats
with alternating tool order. They are not cold-cache, Cargo, dependency, or
session-weighted agent measurements. Stock and native internals/optimization
strategies differ; only these particular source-to-executable jobs are compared.

## 1. Unchanged example programs

All **33/33** top-level `examples/compiler/*.rs` programs passed native CHECK and
BUILD at workers 1 and 8. Every produced program matched stock runtime exit,
stdout, and stderr. This covers arithmetic/control flow, modules, arrays,
structs/enums, generics, traits/associated items, and the supported reference
features. It is a curated repository gallery, not evidence of arbitrary external
crate compatibility. External Cargo dependencies and procedural macros were not
exercised by this gallery.

Receipt: `experiments/results/20261017-intermediate-gallery.jsonl`.

## 2. Differential control-flow checks

Existing `experiments/compiler_correctness.py`, seed 1:

- 32 scalar/recursion/evaluation-order cases matched stock compile and runtime.
- 32 loop/break/continue/temporary-unwind cases matched stock compile and runtime.
- Native workers rotate through 1, 8, and 96 as defined by the existing harness.

Receipts: `20261017-intermediate-scalar.jsonl` and
`20261017-intermediate-loops.jsonl` under `experiments/results/`.
These are generated bounded programs, not substitutes for external projects.

## 3. Source-size ladder and timing

Sources come unchanged from `experiments/compiler.py`'s `source_for` and
`module_source_for`. Count below means generated arithmetic functions, plus
`main` and `fail`. Each function is exercised by a runtime assertion. The module
variant partitions those functions over up to eight inline modules.

Median **process wall milliseconds**, three samples. Failed native compilations
are reported as failures, never as fast successful builds.

| Shape | Functions | Stock BUILD | Native CHECK, 1 worker | Native BUILD, 1 worker | Native BUILD, 8 workers |
|---|---:|---:|---:|---:|---:|
| Flat | 4 | 50.23 | 20.63 | 20.71 | 25.51 |
| Flat | 16 | 52.65 | 74.76 | 74.09 | 102.86 |
| Flat | 64 | 64.30 | 506.24 | 538.97 | 810.61 |
| Flat | 128 | 62.46 | **resource failure** | **resource failure** | **resource failure** |
| Flat | 256 | 99.87 | **resource failure** | **resource failure** | **resource failure** |
| Modules | 4 | 60.39 | 28.66 | 29.15 | 35.76 |
| Modules | 16 | 58.20 | 82.23 | 85.24 | 115.34 |
| Modules | 64 | 60.53 | 555.37 | 579.91 | 791.26 |
| Modules | 128 | 80.72 | **resource failure** | **resource failure** | **resource failure** |
| Modules | 256 | 87.73 | **resource failure** | **resource failure** | **resource failure** |

All 30 stock builds passed and ran correctly. Of 120 native invocations,
72 passed and 48 failed: every native 128/256-function invocation failed at both
worker counts and in both CHECK/BUILD modes. No failed invocation left an
executable. Diagnostics include initialization/move `ResourceLimit`, planning
resource exhaustion, and bounded diagnostic-transport exhaustion. Which stage
reported first varied by scheduling. This is a real practical scaling ceiling,
not an unsupported Rust syntax boundary.

The 64-function samples are substantially slower than stock; eight workers did
not improve these jobs. These measurements prioritize resource/analysis scaling
before broader syntax expansion. They do not isolate a causal bottleneck and do
not justify raising limits. The next useful performance milestone is passing
these identical 128/256-function sources under the existing limits, then
remeasuring the same ladder.

Receipt: `experiments/results/20261017-intermediate-ladder.jsonl`.
The original stock-validated 4/256 benchmark stopped on its first 256-function
failure; its partial measurements and complete failure output are preserved in
`20261017-intermediate-original-flat-partial.jsonl` and
`20261017-intermediate-original-flat-failure.txt`. The follow-up ladder does not
replace or hide that failed run.

## Repeatability and next diagnostic

Both packaged runner entry points were executed again after portability cleanup.
The second gallery run reproduced all 33 successes. The second ladder reproduced
all 150 per-configuration pass/fail outcomes with identical source hashes. Raw
second-run receipts are `20261017-intermediate-packaged-gallery.jsonl` and
`20261017-intermediate-packaged-ladder.jsonl`. Timings in the table remain the
first complete ladder, not a best-of selection across runs.

A separate read-only triage identified repeated whole-CFG provenance validation
inside call analysis as a plausible charged-work scaling hotspot
(`borrow.rs` bind_call/table lookup and `cfg.rs` provenance validation). Dense
ownership state is another scaling candidate. These are static hypotheses, not
measured root causes. The next diagnostic should capture the first rejected
budget operation with resource kind, usage, limit, stack, and function identity
on the unchanged failing source. No limit increase or speculative fix was made.

## Reproduction

```sh
TMPDIR=/path/to/disk/scratch python3 experiments/intermediate_gallery.py \
  --binary target/debug/jr-rust --output /path/to/new-gallery.jsonl
TMPDIR=/path/to/disk/scratch python3 experiments/intermediate_ladder.py \
  --binary target/debug/jr-rust --output /path/to/new-ladder.jsonl
python3 experiments/compiler_correctness.py --binary target/debug/jr-rust \
  --rustc /usr/bin/rustc --cases 32 --seed 1 --output /path/to/new-scalar.jsonl
python3 experiments/compiler_correctness.py --binary target/debug/jr-rust \
  --rustc /usr/bin/rustc --cases 32 --seed 1 --loops --output /path/to/new-loops.jsonl
```

The portable runners preserve all failures and use exclusive output creation.
They invoke real compilers and executable artifacts, not copied compiler logic.
A separate end-to-end external-project milestone remains necessary before
claiming ecosystem readiness.
