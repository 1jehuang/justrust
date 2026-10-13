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
stock cargo. A native hosted build with vendored libgit2 subsequently passed
loader and exact release/channel checks. It is enabled only on our account
host. The fixed alpha.16 client seven-sample comparison observes no-op
1.355 -> 0.997 s and body-check2.690 ->2.542 s, but `suite.py compare` labels
both within noise. No conclusive end-to-end speedup is claimed. Results are
`hosted-native-cargo-stock-before-20261013.json` and
`hosted-native-cargo-fork-after-20261013.json`. Normal hosted check, library
tests and warning-free lint passed after activation.

The stage2 PGO/BOLT compiler build finished in 39m52s. Its interface hash,
dependent link/load, test-filter and 40-round stress checks passed locally
(custom compiler validation requires this unpromoted local binary). It is
now installed on our hosted account only as an explicit nondefault candidate
under `toolchains/staged/jr-b7907f1fe8f7`. Its exact commit, Ubuntu loader,
native cargo and zero activation timestamp were checked. Older agents cannot
accidentally select this namespace.

Agent replacement failures found during dogfooding are fixed in 3687ab6:
the running executable is copied to an immutable per-inode snapshot before
accepting builds. Copies are published atomically without replacing an
already-running snapshot. Real recorded/raw/recorded checks after replacing
the installed binary pass and share fresh Cargo artifacts. All 14 protocol
tests passed; the release owner installed the fix and passed 330 full hosted
tests before alpha.16 publication.

All checks and tests above used the hosted service. Global agent instructions
now require hosted dogfooding and explicitly documented local exceptions.
The release coordinator owns alpha.16 and macOS cross-platform agent
installation. These changes do not establish a 10x performance result.

## Optimized compiler: controlled hosted reference comparison

The same frozen client (`compiler-stage-justrust`, built from 46d38a1 with
the existing seed toggle) ran both sides on the same account host, pinned
desktop/Jcode snapshots, persistent per-toolchain targets, one discarded
warmup and five measured edits. Both used the same native hosted jr-cargo.
Every sample verified hosted execution and the requested compiler ID.
Results are `hosted-jr-reference-before-20261013.json` and
`hosted-jr-reference-after-20261013.json`.

`bench/suite.py compare` reports:

```text
scenario                       A med   B med   delta      %  verdict
desktop-test-edit               7.78    6.54   -1.23   -16%  faster
desktop-body-check              2.70    3.10   +0.40   +15%  within noise
```

The exact reference median is 7.777 -> 6.545 s (1.19x), with rustc
compile 6.349 -> 5.332 s. Actual logs show all three fps_counter tests
passed on every reference sample. The body-check compile phase instead
regressed 1.566 -> 1.685 s, so no default compiler promotion was made.
This compares stock nightly hosted against optimized JR hosted, not stock
local against hosted. The earlier 7.17 s local reference is context only,
not a same-client controlled comparison against this newer compiler.

The new compiler also passed real hosted check
`20261013-085629718-1176060`, full tests
`20261013-085654660-1178588` (332 passed, zero failed, two ignored,
55 compiled units) and warning-free all-target clippy
`20261013-085719414-1181114`. Safe staging support passed the full hosted
332-test suite in `20261013-083002649-1025849`, with CLI namespace and
default-preservation integration coverage.

The next experiments target selective test codegen and cached Cargo startup.
Their explicit opt-in environment variables were previously dropped by the
hosted client. 98c4579 fixes the narrow whitelist and records public controls
in benchmark provenance. No credentials or machine-local executable paths
are forwarded. Hosted check, 333 full tests and warning-free lint passed.

## Explicit opt-ins: no proven end-to-end gain

Seven samples after a discarded warmup, same frozen 98c4579 client and
explicit JR candidate, separate codegen-filter and Cargo-daemon changes:

| Workload | Both off | Filter only | Filter and daemon |
|---|---:|---:|---:|
| Reference edit/test | 6.478 s | 6.376 s | 6.210 s |
| No-op test | 1.272 s | 1.120 s | 1.154 s |
| Body-edit check | 4.067 s | 2.809 s | 3.167 s |

Both `bench/suite.py compare` steps classify every change as within noise.
These options remain opt-in. Actual filter transitions alpha (two tests),
beta (one), unfiltered (three), alpha again (two) passed on hosted; Cargo
reports the environment dependency changing, and the cached alpha binary
restores the correct tests. Every measured Desktop test sample passed
exactly three tests and no-op samples compiled zero units. Files are
`hosted-optins-off-20261013.json`, `hosted-filter-only-20261013.json` and
`hosted-filter-daemon-20261013.json`.

## Authoritative admission: deployed, no proven full-build gain

