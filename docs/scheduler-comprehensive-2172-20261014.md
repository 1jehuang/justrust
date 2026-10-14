# Comprehensive weighted scheduler gate: attempted, storage-blocked

Date: 2026-10-14. Benchmark repair: `2172a450bfeffb04b217d56f83ec4fdab70a9958`.
Evidence: `bench/results/scheduler-comprehensive-2172-20261014/`.

## Result

**There is no scheduler speedup result and no hardware/core-ceiling result.** The unchanged comprehensive seed-1 workload requested 40 points with one sample per point. Only the first scheduler-OFF point was measured. It was noisy. The storage safety guard stopped before measuring the next point, and scheduler ON never began. A `bench/suite.py compare` result cannot be produced from these inputs.

This follows the completed compiler graph/elapsed diagnostics and the real benchmark environment repair, not a default promotion. The graph's CPU work and execution span remain unknown. Fixing lost benchmark controls is a measurement improvement, not evidence that compilation got faster.

## Frozen execution context

The trial used ordinary authenticated account-host transport to run isolated **host-local compute** on the existing 24-core west machine. This is the controlled host-compute baseline, not agent-visible hosted round-trip timing. No credit, quota, tenant-ownership or cache-publishing boundary was bypassed.

| Item | Identity/control |
|---|---|
| Private client source | `059674bf76fa218ba7e475279a86b4b6caf32aac` |
| Private client SHA256 | `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d` |
| Candidate Cargo source | `a45c3546671925c24649e451a673aa71b1385d0d` |
| Candidate Cargo SHA256 | `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d` |
| Pinned compiler | `nightly-2026-10-08` |
| Compiler SHA256 | `66877d44efc77e5d5478c756abf48574c5ce4d41a1102518b20e2ff3f3a0ba56` |
| Cost-hint SHA256 | `29bb6618b1ca614cfed13c3bd77777954f2380f757aeeb2fd799abe2040e44d8` |
| Jobs | `CARGO_BUILD_JOBS=24` |
| Candidate selection | `JUSTRUST_CARGO_FORK=1`, daemon `0`, shadow `0` |
| Observed scheduler epoch | Critical-path mode `0` only |
| Workload | Seed `1`, 40 points, `-n 1`, generator `86207da453da` |
| Guard | Stop before another invocation when free space is below 8 GiB |

The observer wraps the unchanged `versus.run_counted` interface, checking the exact executable environment for pre-build, previous-command and measured calls. It then checks actual record lookup in the private home, actual Cargo/compiler selection, recorded jobs/local routing, and process exit equality with the recorded summary exit. Hash checks are epoch-boundary/trusted-owner consistency, not continuous attestation or proof that a queue reorder occurred.

A concurrent peer release `bc7e3cb` changed the mutable public source/release context and included the exact environment-repair documentation and four local acceptance exports. It did not replace this private trial's fixed client, Cargo or harness. Do not label the trial as using the new global release.

## Exact observed command results

All three commands exited zero. Records are exported in `data/runs/broad-scheduler-2172/2026-10.jsonl` and joined to raw record hashes in `execution-off.jsonl` and `acceptance.json`.

| Exact run ID | Role and command | Recorded wall | Executable wall |
|---|---|---:|---:|
| `20261014-081653879-1167169777` | Unmeasured pre-build: `build --release -p jcode-desktop` | 121.101 s | 121.141 s |
| `20261014-081855187-1167169410` | Point 0 measured: same release build after the generated UI edits | 20.508 s | 20.529 s |
| `20261014-081945795-1167169753` | Next point's unmeasured pre-build: `test -p jcode-desktop-ui --lib --no-run` | 55.687 s | 55.719 s |

The measured call is **not a cold entire-workspace build**. Its 121-second pre-build established target state first, and the plan then edited the UI crate across six generated edit locations. The first point's shape label is retained unchanged in `partial-off.json`.

The first pre-build recorded up to 20 foreign rustc processes, average other-core usage about 1.48 and CPU-pressure time about 21.70 seconds. The measured call observed zero peak foreign rustc and about 0.163 average other cores, but the runner's start-load gate was 8.91 and explicitly classified the sample `noisy=true`. Its process-tree CPU from `run_counted` is 114.46 seconds, not a per-query CPU-work/span measurement. The entire epoch is not an uncontended speed baseline.

## Preparation and storage boundaries actually exercised

