# Hosted speed investigation, October 13, 2026

## Goal and measured state

The target is 10x faster than the standard local toolchain. It is not met.
Pinned `bench/suite.py` desktop snapshots were used, not synthetic replacement
source. The three-round versus result is
`bench/results/versus-20261013-021448-440bad50c73c-dirty.json`.

| Warm workload | Stock local | justrust local | Hosted |
|---|---:|---:|---:|
| Reference edit/test loop | 7.17 s | 4.63 s | 7.58 s |
| Body-edit check | 1.92 s | 1.37 s | 2.76 s |
| No-op test | 0.35 s | 0.32 s | 1.11 s |

The apparent 54x cold-check result compared a fresh local target against a
persistent, already-built remote target. It measures cache reuse, not truly
cold hosted compilation. `suite.py --remote` now rejects cold scenarios
that clear only local targets and verifies every sample actually ran remotely.

## Fixed-client hosted baseline

`bench/results/hosted-cargo-before-20261013.json` pins the alpha.14 executable
and nightly-2026-10-08, five samples after a discarded warmup. The hosted
target remains persistent. No-op test median is 1.400 s (1.149 to 1.572);
body-edit check is 2.895 s (2.832 to 8.565). The slow outlier spent 5.804 s
synchronizing while host compilation stayed 1.911 s. This shows why client
wall time and reconnect tails matter, not only rustc time.

## Shipped changes and acceptance

* `d657b71`: hosted benchmark mode and four Python regression tests.
* `623b85e`: reconnect invalidates every remembered manifest, but mirrors only
  roots requested by the next build. Watch events never replay an invalid
  manifest. Deleted roots are forgotten. Adds opt-in
  `JUSTRUST_REMOTE_TRACE` and slow-sync phase logging. Hosted check
  `20261013-072535095-521653`, targeted regression
  `20261013-072539913-522160`, and all-target warning-free clippy
  `20261013-072550747-524551` passed. No measured reconnect speedup yet.
* justrust-cloud `4cd6d38`: overlaps cached account resolution with the
  authoritative charge. Still requires successful billing and the queued
  telemetry write before HTTP 200. No user credentials, tenant cache scope,
  quota, or billing rules changed. 62 Node and six Python tests passed.
  Worker version `beba6b93-5ae1-4cb8-970e-0308c571fb8e` deployed and health
  checked. Real hosted validation afterwards passed check
  `20261013-074043475-677824`, full tests
  `20261013-074113970-684828` (329 passed, zero failed, two ignored), and
  warning-free clippy `20261013-074132836-689618`. Speed still unquantified.

## Recovery and next measurements

An Arch-built jr-cargo upload could not load Ubuntu
`libgit2.so.1.9`. It was immediately disabled before any benchmark, restoring
stock cargo. A native hosted build with vendored libgit2 is running. Do not
promote it until version/channel compatibility, real tests and fixed-client
before/after measurements pass. The optimized jr-rustc rebuild is separately
running in a reload-resilient systemd unit. It is not validated or promoted.

All checks and tests above used the hosted service. Global agent instructions
now require hosted dogfooding and explicitly documented local exceptions.
The release coordinator owns alpha.16 and macOS cross-platform agent
installation. These changes do not establish a 10x performance result.
