# Rust parallelism audit, October 30, 2026

## Effective settings

- Hosted Cargo already replaces conservative workspace job caps with the host's CPU count, bounded by roughly 2 GiB total memory per job. Explicit CLI job limits take precedence. The audited host exposes 96 CPUs and about 371 GiB RAM, so this cap is 96 jobs, not four.
- Independent crates, pipelined metadata/codegen and LLVM codegen already execute in parallel. A single dirty crate can still dominate an incremental build. Raising Cargo jobs cannot split that crate's frontend.
- Jcode and Desktop already configure their `rustc-parallel-frontend` wrapper, normally eight frontend threads for local unoptimized crates. It deliberately skips optimized units. The clean-room compiler uses opt-level 1 and has no such configuration.
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

See `bench/results/parallelism-rewrite-20261030-t{1,8}.json` and `bench/results/parallelism-rewrite-fullcheck-20261030-t{1,8}.json`. Run `python3 bench/suite.py compare --floor 0 A.json B.json` to retain the conservative verdict. Do not describe the full-check difference as a proven general speedup.

The earlier explicit `jr-ac20de5e7921` hosted attempt (`20261029-180437903-1009058185`) failed because that compiler prefix was not installed on the host and no trusted distribution store was configured. The suite rejected the system-toolchain fallback. Both successful sides were rerun using the available official pinned nightly, rather than comparing different compilers.

## Scope

No many-core resize, dependency codegen-unit change, blanket LTO change, replacement of working project wrappers or claim of maximum compiler parallelism follows from this audit. Useful parallelism is judged by agent wait, correctness, cache behavior and memory, not by keeping all 96 cores busy.
