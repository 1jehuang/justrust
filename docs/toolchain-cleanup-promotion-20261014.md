# Toolchain cleanup promotion gate, October 14, 2026

## Outcome

Source/docs cleanup and reversible legacy retirement are complete. The owned
release compiler and clean committed Cargo are installed as the **nondefault**
`jr-910e39a6c3ef`. Correctness acceptance is recorded in
`docs/toolchain-activation-20261014.md`. The default remains `jr-ac20de5e7921`.
No speedup or successful performance promotion is claimed.

## Comprehensive workload attempts

The required commands used `bench/suite.py run --synth 1 -n 1`, 40 points,
explicit old/candidate toolchain IDs, the same generator and pinned workloads,
and local execution. Before/after outputs were deliberately separate.

1. Task `706392rm53` ran all 40 baseline points, then rejected the run because
   another session replaced the installed justrust executable. It did not
   save a valid result. The candidate command in that chain did not execute.
2. Task `875034zlyx` used a frozen justrust copy, SHA-256
   `575813196c3029a0071c19d2e8e47709cc332da672efdbb5103d106b46691786`.
   It completed and saved the 40-point baseline, then a server reload
   interrupted candidate preparation. The baseline is committed as
   `bench/results/toolchain-cleanup-before-20261014.json`.
3. The candidate resumed in a detached, reload-resilient process using the
   same frozen binary. A subsequent reload did not kill it. Fifteen points
   were recorded before cancellation on the blocking evidence below.
   Private benchmark snapshots were explicitly restored using the harness's
   backup restoration functions. No live project source was overwritten.

## Why promotion is blocked

`bench/suite.py show` reports **Expected-session report blocked** for the
baseline: all 40 points are noisy or have missing noise provenance. Multiple
passing-plan test points also have inconsistent failure/outcome provenance,
and point 28 is flagged as a suspect cache/pre-build artifact. Thus completing
more candidate points cannot rescue this baseline into a valid comparison.
The candidate's first 15 points were also noisy, with unrelated compiler
activity and substantial system load. No other session was stopped to obtain
better numbers.

The candidate was stopped rather than spending another long build cycle on a
comparison already invalidated by its baseline. Its raw partial observations
are preserved as
`bench/results/toolchain-cleanup-after-partial-20261014.json`. That file is
explicitly partial and blocked; environment metadata was captured **after**
abort and is labeled accordingly, not claimed as original startup metadata.
`bench/suite.py compare` rejects the pair. These are evidence of a blocked
promotion gate, not an estimated speedup or a full after run.

All 55 observed baseline/candidate run IDs have redacted exports in
`data/runs/xps14-toolchain-cleanup-gate/2026-10.jsonl`. This includes baseline
run `20261014-093757263-2242292135` and candidate run
`20261014-101956044-2917310751`, both with detected foreign compiler activity.

## Outcome failures investigated after the initial gate report

The outcome blockers are not merely missing telemetry. Baseline workspace run
`20261014-094536977-2242292635` and candidate workspace run
`20261014-104409647-2917310344` both exited 101 with the same seven confirmed
failing tests:

- `auth::tests::browser_suppressed_inside_test_harness_without_env_overrides`:
  assertion that a test binary should be detected with a custom target directory.
- `provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list`:
  observed model catalog did not equal the fixture's two-entry expected catalog.
- `tool::bash::tests::repository_commands_export_a_logged_cargo_function`:
  assertion that execution occurs inside a discoverable Jcode repository.
- `tool::tests::tool_descriptions_stay_under_token_cap`: descriptions exceeded
  the snapshot's 20-token cap, including the codemode description.
- `tool::tests::tool_parameter_descriptions_stay_under_token_cap`: a Gmail
  parameter description exceeded the snapshot's 25-token cap.
- `server::debug_command_exec::tests::debug_tool_selfdev_reload_returns_promptly_for_direct_execution`:
  could not discover a Jcode repository directory.
- `tool::tests::post_tool_feedback_hook_appends_stdout_to_tool_result`:
  expected hook-output assertion failed.

A separate wake-delivery timeout passed the harness's isolated retry and was
not counted among these seven. Full assertion output remains in each run's
`output.jsonl` and is accessible with `justrust log <id>`. The matching failure
names are evidence of existing baseline problems for these cases, not proof
that the entire candidate workload is equivalent or passing. Other baseline
points also have confirmed failures. A fresh promotion gate must establish
valid supported snapshot/test-environment behavior in addition to low noise;
do not suppress these tests or reinterpret exit 101 as a passing timing sample.
These pinned Jcode fixture/environment failures were not fixed by modifying the
user's live projects during a toolchain-cleanup task.

## Safe next step

Keep the installed candidate selectable with
`JUSTRUST_TOOLCHAIN=jr-910e39a6c3ef`, but retain the existing default and official
fallback. Global promotion requires a fresh, complete, controlled before/after
comparison with frozen executable identity and valid test outcomes. Do not
reuse this noisy baseline or advertise the partial observations as a speed win.
