# Hosted justrust vs stock Cargo on a laptop: Jcode, ripgrep, uv

Measured 2026-11-21 on Jeremy's XPS 14 (Intel Core Ultra X9 388H, 16 logical
CPUs, 64 GB, Arch Linux) against justrust hosted (one 96-vCPU, 30 GiB guest).
Stock is `/usr/bin/cargo` 1.99.1 with the system rustc. Hosted is the
published client (`justrust build|test|check --remote`, updated through
`v0.1.24`). justrust local is not part of this comparison.

Scripts: `bench/warm_three.py` (edit loop) and `bench/cold_three.py` (first
build, release and dev/test). Raw samples, identities and run ids are in
`bench/results/`.

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
applies. Release samples count only if the binary is on the laptop
afterwards and runs `--version`; test samples count only with nonzero passed
tests.

### Release builds (quiet `epp=performance` stock baseline vs hosted `v0.1.24`)

| Workload | Stock (quiet, `epp=perf`) | Hosted (`v0.1.24`) | Prebuilt hits / misses | Speedup |
|---|---:|---:|---:|---:|
| Jcode `build --release -p jcode --bin jcode` | 539.0s (535.4, 542.6) | 140.1s (prior v0.1.20: 137.1s) | 565 / 40 | 3.85x |
| ripgrep 15.2.0 `build --release` | 9.7s (8.9, 10.6) | 7.6s (6.9, 8.3) | 34 / 0 | 1.28x |
| uv 0.13.9 `build --release -p uv` | 549.3s (547.2, 551.5) | 366.0s (prior v0.1.20: 383.2s) | 479 / 29 | 1.50x |

### Cold dev/test builds (`cold-three-ac-v0.1.24.json`, hosted `v0.1.24`)

| Workload | Stock (paired*) | Hosted (`v0.1.24`) | Hosted compile only | Prebuilt hits / misses |
|---|---:|---:|---:|---:|
| Jcode `test -p jcode-tui --lib jarvis_listen` | 224.5s* | 140.8s | 87.5s | 496 / 109 |
| ripgrep `test -p ignore --lib` | 10.7s* (9.6, 11.9) | 7.7s (6.8, 8.5) | 5.2s | 11 / 4 |
| uv `check -p uv-resolver` | 33.5s* (33.3, 33.8) | 58.7s (55.7, 61.7) | 19.2s | 394 / 67 |

\* See Fairness notes below: the laptop had background CPU load during the
`cold-three-ac-v0.1.24.json` stock samples, so those paired stock timings are
an upper bound on a quiet laptop.

- **Jcode** is the clear win in both release (3.85x) and cold test (140.8s
  end-to-end, 87.5s on host). 565 of 605 release dependency units came
  prebuilt, and the hosted machine compiles Jcode's own crates in parallel.
- **ripgrep** sees a modest win (7.6s vs 9.7s release, 7.7s cold test) after
  hosted admission and sync-overlap optimizations. It is small enough that a
  quiet laptop already builds it in about 10 seconds.
- **uv release** is a smaller win (1.50x) for a structural reason: its
  release profile is `lto = "fat"`, `codegen-units = 1`, so the final `uv`
  link and LTO step is one single-threaded unit (about 259s on host).
  Prebuilt dependencies and more cores cannot shorten it, and uv pins
  upstream rustc 1.100.0 rather than justrust's compiler.
- **uv cold `check`** is slower end-to-end on a brand-new checkout path
  (58.7s vs 33.5s local) even though hosted compile itself is faster (19.2s
  vs 33.5s, `justrust show cold-three-48e883aad47a4efba6678d83752b65cc`):
  uploading a fresh full `uv` repository tree over the WAN took 35.8s.

Hosted `v0.1.24` run ids (`bench/results/cold-three-ac-v0.1.24.json`): Jcode
release `cold-three-2799907a…`, Jcode test `cold-three-ca2388c2…`, ripgrep
release `cold-three-4f01f596…`, `cold-three-73b711be…`, ripgrep test
`cold-three-7f0cafe6…`, `cold-three-37dd475d…`, uv release
`cold-three-c6aa8047…`, uv check `cold-three-0979d54f…`,
`cold-three-48e883aa…`. Prior `v0.1.20` quiet-stock release baseline:
`bench/results/cold-three-20261121.json`.

### Fairness notes

- **Power state and background CPU load.** Both harnesses now record per-sample
  power state (`ac`, `epp`, `governor`, `platform_profile`). During the
  `v0.1.24` benchmark window, a background local `python3` process (~100% CPU)
  and screen recorder (~228% CPU, started 23:32) competed with local stock
  Cargo, and the laptop was unplugged for the final `jcode`/`uv` release
  samples (`ac=False`, `epp=balance_power`, where local stock slowed to
  680.4s / 719.3s). Hosted timings were unaffected because compilation runs
  on the remote host. To avoid overstating hosted speedups, the release and
  warm summary tables below compare `v0.1.24` hosted medians against the
  earlier quiet `epp=performance` stock medians (`cold-three-20261121.json`
  and `warm-three-20261121-010532-3a9641657108.json`), and flag the contended
  paired stock numbers explicitly.
