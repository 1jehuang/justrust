# Native compiler profiling

This adds observability, not an optimization or broader Rust support. The
compiler's existing resource limits and admission checks are unchanged.

## Lightweight public profile

```sh
justrust build --release --bin jr-rust
target/release/jr-rust program.rs --workers 1 --check --profile new-profile.json
```

`--profile` creates a new JSON file on semantic success or failure. Existing
files are never overwritten. Unlike `--trace`, it does not serialize CFGs,
whole ownership states, or proof payloads. It contains:

- Compiler-call wall time, graph wall time, queue/wait totals and summed poll time.
- Summed task-wall intervals by typed graph operation and the slowest 128 nodes.
  Intervals include transition bookkeeping and are **not CPU time**. Their sum
  can exceed elapsed time with multiple workers. Capture losses and unmatched
  intervals are explicit.
- Planning work/nodes/applications/obligations/diagnostic-byte usage and logical
  reservation counts, current/peak/cumulative charged bytes.
- The **first** instrumented rejected budget operation, including the kind,
  increment, previous usage, limit, arithmetic-overflow indicator and source
  call site. Later diagnostic rendering failures do not overwrite it.
- The rejecting graph/task identity, when failure occurred during a provider
  poll, and its typed operation/key where captured events permit that join.

Budget bytes are conservative admission estimates, **not actual heap bytes**.
Raw allocator failures, process OOM, and semantic guards outside the instrumented
budget APIs are not automatically explained by the first-failure field. A null
field does not establish that compilation succeeded. Concurrent first failure
means the first recorded rejection under the existing ledger lock, not a claim
of a globally deterministic cross-thread causal ordering.

The profile's compiler-call timer and optional allocator snapshot are taken
before profile serialization. Normal CLI total time can still include profile
writing. Profiling output has costs and is not a free timing baseline.

## Opt-in actual allocator counts

```sh
justrust build --release --features allocation-profile --bin jr-rust
target/release/jr-rust program.rs --check --profile new-allocation-profile.json
```

The feature installs a wrapper around `std::alloc::System` **in this CLI only**.
Default builds and library consumers do not receive an instrumented allocator.
It counts successful allocation/deallocation/reallocation calls, cumulative
requested allocated/freed bytes, and current/peak requested live bytes from
process start through compiler return. Failed reallocations leave accounting
unchanged. Reallocations account for the full old and new requested sizes.

These are Rust global-allocator requested layout sizes, not RSS, allocator
metadata, allocation stacks, or semantic-category attribution. Native code that
bypasses Rust's global allocator is not counted. Peak order follows accounting
hooks, and snapshots across atomic fields are not transactional while other
threads allocate. Compiler workers have joined before CLI sampling. Keep this
feature off timing baselines and measure its overhead separately.

## CPU samples and process resource use

`experiments/compiler_profile.py` runs the actual existing CLI, without building
it or invoking the remote service itself. Run it **inside** the normal metered
hosted experiment. It preserves the child's exit status and records child wall,
user/system CPU, maximum RSS, page faults and context switches using `wait4`.

```sh
python3 experiments/compiler_profile.py --output-dir /scratch/new-cpu-profile \
  --mode record --input program.rs --execution-context hosted \
  -- /absolute/path/jr-rust program.rs --workers 1 --check
```

`--mode stat` requests perf counters; `--mode record` requests sampled CPU stacks.
Permission failures, unavailable counters, truncation and symbolization problems
remain explicit in the receipt. A preflight failure falls back to one
resource-only invocation unless `--require-profiler` is supplied. Once a compiler
starts, the runner never retries it. Raw sample artifacts can contain private
paths or stack memory and need review before publication.

The known 96-core host has `perf_event_paranoid=4`, which denied an unprivileged
probe on October 17, 2026. No global sysctl or permission setting was changed.
An explicitly authorized `--perf-sudo` mode scopes elevation to installed system
perf, uses `runuser` to preserve the invoking compiler user's UID/GID/groups, and
rejects a mismatching worker identity before compiler launch. It does not run
the compiler as root, install tools, modify sudoers, or grant a global capability.
This mode changes the inherited environment and is labeled accordingly.
Actual privileged sampling acceptance and overhead measurements remain pending.

## Validation and scope

The hosted all-feature suite passes 1,498 tests, including reservation release,
first-failure stability, overflow, poison, concurrent budget use, allocator
accounting, and public CLI executable-byte parity with/without `--profile`.
Strict all-target/all-feature lint passes. New JSON metadata is additive. CPU
runner mock checks are supplemental, not proof of usable hosted CPU stacks.

The October 17 checkpoint additionally exercises the public CLI's expanding
`grow::<(T,)>` recursion at workers 1 and 4: compilation fails at planning depth
129 against limit 128, and JSON joins the rejection to the `mono_plan` task.
A parser nesting rejection was also observed with a null budget failure, as
expected for a guard outside the instrumented ledger.

The synthetic `main_only` control uses a checked arithmetic chain and literal
`println!`, not unsupported formatted output. Its 4- and 128-count controls,
plus 4-count called/independent controls, were compiled and executed at workers
1 and 4. These eight local CLI smoke checks are functional checks, not a timing
comparison or the deferred benchmark matrix. An allocation-feature debug CLI
reported positive allocation counts and consistent settled byte totals. The
stable client routed that debug artifact build locally by policy; the full
check/test/lint gates ran on hosted, including the default-feature CLI test.
The resource-only runner was also exercised against that actual local CLI and
recorded a successful child with UID/GID preserved and no truncation. This does
not validate perf counters, CPU stacks, privileged sampling, or overhead.

Further benchmarking was deferred while language implementation progresses.
Neither this instrumentation nor the small scaling workload establishes full
Rust compatibility, representative project performance, or architectural benefit.
