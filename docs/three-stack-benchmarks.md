# Direct justrust, Cargo, and Blaze measurements

## Result: direct controlled-local comparison without speculative precompilation

On **November 1, 2026**, the corrected direct comparison completed **126 of 126 correctness-gated calls**: three stacks, seven phases, and six repetitions. Each repetition used a different stack order, covering all six permutations. Independent offline replay of the full raw evidence passed. These are actual executions, not estimates from feature-toggle benchmarks. The initial, confounded run is preserved separately below.

**Blaze was faster on this warm edit/error/recovery loop, but slower from cold build-artifact state.** With speculative precompilation disabled, the shipping justrust client had roughly Cargo-scale compilation timings and higher unchanged-command overhead. This does not establish a general justrust speed advantage, nor measure its default hosted or speculative-prebuild experience.

### End-to-end wall time, median [minimum–maximum], seconds

| Phase | Stock Cargo | Shipping justrust client | Source-built Blaze |
|---|---:|---:|---:|
| Cold filtered test | 11.349 [10.817–12.070] | 11.612 [11.422–12.268] | 25.766 [25.152–26.667] |
| Warm unchanged test | 0.03918 [0.03385–0.04410] | 0.05244 [0.05020–0.05943] | 0.03128 [0.02972–0.03541] |
| Reachable test-body edit | 1.079 [0.946–1.196] | 1.086 [0.964–1.224] | 0.258 [0.227–0.278] |
| Type-error diagnostic | 0.615 [0.461–0.712] | 0.611 [0.491–0.709] | 0.115 [0.108–0.120] |
| Type-error recovery, fresh edit | 1.029 [0.855–1.250] | 1.050 [0.930–1.163] | 0.111 [0.100–0.116] |
| Assertion failure, fresh edit | 1.070 [0.931–1.196] | 1.055 [0.982–1.206] | 0.112 [0.105–0.128] |
| Assertion recovery, fresh edit | 1.056 [0.980–1.113] | 1.116 [1.073–1.169] | 0.112 [0.093–0.125] |

There are **six retained samples in every cell**. No first touch or outlier was discarded. Full-precision samples and raw-output provenance belong with the evidence archive, not just this rounded table.

For the reachable test-body edit, the ratio of Cargo's median to Blaze's was **4.18×**. Conversely, Blaze's cold median was **2.27× Cargo's**. Cargo and justrust edit medians were 1.079 and 1.086 seconds, respectively. These are descriptive observations on this pinned workload, not significance tests or universal speedup claims. Machine noise is **unknown**.

## Exactly what was compared

| Stack | Actual executable | Identity |
|---|---|---|
| Cargo | Stock Cargo with explicit stock rustc | Cargo 1.101.0-nightly, pinned toolchain below |
| justrust client | Installed shipping client, copied byte-for-byte to freeze its identity | `justrust 0.1.0-alpha.19`, SHA-256 `d967f1aa30552682c807f3002bdc416017ba5890a38478b2cc9cf03c577c0c37` |
| Blaze | Source-built upstream `ra-jit cargo test` | `blaze 0.20261027.0`, upstream revision `5d4f78f7577cf7170993b74f735099dd03f5c6ca`, ELF SHA-256 `fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263` |

The Blaze source checkout is from `indexable-inc/blaze`, with the recorded build exiting successfully. This is a pinned source-built reference, not a claim to test whichever upstream release is newest when this document is read. Its MIT/Apache-2.0 lineage is distinct from the shipping justrust client.

The experimental justrust native engine is **not** the shipping justrust client and was **not a fourth stack in this run**. Earlier 73.83% and 97.78% same-engine feature-OFF/ON retouch measurements are not direct justrust-versus-Blaze or justrust-versus-Cargo results.

### Machine and workload

