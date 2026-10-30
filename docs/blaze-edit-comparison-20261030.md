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

Results pending execution. No speedup or maximum-reuse claim is established by
this protocol alone.