Backend 3ff0c61 replaces two serial status reads with one statement, keeping
ledger counts and billed totals in the same read snapshot. Account and period
bindings remain separate, no multiplying join is used, and no auth, charge
transaction, quota, reservation, outbox or pricing rules changed. Ten billing
tests and the complete 435-test SQLite-backed backend suite passed, including
other-account/old-period spend, empty ledgers and cap exhaustion. The backend
already had Smart Placement, so no placement setting was changed.

Deployed subscription-api version
`2cccc5ee-149b-4666-84f0-8b8aa2855321` on October 13, 2026 at 16:19 UTC.
An initial health probe interrupted the measurement script after deployment;
both actual health endpoints subsequently returned 200, and the after-only
measurements resumed without a second deployment.

Eleven measured no-op builds on each side with the same frozen client,
compiler and controls give 1.288 -> 1.240 s. `bench/suite.py compare`
reports -4%, within noise. Every sample ran hosted, passed three tests and
compiled zero units. Results are `hosted-status-query-before-20261013.json`
and `hosted-status-query-after-20261013.json`. Twenty-one idempotent repeats
of an existing charged run observed admission medians 0.640 -> 0.370 s, but
these are not new-charge requests, and the after tail reached 2.783 s. No
conclusive full-build speedup is claimed. Repeats do not charge or requeue
completed telemetry.

## Restored native Cargo parallel test pool

The Cargo fork stores binaries under build/<package>/<hash>/out instead of
deps. Our runner rejected that layout and fell back to serial execution,
while the report ignored its pool-launch header and lost binary timing.
5c30aa5 restores eligibility only for the exact Cargo hash/path shape,
keeping rustdoc temporary executables inline. 4cf336e and 198209d repair
named-pool headers and show actual counts even when inline timing is unknown.
The installed source passed 336 hosted tests and warning-free lint.

Real hosted alpha/beta/unfiltered transitions pass, inline --nocapture reports
actual counts, repeated builds preserve fresh artifacts, and the full current
suite passed 336 tests across seven binaries with zero compiled units, run
`20261013-092457071-1354392`.

Pinned full-test before/after files `hosted-test-pool-before-20261013.json`
and `hosted-test-pool-after-20261013.json` use preserved 98c4579 and repaired
198209d clients, otherwise unchanged JR compiler and disabled opt-ins. All
five edited test runs pass the same 157 tests, zero failures, five compiled
units. `bench/suite.py compare` reports full-test edit 3.324 -> 2.971 s,
-11%, faster. No-op 1.096 -> 0.938 s is within noise. This is a measured
hosted improvement, not attainment of the 10x stock-local target.

## Detached Cargo CPU accounting

The opt-in daemon data above contains invalid total CPU metrics. Run
`20261013-091246820-1272766255` reports 0.027 s while its compiler unit used
6.81 s and sampled processes used 8.03 s. wait4 covers the caller, not the
detached worker. Do not interpret this as CPU or instruction savings.

b70dbbc corrects total CPU using the maximum of waited CPU, observed process
CPU and compiler CPU, never their overlapping sum. Observed-only values are
explicitly labelled as lower bounds. Full 337 hosted tests and warning-free
lint pass. Real reference edit `20261013-093920906-1440678992` now reports
8.44 s with sampled-process lower-bound provenance, unit CPU 6.70 s and all
three selected tests passed. `hosted-daemon-cpu-accounting-after-20261013.json`
records the diagnostic acceptance, not a speed claim.


## Frontend threading: measured, not promoted

`hosted-frontend-threads1-20261013.json` and
`hosted-frontend-threads8-20261013.json` use the same frozen b70dbbc client,
JR candidate, pinned sources and five samples after a discarded warmup.
Filter and daemon stay disabled. Explicit Cargo rustflags select 1 or 8
threads and pass timing, recorded in every command and benchmark provenance.
`bench/suite.py compare` reports reference 7.201 -> 6.819 s (-5%) and
body-check 2.962 -> 2.840 s (-4%), both within noise. No default is changed.
Do not compare timing-instrumented results against older uninstrumented runs
as a controlled gain.

Reference frontend medians barely move, 2.345 -> 2.311 s. Run
`20261013-094425981-1461275918` shows monomorphization graph walk 0.840 s,
macro expansion 0.774 s, dependency-graph serialization 0.717 s, symbol
partitioning 0.613 s and resolution 0.505 s. These are observed phase
hotspots, not additive costs: nested spans overlap. The earlier parallel
monomorphization branch was already within noise overall and is not
promoted. Future work must reduce those costs with correctness-preserving
changes, rather than repeatedly toggling threads.

After the CPU-reporting change, ordinary hosted check
`20261013-100133452-1575817` compiled zero units with 56 fresh, preserving
artifacts. Its first reconnect spent 3.136 s syncing despite zero build-time
files, so reconnect latency remains a separate optimization opportunity.