1. At 08:03 UTC the isolated harness stopped before measurements: excluding all results had also excluded one committed report-test fixture. The exact fixture was copied into that isolated harness, not omitted or skipped. All 186 benchmark tests then passed on the host.
2. The 08:05:57–08:13:24 attempt completed and cached the unchanged generator's stock probe tables. It stopped at the 8-GiB guard before its first actual measured/pre invocation. The earlier free-space observation was about 6.2 GiB. The initial private setup had about 18 GiB free.
3. Only this new trial's **unmeasured, regenerable stock-probe build outputs** were reclaimed. An initial clean of their parent container correctly refused because that container had no `CACHEDIR.TAG`. That safeguard was not bypassed and no tag was fabricated. Standard Cargo clean succeeded only for the three actual child cache roots with existing tags, verified canonical non-symlink paths. About 11 GiB was recovered. Probe tables/hashes, snapshots, logs and all existing service/private caches were preserved.
4. The 08:16:53–08:20:44 resumed attempt reused those exact probe tables, unchanged generator/weights, private identities and controls. It measured one OFF point, completed the next pre-build, and stopped below the same space guard before the next measured call. No ON receipt exists.
5. The actual record/hash/export retrieval finished at about 08:23 UTC. The account-host lease was explicitly released at 08:28 UTC. No further compiler, host-status, SSH, installation or provisioning calls are planned by this attempt.

## Requirement-to-observation map

| Requirement/boundary | Observed result |
|---|---|
| Do not silently lose intended synthetic controls/home | Fixed in `2172a45`, nine regressions and real private check/test/recheck passed |
| Whole benchmark test interfaces | All 186 tests passed locally and in the isolated host harness |
| Immutable client/compiler/Cargo/cost inputs | Actual start/final hash checks agree; initial identity inventory retained |
| Exact child execution controls | Three pre/measured child environments asserted; mode OFF, jobs24, fork1/daemon0/shadow0 |
| Public command/compiler integration | Three actual command records confirm private candidate/compiler, jobs and local routing |
| Actual process/record outcome consistency | All three process and summary exits agree at zero |
| Generator/seed/weights/workload identity | Seed1/40/n1/generator fingerprint and actual first point retained |
| Complete OFF epoch | **Blocked** after one noisy point |
| Complete ON epoch | **Not entered** |
| Matched broad compare | **Unavailable**, no fabricated or subset promotion |
| Error and guard preservation | Missing-fixture, storage stops and Cargo parent-clean refusal retained/documented |
| Existing data and security | Source/service/measured caches preserved, no volume/reboot/IAM/general-sudo change |
| Packaging/record evidence | Three exact exports and artifact manifest audited; independent compact review reproduced joins |
| Improvement/hardware decision | **Unresolved**. No speedup, core ceiling or default promotion |

## Model and remaining safe-next-step limitations

The bounded package-cost model was derived from two earlier CHECK controls: `20261014-034928222-288986662` and `20261014-035026203-288986929`, already exported in `data/runs/hosted-west-cargo-scheduler/`. It uses coarse successful package-unit wall estimates, excludes ambiguous names and preserves the earlier concurrent-build confound. It is neither a true execution DAG nor an independently optimal scheduler model. The complete training receipt and exact hints are retained.

Read-only investigation of the existing cloud implementation found no verified authorized online filesystem-growth mechanism: the inspected worker role does not grant ModifyVolume/SSM/Reboot, bootstrap removes general sudo and permits only its directory-creation wrapper, and recurring owned boot scripts handle readiness/idle behavior rather than volume growth. Effective guest cloud-init growpart/resizefs behavior was not observed. This does **not** say AWS cannot resize EBS, nor that an administrator could not arrange it. It means this bounded attempt did not verify a safe permitted growth path and did not reboot, resize, broaden IAM or relax sudo to force a result.

The next complete comparison needs sufficient separately arranged scratch capacity with verified guest filesystem growth, or a suitably sized disposable benchmark host, while retaining the same immutable source/control/authorization checks. Do not delete historical/user/service caches or replace this failed broad gate with a Desktop-only speed claim.

Artifact copies of the observer and launchers are explicitly redacted text for audit, not directly runnable commands until private paths are supplied by the operator. Raw successful run records and original logs remain in private scratch storage; public copies redact absolute operator/home paths and the private SSH host address.