- Hosted wall time includes source sync, admission/queueing, and downloading
  the release binary back to the laptop.

## Warm edit loop (`warm_three.py`)

One-line edit in a library file, then the command, 5 rounds each, after a
warm-up build on each stack. Every sample must prove the edited source was
compiled via rustc's unused-variable warning for that round's unique marker.

| Workload | Stock (quiet, `010532`) | Hosted `v0.1.24` (`215114`) | Result vs quiet stock |
|---|---:|---:|---|
| Jcode `test -p jcode-tui --lib jarvis` | 10.21s | 7.98s (7.81-9.14s) | hosted 1.28x faster |
| Jcode `check -p jcode --bin jcode` (multi-crate edit) | 9.41s* | 3.20s (3.16-3.26s) | hosted 2.94x* faster (2.6s host compile) |
| ripgrep `test -p ignore --lib` | 0.31s | 0.79s (0.74-1.44s) | stock faster by 0.48s |
| uv `check -p uv-resolver` | 0.63s | 1.11s (1.09-1.94s) | stock faster by 0.48s |

\* `jcode-bin` was added in the `v0.1.24` run (`215114`), where local
background load slowed stock builds (`jcode` stock 21.71s vs 10.21s quiet;
`ripgrep` 1.28s vs 0.31s quiet; `uv` 1.66s vs 0.63s quiet). Hosted `jcode-bin`
compiled both `jcode-tui` (2.25s) and `jcode` (0.16s) in 2.6s on host (3.20s
end-to-end; 4 verified rounds plus 1 round that attached to a remote `-pre`
prebuild in 3.22s whose raw log was not fetched locally).

For sub-second incremental steps (`ripgrep`, `uv`), the hosted round trip
(pooled admission + sync + dispatch + results back, ~0.45-0.5s) still exceeds
local compile time on a quiet laptop. Hosted wins clearly once the warm
compile takes multiple seconds (`jcode`, `jcode-bin`) or when the laptop is
under local CPU contention or battery throttling.

Result files: `bench/results/warm-three-20261121-010532-3a9641657108.json`
(quiet stock baseline) and
`bench/results/warm-three-20261121-215114-f18c1722b084.json` (`v0.1.24`,
hosted run ids `warm-three-77f72fa…`, `warm-three-af3f905…`,
`warm-three-1f98d2d…`, `warm-three-e97b039…`).

### Synthetic workload (`versus.py --synth 1`, partial)

A paired `stock` vs `remote` run of `bench/versus.py --synth 1 -n 1` captured
14 of 40 points (`bench/results/partial/synth1.json`) before a client release
(`v0.1.26`) invalidated the resume checkpoint and local screen recording
distorted stock timings. On heavy commands, hosted cut wall time sharply
(point 0: 22.15s vs 49.76s; point 11 full `jcode` workspace test: 57.9s vs
195.85s), while tiny sub-second checks (`0.19-0.45s` local) paid ~0.3-0.5s of
hosted round-trip overhead, and 2 full `jcode` test points hit host-dependent
test failures on remote Linux (8/6340 and 6/1774 tests failed on environment
assumptions such as local config paths).

## Optimizations and bugs fixed while benchmarking

1. **Hosted admission latency (`justrust-cloud` + `v0.1.24`).**
   - Moved `recordQueued` D1 writes into Cloudflare `waitUntil` in
     `~/justrust-cloud` (deployed to prod), cutting worker `queue` phase from
     ~100ms to `0.0 ms` on live traces.
   - Added a persistent pooled HTTPS admission client (`HostedAdmissionPool`)
     in the local `justrust` daemon (`src/remote_hosted.rs`,
     `src/remote_daemon.rs`) so warm requests reuse warm TLS connections to
     `build.jcode.sh` instead of paying a fresh TLS handshake per command.
   - Overlapped `Msg::PreSync` with hosted admission and added Linux inotify
     quiet-detection (`is_quiet_since` in `src/remote_watch.rs`) so `run()`
     skips redundant workspace rescans when no files changed after `PreSync`.
2. **Deleting one checkout failed builds of another** (fixed in v0.1.18 and
   v0.1.19). After a hosted build of checkout A, deleting A made the next
   build of checkout B fail with "another connection changed the source
   mirror". Now only a displaced root that the build actually reads blocks
   it.
3. **Hosted ran Jcode with 2 jobs instead of 14** (fixed in v0.1.20). The
   hosted pool inflates an idle guest's memory balloon to about 6 GiB. The
   agent sized `CARGO_BUILD_JOBS` from `MemTotal` at build start (5.4 GiB),
   so it chose 2 jobs on a 96-CPU machine. That cold Jcode build took 584s
   instead of 214s. Jobs are now sized from installed memory (online memory
   blocks, which the balloon does not change). Both post-fix samples ran 14
   jobs. They took 128-146s, against 214s for the one pre-fix sample that
   also got 14 jobs. All three had the same 565 prebuilt hits, so the
   cause of that gap is not established.
