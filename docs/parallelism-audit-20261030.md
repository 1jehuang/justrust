# Rust parallelism audit, October 30, 2026

## Effective settings

- Hosted Cargo already replaces conservative workspace job caps with the host's CPU count, bounded by roughly 2 GiB total memory per job. Explicit CLI job limits take precedence. The audited host exposes 96 CPUs and about 371 GiB RAM, so this cap is 96 jobs, not four.
- Independent crates, pipelined metadata/codegen and LLVM codegen already execute in parallel. A single dirty crate can still dominate an incremental build. Raising Cargo jobs cannot split that crate's frontend.
- Jcode and Desktop already configure their `rustc-parallel-frontend` wrapper, normally eight frontend threads for local unoptimized crates. It deliberately skips optimized units. The clean-room compiler uses opt-level 1 and had no such configuration at the start of this audit.
- The vendored compiler still defaults its frontend to one thread. Its backend defaults to available parallelism. Newer native `--jobs-frontend` controls do not mean the frontend automatically uses all CPUs.
- The test pool and libtest already parallelize tests. A recent clean-room full library test run (`20261029-175106375-937110`) averaged 29.2 CPUs, peaked at 73.1 and passed 2,149 tests. It is not a single-threaded test-runner problem.

## Bugs fixed

Hosted transport was dropping `CARGO_BUILD_JOBS`, `RUST_TEST_THREADS`, the Jcode frontend controls and the justrust test pool/splitting controls. These explicit limits and opt-outs now survive the hosted boundary. `JUSTRUST_FRONTEND_THREADS` is forwarded for the native Cargo experiment too. No secrets or local wrapper paths were added to the allowlist.

Thread telemetry now recognizes both split `-Z threads=N` and native `--jobs-frontend[=]N`. Backend/global job limits alone are not mislabeled as frontend threading.

Benchmark liveness sampling used to abort if rustc exited between `pgrep` and reading `/proc/PID/stat`. A missing process is now conservatively counted as live, preserving a noisy sample rather than falsely declaring it quiet. Permission errors and malformed/reused identities still fail closed. The measurement-policy identifier was versioned so old/new classifications cannot be silently mixed.

## Pinned hosted measurements

All initial comparisons use the clean-room snapshot `7bc3f6e5f`, nightly `2026-10-08`, the same hosted route and three measured samples after a warmup. Results preserve all noise flags. These are scoped diagnostics, not a comprehensive promotion gate.

| Workload | One frontend thread | Eight frontend threads | Interpretation |
|---|---:|---:|---|
| Warm incremental filtered test edit | 7.732 s | 7.760 s | No demonstrated benefit |
| Full library recheck, incremental explicitly disabled | 13.331 s | 6.110 s | Promising frontend-heavy case, but comparison is uncontrolled because some samples are noisy |
| Warm incremental library check | 3.271 s | 3.174 s | Within the suite's default 10% noise floor, compiler phase 2.44 vs 2.49 s |

Sixteen frontend threads did not improve the full-check median: 6.86 s versus 6.11 s at eight, with noisy samples. The incremental test medians also remained flat (7.66 s at sixteen). There is no evidence to request all 96 frontend workers.

See `bench/results/parallelism-rewrite-20261030-t{1,8}.json` and `bench/results/parallelism-rewrite-fullcheck-20261030-t{1,8}.json`. Run `python3 bench/suite.py compare --floor 0 A.json B.json` to retain the conservative verdict. Do not describe the full-check difference as a proven general speedup.

The earlier explicit `jr-ac20de5e7921` hosted attempt (`20261029-180437903-1009058185`) failed because that compiler prefix was not installed on the host and no trusted distribution store was configured. The suite rejected the system-toolchain fallback. Both successful sides were rerun using the available official pinned nightly, rather than comparing different compilers.

## Deployed native policy and measured result

Source commit `a237096f6` implements workspace-only selection inside jr-cargo, not another compiler wrapper. `JUSTRUST_FRONTEND_THREADS` is read from Cargo's resolved `[env]` configuration or the process environment with normal override/force precedence. The setting is applied only with an actual nightly/dev compiler. Explicit compiler thread/global-job flags win. Build scripts, nonmember dependencies, std and rustdoc are excluded. Command construction and fingerprints use the same policy.

