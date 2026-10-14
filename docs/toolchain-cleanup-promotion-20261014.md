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

## Safe next step

Keep the installed candidate selectable with
`JUSTRUST_TOOLCHAIN=jr-910e39a6c3ef`, but retain the existing default and official
fallback. Global promotion requires a fresh, complete, controlled before/after
comparison with frozen executable identity and valid test outcomes. Do not
reuse this noisy baseline or advertise the partial observations as a speed win.
