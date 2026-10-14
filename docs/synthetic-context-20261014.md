# Synthetic benchmark execution-context repair, 2026-10-14

## Scope and outcome

Commit `2172a450bfeffb04b217d56f83ec4fdab70a9958` fixes a concrete measurement defect, not compiler speed: the single-stack comprehensive suite previously recorded public optimization settings but discarded them before launching its synthetic children. It also discarded the selected `JUSTRUST_HOME`, while the runner looked for records and installed the selected toolchain in that home.

`versus.stack_env` now supplies the runner's selected recording home for justrust stacks. Stock still receives no implicit justrust context. Only the single-stack suite explicitly copies the bounded public optimization allowlist, optional hint path and explicitly supplied update/auto-install controls into its stack. Normal versus baselines still scrub ambient optimization settings. Scenario and explicit stack overrides retain their previous precedence. The runner owns routing and run IDs.

Public provenance includes `CARGO_BUILD_JOBS` and the valid optional hint file's digest, not its raw private path. Credentials and unrecognized experimental variables are not propagated by the new allowlist. This is not a general environment sanitizer or an attestation of scheduler behavior.

## Requirement-to-observation map

| Requirement | Observed check | Result |
|---|---|---|
| Private record/toolchain home reaches synthetic execution | Single-stack suite test plus real `exec` child | Failed before fix, passes afterward |
| Explicit scheduler, fork, daemon, shadow and jobs controls are not silently lost | Public suite entry test followed by `stack_env`, including real child | Pass |
| Every public optimization control, including explicit zero, reaches execution and reported provenance | Allowlist matrix regression | Pass |
| Normal versus baselines do not inherit the candidate's scheduler/jobs settings | Default justrust and stock boundary tests | Pass |
| Explicit stack precedence is unchanged | Scenario/default-home/stack override regression | Pass |
| Runner routing/run IDs remain owned by the runner | Force-routing ambient variable is scrubbed, local run ID supplied | Pass |
| Private hint path is executable context, digest is public provenance | Valid-hint regression | Pass |
| Missing optional hints preserve Cargo fallback without a claimed valid digest | Missing-hint regression | Pass |
| No new default flags or global toolchain selection are invented | Unset and system-toolchain regressions | Pass |
| Existing benchmark interfaces remain usable | All 186 benchmark tests, suite/versus `--help` | Pass |
| Actual justrust uses the selected private Cargo/compiler and records | Tiny check, one actual test, recheck | Pass, exact exports below |
| Compiler speed, weighted scheduler activation/reordering or complete CPU execution span | Not established by this repair | No claim |

Eight initial tests were added before implementation. Four failed, including the real child missing `JUSTRUST_HOME`. After the repair, those eight passed. A ninth all-control/zero-value regression was added and passed. Independent read-only review also ran the original eight and the complete 185-test pre-matrix suite. Final owner validation ran all 186 tests successfully.

## Actual private-context workflow

This acceptance deliberately runs **locally**, because the changed boundary is a local benchmark child's selected home and record lookup. It does not substitute for hosted performance measurement. No existing project, global client or installed toolchain was changed.

Frozen client source: `059674bf76fa218ba7e475279a86b4b6caf32aac`.
Client SHA256: `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`.
Private candidate Cargo SHA256: `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d`.
Compiler selection: `nightly-2026-10-08`, private copied prefix.

| Exact run ID | Actual command | Observed result |
|---|---|---|
| `20261014-005234369-3084814456` | check | Exit 0, one compiled unit. First helper then failed because it incorrectly expected an arbitrary JUSTRUST flag in filtered `meta.env` |
| `20261014-005414405-3093505455` | check | Exit 0, one fresh unit |
| `20261014-005414445-3093505147` | test | Exit 0, one test passed |
| `20261014-005414740-3093505330` | check | Exit 0, one fresh unit |

The corrected helper verified each run record in the selected private home, actual `real_cargo` and `real_rustc` private paths, metadata jobs `2`, and local routing. The critical-path flag was verified in the exact environment passed to the executable, not falsely attributed to this client's filtered metadata. A one-crate fixture does not prove ready-queue reordering. All four path-redacted summaries are exported in `data/runs/synthetic-context-20261014/2026-10.jsonl`.

Foreign laptop compilers were observed during the functional acceptance. No timing or speedup claim is made.

## Comprehensive follow-through

A separate private-prefix host trial was started with frozen client/Cargo/compiler identities, seed 1, 40 points, one sample per point, jobs 24, scheduler OFF then ON, and bounded storage/time gates. Its outcome must be reported separately. The initial isolated preparation correctly stopped before measurements because excluding all benchmark results also omitted one committed report-test fixture. That exact fixture was copied into the isolated harness, all 186 tests passed there, and the original failed preparation log was retained.

The measured observer checks the actual executable environment, then actual recorded Cargo/compiler/jobs/local routing and recorded outer exit against the process result for pre-build, previous-command and measured invocations. Epoch boundary hashes are not continuous attestation. Sequential OFF/ON cache warming, model-training confounds, noise, missing/error points and storage limits must be audited before any performance interpretation. No compiler or production default is promoted by this repair.