A properly versioned release candidate was built with `toolchain/cargo/build-jr.sh nightly-2026-10-08`, run `20261029-182119955-1161004`. Artifact SHA256 is `ea7f219a5283f26b2dfe59938c9706b9ab9233cb03d13ff9ab5f77e37027b0a6`; Cargo reports upstream commit `f3865b2a4` and jr-cargo source fingerprint `5abb73e57230a73e4504dbae6f518def2e651424` (the latter is a source fingerprint, not a Git commit). It is installed into local `nightly-2026-10-08` and `jr-ac20de5e7921`, and checksum-verified before installation into the active hosted nightly prefix.

The deployed native policy was then measured at one and eight threads, with **all six measured samples quiet**. `bench/suite.py compare` reports full nonincremental recheck **13.43 -> 6.04 s, -55%, faster**. Compiler time was 12.62 -> 5.31 s. Recorded average compiler CPU occupancy rose from approximately 1.0 to 4.34 cores, not 96. CPU time rose from approximately 12.66 to 23.21 s, a deliberate wall-time versus CPU-work tradeoff. Each recording was checked for the actual expected `frontend_threads` field, not only an environment setting. Exact results: `bench/results/parallelism-native-fullcheck-20261030-t{1,8}.json`.

A second deployed-policy comparison kept warm incremental test edits within noise: **7.67 -> 7.79 s**, with all measured samples quiet. Exact results are `bench/results/parallelism-native-testedit-20261030-t{1,8}.json`. This scoped full-recheck win is not a warm-test speedup.

The clean-room checkout now opts in via `.cargo/config.toml` `[env] JUSTRUST_FRONTEND_THREADS = "8"`, committed and pushed on its `main` as `a0cb0e435`. This is a scoped workspace setting, not a global compiler default. `JUSTRUST_FRONTEND_THREADS=0` disables it. Stock Cargo/stable rustc ignore the policy without needing global unstable flags or `RUSTC_BOOTSTRAP`. Existing Jcode/Desktop wrappers were not replaced or retuned.

Native validation includes three policy unit tests, four real Cargo integration tests with zero ignored, and a 236-test Cargo library suite before the final env-config-only addition. The integration cases verify unaffected dependencies/build scripts stay fresh, count changes invalidate affected workspace units, repeated identical settings stay fresh, and explicit/config/force precedence. Cargo strict Clippy remains blocked by ten preexisting violations, separately documented in `docs/parallelism-audit-native-20261030.md`; the shipping client's full hosted tests and strict Clippy passed.

## Real workspace and cache acceptance

Normal, unmodified hosted commands from the live clean-room checkout passed after activation:

- `justrust check`, `20261029-182734419-1184789`: three workspace units compiled, fourteen fresh, actual frontend count eight for every compiled local unit.
- `justrust test --lib engine::tests`, `20261029-182737850-1185038`: seven passed, no failures, eight frontend threads recorded.
- Repeated `justrust check`, `20261029-182744439-1184788`: zero compiled, fifteen fresh.

A controlled local compatibility fixture also verified recorded/unrecorded/recorded artifact sharing with the same pinned nightly and native eight-thread setting. First recorded check `20261029-182822648-1187784` compiled one unit. Direct unrecorded jr-cargo then reported `Fresh`, and final recorded check `20261029-182822856-1187913` compiled zero units. This local-only check tests the recorder boundary, not build performance. No source edits or cache wipes were used to obtain freshness.

## Comprehensive gate status

The required `bench/suite.py run --synth 1 -n 1` baseline was attempted twice. Server reloads interrupted the first at four and the second at eleven of forty points. The second partial is retained as `bench/results/parallelism-20261030-before-interrupted.json`, explicitly `partial: true`. It is not a completed baseline, is not compared with a substitute workload, and cannot authorize a blanket default promotion. No comprehensive speedup is claimed.

## Client deployment and acceptance

The validated client is installed at `~/.cargo/bin/justrust`, SHA256 `cff1a234f4866595a15153fafbd0611baba78affb7e0162d1c54d7827e42fe57`. Its staged identical executable passed a real forced-hosted test, `20261029-181739342-1143678`, asserting exact `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=3`, `JUSTRUST_FRONTEND_THREADS=8` and `JUSTRUST_TEST_JOBS=2` in the remote test process. This verifies transport end to end rather than only checking the allowlist.

## Scope

No many-core resize, dependency codegen-unit change, blanket LTO change, replacement of working project wrappers or claim of maximum compiler parallelism follows from this audit. Useful parallelism is judged by agent wait, correctness, cache behavior and memory, not by keeping all 96 cores busy.
