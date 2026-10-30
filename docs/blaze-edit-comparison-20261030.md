# Blaze versus owned query-retention experiments, 2026-10-30

## Question and scope

Compare warm edits on the same pinned justrust library source (`f97fcc2`) using:

1. Frozen owned compiler, all three experimental flags off.
2. The identical compiler, scalar retention, associated-item retention and empty
   lint-level fast path all on.
3. Pinned stable Blaze `0.20261027.0` (`49ca152`), using its own toolchain.

This is a focused `check --lib` comparison, not the comprehensive session-weighted
workload, not a default-promotion gate, and not an attribution of differences to
one particular query-retention flag. Blaze and rustc have different diagnostic
coverage. A successful check is not an executable correctness or runtime test.

## Installation and compatibility

The official installer was downloaded and inspected before execution. It was
run with `--no-toolchain --verbose` and a new private `CARGO_HOME` under
`/home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo`. Release checksums
were checked by the installer. No normal Cargo configuration was changed and
`blaze enable` was not run. The subsequently requested Blaze toolchain downloaded
into its own normal toolchain cache, not over the installed Rust compiler.

An untimed compatibility smoke of `blaze check --lib` on an archived `f97fcc2`
checkout succeeded and printed `[blaze]`. Its 25.96-second process duration
includes initial loading and is not a warm edit result or a clean cold-cache
benchmark. The measured comparison must use a separate isolated Blaze daemon,
disk store and target rather than reusing this smoke's loaded state.

## Prospective protocol

Preparation and deliberate type-error/recovery validation are separate from
measured edits. Each stack must diagnose the inserted invalid assignment and
successfully check after restoration before it can supply warm measurements.
Each replicate inserts the same unique `black_box` statement into `src/runs.rs`
for every stack. Stack order rotates across replicates. Per-stack incremental
targets are separate; all use the same source path. Original source bytes must
be restored even after failure.

Owned compiler timings disable remote execution, outer artifact caches and
buildd to isolate the native compiler comparison. Self-profiling and incremental
hash verification are not enabled in timed runs. Blaze retains its resident
state and private disk store, which are part of the architecture under test.
Compiler identities, commands, raw diagnostics, process durations and contention
observations must be retained. Failed or incomplete stacks cannot rank as fast.
Noisy samples remain observations, not evidence of a causal optimization.

The first three-stack attempt used the long `matched-check` output path below
the private comparison root. Owned preparations and gates succeeded, but Blaze
preparation exited 101 because its Unix socket path was 121 bytes, beyond the
platform limit. That is a harness setup failure, not an unsupported Rust program
or a fast Blaze sample. Its raw failure and partial measurements are retained
separately. The entire three-stack run is repeated in a new shorter output
directory, without combining samples from the failed attempt.

## Observed results

The complete short-path rerun exited **0**. Each stack rejected the deliberate
type error with exit **101**, then accepted the restored source with exit **0**.
All nine measured edits succeeded. Compiler/driver/helper identities remained
unchanged and all 63 archived source files matched their original hashes after
restoration. Raw evidence is in
`bench/results/blaze-edit-comparison-20261030/complete/`.

| Stack | Warm edit samples, seconds | Median, seconds |
| --- | --- | ---: |
| Owned, experiments off | 1.220236 / 1.015603 / 0.915415 | 1.015603 |
| Owned, all three experiments on | 1.322271 / 1.015780 / 0.969004 | 1.015780 |
| Blaze | 0.114256 / 0.114001 / 0.114129 | 0.114129 |

These are **observations for this one library-check edit**, not a general speed
claim. The owned medians are effectively identical in this run, so it does not
demonstrate a latency benefit from the retention/lint experiments. The observed
owned-off/Blaze median ratio is 8.899, but the different compiler implementations
and diagnostic contracts prevent treating this as interchangeable correctness.
This was not a build/link/test comparison.

Blaze's measured stderr explicitly reports `[blaze]`, rather than counting a
stock fallback as Blaze. The owned sample reports show one local crate rebuilt
because `src/runs.rs` changed, with dependencies fresh. Blaze keeps a resident
workspace database and its private disk store between calls, whereas the owned
compiler is invoked afresh with Cargo incremental artifacts on disk. This is an
architecture comparison, not isolation of a single caching mechanism.

Preparations took 16.287 s off, 16.376 s on and 34.225 s Blaze. They include fresh
target population and loading, with shared dependency downloads and already
cached toolchains. They are neither clean cold-machine measurements nor part of
the warm medians. Error/recovery gates likewise are excluded from medians.

All samples retain **unknown contamination**, not a clean-machine verdict.
Other applications were active and the CPU governor was `powersave` on AC.
The harness uses bounded `Popen.wait(timeout=360)`, whose polling/reaping overhead
is included in the process duration and can materially affect short calls.
For example, the first Blaze command reports 0.07 s internally while the harness
observes 0.114 s. Do not interpret sub-millisecond differences or this small
sample as a precise causal effect. No comprehensive session-weighted comparison,
maximum-reuse conclusion or default promotion follows.

Validation: all **18 focused harness tests** and **538 full bench tests** pass.
Twelve owned run summaries were exported into
`data/runs/xps14-blaze-owned-{off,on}-20261030/`, including the six calls per stack
(prepare, expected error, recovery, three edits). The initial failed setup is
retained under `failed-long-socket-path/`, never merged with the complete run.

Exact complete-run invocation (the output directory must not already exist):

```sh
python3 bench/blaze_edit_compare.py \
  --source /home/jeremy/.jcode/scratch/blaze-comparison-20261030/source \
  --output /home/jeremy/.jcode/scratch/bz30 \
  --blaze /home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze \
  --justrust /home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client \
  --rustc /home/jeremy/.jcode/scratch/provider-profile-20261030/sysroot/bin/rustc \
  --cargo /home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo \
  --command check --samples 3
```