- Linux `7.2.11-arch1-1`, x86-64, Intel Core Ultra X9 388H, 16 logical CPUs, about 66.9 GB RAM, no swap.
- Workload: justrust snapshot `f97fcc2`, exactly 63 pinned source files, unchanged lockfile.
- Toolchain: `nightly-2026-10-27-x86_64-unknown-linux-gnu`, rustc `1.101.0-nightly` (`16d1ffac1`, October 26, 2026), with a complete 4,020-file identity manifest.
- Toolchain manifest SHA-256: `34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.
- Command subset: `test --lib -- summary::tests:: --test-threads=1 --nocapture`. Cargo and justrust additionally receive `--locked --offline`.
- The exact four tests are `shares_overlapping_time`, `splits_passes_without_double_counting`, `parses_libtest_results`, and `test_failures_are_not_compile_errors`, all under `summary::tests`.
- Successful calls require exactly four passes and 144 filtered tests. Assertion-failure calls require the exact expected failing test and three passes. Type-error calls require status 101 and the fresh `E0308` diagnostic, not test execution.

## Controlled-local contract

Cargo and the shipping client use the **same actual Cargo/rustc binaries**, with dev/test optimization level 0, debug information level 0, and incremental compilation enabled. Blaze uses its own native compiler and Cranelift path. Backend equivalence is therefore **not claimed**. Stack-specific orchestration/cache/linker behavior is part of the observation, not an isolated attribution to any one component.

The shipping client is explicitly configured with `JUSTRUST_TOOLCHAIN=system` and `JUSTRUST_REMOTE=off`. Prebuilt downloads, remote/peer caches, usage telemetry, and the speculative build daemon/prebuild are disabled (`JUSTRUST_BUILDD=0`, `JUSTRUST_BUILDD_PREBUILD=0`). Its other local caching behavior remains enabled. Expensive tool/seed identity checks occur before source mutation, not in an untimed preparation gap after the edit. All 42 justrust calls retain actual metadata verifying the selected Cargo/rustc, `place=local`, and no owned-toolchain substitution.

Blaze's proc-macro-server controls are explicitly `BLAZE_PROC_MACRO_SERVER=0` and `RA_JIT_PROC_MACRO_SERVER=0`. Rustc-built proc macros remain part of this workload. Native/Wasm proc-macro-server behavior is not covered. Both native remote controls are off. Cargo's offline control is set for every stack, but this is **not an OS network sandbox**.

This is **not a measurement of the shipping client's default hosted service**, default owned toolchain, speculative-prebuild benefit, network artifact downloads, or toolchain installation. The corrected run has exactly 42 shipping-client recordings, seven per private home, with no extra prebuild recordings or build-daemon directories.

### Cold and warm definitions

Dependencies were fetched into a fresh public crates.io source/cache/index seed **before timing**. Each stack and repetition received an independent writable copy. No compiled dependency artifacts, target directories, resident daemon state, or generated sysroot caches were seeded. The preinstalled toolchain itself is a prerequisite, not a timed download.

A cold sample means **cold build/daemon artifacts with dependency sources available**, not cold OS page caches or a network download benchmark. Each cold preparation is followed by warm unchanged execution, a reachable test-body edit, a type error and fresh recovery, then an assertion failure and fresh recovery. These state transitions are intentional. The later phases are not independent cold builds.

Fresh reachable nonces prove current code execution in the edited/recovery calls: **54 distinct execution nonces** passed their exact raw-output checks. The injected assertions do not replace the original test assertions. This experiment edits a test body, not an arbitrary production function.

## Reproduce and audit

The harness is `bench/three_stack.py`. It requires explicit absolute executable paths and a pristine copy of the pinned workload, refuses existing output directories, verifies inputs before every invocation, and persists an append-only hash-chained partial journal. It does not install tools or silently select a different stack.

```sh
python3 -B bench/three_stack.py run \
  --cargo "$TC/bin/cargo" --rustc "$TC/bin/rustc" \
  --justrust "$FROZEN_SHIPPING_JUSTRUST" \
  --blaze "$SOURCE_BUILT_BLAZE_RA_JIT" \
  --toolchain "$TC" --source "$PRISTINE_SOURCE63" \
  --cargo-seed "$PUBLIC_REGISTRY_SEED" \
  --output "$FRESH_SHORT_ABSOLUTE_OUTPUT" --samples 6 --timeout 360

