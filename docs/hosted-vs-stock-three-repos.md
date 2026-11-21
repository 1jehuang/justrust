# Hosted justrust vs stock Cargo on a laptop: Jcode, ripgrep, uv

Measured 2026-11-21 on Jeremy's XPS 14 (Intel Core Ultra X9 388H, 16 logical
CPUs, 64 GB, Arch Linux) against justrust hosted (one 96-vCPU, 30 GiB guest).
Stock is `/usr/bin/cargo` 1.99.1 with the system rustc. Hosted is the
published client (`justrust build|test|check --remote`). justrust local is
not part of this comparison.

Scripts: `bench/warm_three.py` (edit loop) and `bench/cold_three.py` (first
build). Raw samples, identities and run ids are in `bench/results/`.

## Why hosted can be faster

- **Prebuilt dependencies.** A first build can download dependency artifacts
  the prebuild fleet already compiled for the exact toolchain and flags.
- **Reuse of prior work.** Builds can reuse artifacts from earlier builds
  on the same hosted account.
- **Optimized toolchain.** Unpinned projects can build with justrust's own
  compiler (`jr-af1ccf35…`).
- **Hosted compute.** A large machine can run many more rustc jobs at once
  than a laptop.

Each of these is a "can". Where it did not help, the tables below say so.

## First build of a fresh checkout (`cold_three.py`)

Every sample is a new `git archive` extraction at a new path with an empty
`target/`. Stock gets its crate sources fetched beforehand, unmeasured.
Hosted uses a fresh `JUSTRUST_DEPCACHE_NAMESPACE`, so the account's own
caches start empty, like a new account. The public prebuilt store still
applies. A sample counts only if the release binary is on the laptop
afterwards and runs `--version`.

| Workload | Stock | Hosted | Prebuilt hits / misses | Speedup |
|---|---:|---:|---:|---:|
| Jcode `build --release -p jcode --bin jcode` | 539.0s (535.4, 542.6) | 137.1s (127.9, 146.3) | 565 / 40 | 3.93x |
| ripgrep 15.2.0 `build --release` | 9.7s (8.9, 10.6) | 8.4s (6.7, 10.1) | 34 / 0 | 1.16x |
| uv 0.13.9 `build --release -p uv` | 549.3s (547.2, 551.5) | 383.2s (355.3, 411.2) | 479 / 29 | 1.43x |

Medians, with the individual samples in parentheses.

- **Jcode** is the clear win. 565 of 605 dependency units came prebuilt, and
  the hosted machine compiles Jcode's own crates in parallel.
- **ripgrep** is a near tie. It is small enough that a laptop builds it in
  about 10 seconds, so prebuilt dependencies save little.
- **uv** is a smaller win than Jcode, for a structural reason. Its release
  profile is `lto = "fat"`, `codegen-units = 1`, so the final `uv` link and
  LTO step is one single-threaded unit: 259s of the 347s hosted build
  (`justrust show cold-three-44f0bc05…`). Prebuilt dependencies and more
  cores cannot shorten it. Hosted also builds uv with uv's own pinned rustc
  1.100.0, not justrust's compiler.

Hosted run ids: Jcode `cold-three-99ded8bc…`, `cold-three-d0f6d1ec…`. uv
`cold-three-4ea11e77…`, `cold-three-44f0bc05…`. ripgrep
`cold-three-4e69475b…`, `cold-three-9336bddc…`.

### Fairness notes

- The laptop was on battery for the whole session. The stock samples in the
  table were run with EPP set to `performance` on every CPU (RAPL 65 W), the
  same policy the laptop uses on AC. Each sample records its power state.
  Earlier stock samples taken in battery mode (`balance_power`) were 10-21%
  slower (Jcode 590.6s, uv 666.1s median). They are kept in the result file
  marked `superseded` and are not used.
- The hosted Jcode samples were taken after fixing two bugs this benchmark
  found (below). Earlier hosted samples are also kept, marked `superseded`
  or failed.
- Hosted wall time includes source sync, queueing and downloading the
  release binary back to the laptop.

## Warm edit loop (`warm_three.py`)

One-line edit in a library file, then the command, 5 rounds each, after a
warm-up build on each stack.

| Workload | Stock | Hosted | Result |
|---|---:|---:|---|
| Jcode `test -p jcode-tui --lib jarvis` | 10.21s | 8.11s | hosted 1.26x faster |
| ripgrep `test -p ignore --lib` | 0.31s | 0.78s | stock faster by 0.47s |
| uv `check -p uv-resolver` | 0.63s | 1.04s | stock faster by 0.41s |

For small incremental steps, the round trip to hosted (sync, dispatch,
results back, about 0.4-0.5s) costs more than the compile it offloads.
Hosted wins only when the incremental compile itself takes several seconds,
as with Jcode. Jcode's very first hosted sync of the whole repository took
197s. That one-time cost is excluded from the warm numbers.

Result file: `bench/results/warm-three-20261121-010532-3a9641657108.json`.

## Bugs found and fixed while benchmarking

1. **Deleting one checkout failed builds of another** (fixed in v0.1.18 and
   v0.1.19). After a hosted build of checkout A, deleting A made the next
   build of checkout B fail with "another connection changed the source
   mirror". Now only a displaced root that the build actually reads blocks
   it.
2. **Hosted ran Jcode with 2 jobs instead of 14** (fixed in v0.1.20). The
   hosted pool inflates an idle guest's memory balloon to about 6 GiB. The
   agent sized `CARGO_BUILD_JOBS` from `MemTotal` at build start (5.4 GiB),
   so it chose 2 jobs on a 96-CPU machine. That cold Jcode build took 584s
   instead of 214s. Jobs are now sized from installed memory (online memory
   blocks, which the balloon does not change). Both post-fix samples ran 14
   jobs. They took 128-146s, against 214s for the one pre-fix sample that
   also got 14 jobs. All three had the same 565 prebuilt hits, so the
   cause of that gap is not established.
