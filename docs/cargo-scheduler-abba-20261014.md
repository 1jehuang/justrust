# Cargo critical-chain scheduler: pinned 24-job ABBA

October 14, 2026. This is a completed correctness/performance experiment, not a
scheduler promotion or proof of a hardware parallelism ceiling.

## Result

The uniform-cost ready-unit scheduler produced **no demonstrated full-loop
benefit** in this experiment. It remains default-off. The complete before/after
measurements are committed under
`bench/results/scheduler-a45-j24-20261014/`, including all four segments and an
invocation audit. No noisy samples were removed or reclassified.

Actual `bench/suite.py compare` output:

| Pinned workflow | OFF median | ON median | Original suite verdict |
|---|---:|---:|---|
| Cold check, shared caches enabled | 9.45 s | 9.36 s | uncontrolled |
| Cold check, all justrust caches disabled | 27.77 s | 28.01 s | uncontrolled |
| Reference edit and three FPS tests | 4.98 s | 4.96 s | uncontrolled |
| Body-edit check | 1.32 s | 1.29 s | within noise |
| Unchanged filtered tests | 0.18 s | 0.18 s | uncontrolled |

The body-check row is not a causal gain either: conservative independent host
activity evidence marks the earlier epoch uncontrolled even where the original
sampler said `noisy=false`. It also falls below the suite's 10% noise floor.

Each mode has four measured samples per workflow. The full sweep ran from
03:46:59 through 04:10:58 UTC, in OFF/ON/ON/OFF order. Forty measured runs and
sixteen warmups all exited zero. All filtered test invocations reported three
passes and zero failures. The final quiet-period ON/OFF segment medians were
9.36/9.43 s cached cold, 27.73/27.82 s uncached cold, 4.98/4.96 s reference,
1.29/1.33 s body check, and 0.18/0.18 s unchanged tests. These are retained
controls, not a cherry-picked speedup or a replacement for the full ABBA.

## Immutable experiment identity

| Component | Actual identity |
|---|---|
| justrust source | `ec42e0f2e92eb429ac5a020419f03e9a1288a15c` |
| justrust binary SHA-256 | `2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53` |
| Cargo candidate source | `a45c3546671925c24649e451a673aa71b1385d0d` |
| Cargo binary SHA-256 | `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d` |
| rustc binary SHA-256 | `66877d44efc77e5d5478c756abf48574c5ce4d41a1102518b20e2ff3f3a0ba56` |
| Compiler | `nightly-2026-10-08`, rustc 1.101.0-nightly, `1d81eb4ad` |
| Copied installed-marker SHA-256 | `edb72c0b08476c6bfe91300cbc3b247fbb6c9c73e7d0d0979d94d04ac3b1401f` |
| Workload snapshots | Desktop `7d38fb7`, jcode `abff94eeb` |
| Machine | west m8azn.6xlarge, EPYC 9R05, 24 reported processors |

The exact accepted executable was copied from an immutable scratch artifact,
not the concurrently mutable globally installed CLI. A private JUSTRUST_HOME
and private benchmark work directory copied the accepted installed nightly
prefix. Only that private prefix's `bin/jr-cargo` was replaced. Production Cargo,
compiler selection, quotas, tenant isolation, worker and host type were not
changed by this experiment.

The benchmark repository revision and dirty status in the result describe the
launcher checkout, not whether the immutable CLI was built from dirty source.
The ancillary `env.cargo` probe describes PATH's system Cargo 1.99.0, not the
Cargo selected for these builds. Each actual invocation independently selected
the private candidate Cargo shown above. Cargo's embedded version reports its
upstream commit, not the candidate's actual source commit.

Controls: 24 Cargo jobs, fork selection enabled, daemon and shadow disabled,
auto-update and auto-install disabled. Uniform costs, no hint table. Desktop's
workspace wrapper reported eight frontend threads on actual UI compiler units.
These were deliberately host-local computations on the hosted machine, not
service-latency or network comparisons. Cold targets were newly created and
removed by the suite; a shared-cache cold check is not an uncached compile.

## Observed acceptance and limitations

| Requirement | Actual observation |
|---|---|
| Select the candidate, not an official-Cargo fallback | All 56 invocation metadata files identify the private candidate and pinned compiler, correct nightly, local route and 24-job environment |
| Preserve immutable inputs throughout | All four segment guards and final audit passed binary/compiler/marker SHA checks |
| Exercise cache-enabled and cache-disabled cold work | Both cold scenarios processed 887 compiler units per run; original cache hit/miss and compiled counters retained |
| Exercise agent edit/check/test loops | Reference and body edits rebuilt one unit; unchanged tests rebuilt zero; FPS tests passed |
| Preserve cache reuse and failure behavior | Separate public Cargo integration acceptance in `cargo-critical-path-acceptance-20261014.md` covers freshness hashes, errors/retry, jobserver limits and pipelining |
| Preserve noise and independent observations | Original flags retained; the audit adds known-confound annotations without altering them |
| Preserve exact cited run records | All 56 audited records exported from the private home using shipped `justrust export`, repository-root working directory; committed under `data/runs/hosted-west-cargo-scheduler/2026-10.jsonl` |
| Audit published result numbers | Every measured recorded-wall, unit count, test count and other-core figure checked against its actual summary; IDs match exact exports |
| Verify OFF/ON control | Launcher explicitly sets the switch; each segment's captured environment matches its intended mode. The frozen recorder does not record this switch per invocation or emit runtime branch telemetry |
| Preserve normal service defaults | Candidate exists only in a private copied prefix; host lease explicitly released after the successful sweep |

Independent concurrent Desktop release work was observed and its owner
confirmed completion at 03:58:45 UTC. Its entire earlier activity history is
not known, so the audit conservatively marks all eighteen logged runs beginning
before that completion uncontrolled. Minor platform SSH/probe intervals are
also retained where they overlap a run, without claiming an impact magnitude.
The live globally installed CLI changed concurrently, but the private selected
binaries and their observed invocation paths did not.

A separate read-only review found a frozen sampler defect: it subtracts sums
of *currently alive* descendants' lifetime CPU ticks. Exiting children can make
that sum shrink and suppress real surviving-child CPU increments. The derived
`avg_other_cores` can therefore be inflated. Non-atomic PID/ancestry snapshots
are another limitation. These findings do not erase confirmed external activity
or authorize rewriting old noise flags. Repair the sampler and establish a new
measurement epoch before stronger utilization/noise conclusions.

## What to do next

This ready-unit priority change cannot make a single remaining UI crate use
more processors. The observed one-unit edit workflows illustrate that boundary,
not a fixed limit on the compiler's eventual internal parallelism. Before
choosing hardware, repair attribution and then measure actual dependency edges
and per-unit costs, including Metadata/All pipelining, instead of inferring a
causal span from profile occupancy. A measured-cost trial or lower-job-count
trial is a new experiment, not an assumed improvement. Compiler-internal changes
remain subject to the compiler checkout's local policy/review gates.