python3 -B bench/three_stack.py replay --output "$COMPLETED_OUTPUT"
python3 -B -m unittest discover -s bench -p 'test_three_stack.py'
```

The source seed contains only public crates.io `registry/src`, `registry/cache`, and `registry/index` trees. Do not copy an entire user Cargo home, credentials, global build caches, or target artifacts into it. The seed's provenance is a root-supplied public-source contract, not something inferred from directory names. Legitimate source modules named `Credentials` or `target` are not private credentials or compiled build artifacts.

The completed run retained exact executable/toolchain/source/seed identities, commands, statuses, stdout/stderr, shipping-client raw recordings and actual route metadata. Offline replay reparses the original tuple-valued test oracles, validates all six order permutations and fresh private state, and recomputes every phase's statistics. It does not execute a compiler or require installed benchmark executables. The source's 63 files were restored byte-for-byte with their timestamps, and all six benchmark-owned Blaze homes were stopped successfully.

Validation: **40 dedicated mocked tests passed**, including the timing-boundary regression, and the final exact-source benchmark Python suite passed **801 tests**. The actual frozen corrected harness passed all 126 native calls and independent raw replay. Root preflight path/seed-layout failures happened before native execution and are not measurements. A separate shipping-client pilot validated recording/route metadata and is excluded from the table.

## Initial run and detected confound, retained rather than hidden

The first run also passed all 126 semantic gates, but export revealed **88 justrust recordings: 42 measured calls plus 46 background `--no-run` prebuilds**. The benchmark wrote an edit and then performed expensive identity checks before starting the command timer. Speculative watchers could therefore do useful work outside the measured interval, and earlier round workers continued watching the shared source. That is not an accepted fair edit-to-result comparison.

The initial test-edit medians were Cargo 1.125 seconds, justrust 0.601 seconds, and Blaze 0.247 seconds. Those figures remain historical command-latency observations, **not the headline result**, and cannot isolate the cause of the difference. All six benchmark justrust daemons and the separate pilot daemon were stopped using their private homes, their PIDs were verified absent, and the corrected run used new state. No caches or unrelated workers were destroyed.

A future default-prebuild experiment must measure from the edit itself, account for work between commands, and keep background workers isolated between arms. Disabling the feature here does not show that its genuine agent-session benefit is zero.

## Portable evidence

The [retained evidence archive](../bench/results/three-stack-20261101/README.md) contains the corrected primary run and the separately labeled, confounded initial run. It preserves every sample, correctness receipts, identities, cleanup records and privacy-projected raw output. Independent offline replay and all 30 negative/isolation tests passed. Replay requires only Python, not installed compilers or the original private directories:

```sh
python3 -I -B bench/results/three-stack-20261101/validate.py
python3 -I -B bench/results/three-stack-20261101/validate.py --run initial
python3 -B bench/results/three-stack-20261101/test_archive.py
```

## What is still not established

This pinned common subset does **not** replace the session-weighted synthetic benchmark. In particular it does not measure:

- `check`, `clippy`, release builds, full test suites, doctests, cross compilation, or arbitrary production-function edits;
- Desktop/Jcode or a representative distribution of project sizes and dependency graphs;
- hosted execution, concurrent tenants, cache-poisoning/isolation behavior, network cost, or a fresh-machine toolchain download;
- quiet-machine repeatability, statistical significance, or broad Rust compatibility.

Waited-child CPU accounting also misses some resident/hosted work. Wall time is the end-to-end metric here. No pooled cold/warm overall score is reported. The next extension should add matched command/change workloads and a coverage table, then run the session-weighted Cargo/justrust comparison and Blaze's supported overlap without treating unsupported cases as wins.
