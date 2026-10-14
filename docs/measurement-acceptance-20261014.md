# Measurement correctness acceptance, October 14, 2026

This is correctness evidence, not a compiler speedup comparison. Historical
profiles and scheduler noise flags have not been recalculated or cleared.

## Shipped changes

- `4809f23`: accumulate sampled CPU deltas per `(pid, start_ticks)` identity.
  Keep the last observed counter after exit or a missing snapshot. Overlapping
  roots do not count the same counter twice, and a regressing read cannot create
  a later lifetime-CPU spike. This uses the existing tracking map.
- `e629a00`: classify a compile failure only if the outer command also failed.
  Intentionally failing compiler probes inside successful tests retain their
  unit records, diagnostics, first error and error count without making the
  successful outer command appear to fail.

Installation used an exact clean `git archive` of
`e629a004068923360bc2cb8880ca160020c758f7`, not the mutable working tree.
Installed CLI SHA-256:
`08d0601aa8bfa33decdcdbeb986a2d9558ba0873d5f1b5d3659117c034660abe`.
The digest was checked before and after the installed hosted acceptance series,
and again after the real Desktop edit/restoration and nested-error fixture.

## Requirement-to-observation mapping

| Requirement | Actual observation |
|---|---|
| Preserve survivor CPU after a different child exits | CPU regression failed before the fix, passed afterward |
| Do not recount a process's lifetime after one missing snapshot | Regression failed before, passed afterward |
| Distinguish PID reuse and overlapping process roots | Regression failed before, passed afterward |
| Do not turn a regressing counter into a later spike | Regression failed before, passed afterward |
| Correct success despite intentional nested compiler failures | Twelve summary tests passed, including unit-only and captured-output cases |
| Preserve genuine compiler failures | Summary exit-101 cases passed; actual installed CLI emitted full E0308 and `FAILED to compile`, exit 101 |
| Preserve runtime test-failure classification | Summary runtime-failure regression retained the failed-test footer without calling it a compile failure |
| Exercise the installed CLI with a real nested compiler failure | Minimal real test invoked public `justrust cargo check`, expected E0308/101, retained its full output, and the outer command correctly reported two tests passed/exit 0 |
| Dogfood normal hosted routing and packaging | Installed check/full tests/strict all-target lint/recheck all succeeded on hosted with pinned nightly |
| Preserve unchanged Cargo artifacts | All four installed justrust acceptance runs compiled zero units and retained 62 fresh units |
| Validate a real reference body edit | Desktop fps_counter edit passed three tests with one compiled UI unit and 745 fresh units |
| Preserve unchanged Desktop reuse | Immediate repeat passed three tests, zero compiled units, 746 fresh units |
| Restore unrelated repository bytes | Restored from a backup, not Git; restored file SHA-256 exactly matched the original |
| Recording must not invalidate artifacts | Actual recorded/unrecorded/recorded minimal check shared one compiler/target, both repeats were Fresh without Dirty or recompilation, and rmeta bytes were identical |
| Retain failed experiments and harness mistakes | Original misleading 397-test success, four red regressions and one invalid `cargo --local` invocation are exported unchanged |

## Exact run IDs

Source validation, all hosted:

- Baseline check: `20261013-213034882-1395672`.
- CPU regressions before fix: `20261013-213037568-1395671`, four failures,
  exit 101. This was a real compiled red run, not only predicate inspection.
- Fixed check: `20261013-213334654-1409344`.
- CPU regressions after fix: `20261013-213336102-1409383`, four passes.
- Summary cases: `20261013-213338134-1409502`, twelve passes.
- Full suite: `20261013-213338635-1409542`, 404 passes, zero failures,
  three ignored.
- Strict all-target lint: `20261013-213351746-1409303`, exit 0.

Installed-client normal hosted acceptance:

| Command | Run | Result |
|---|---|---|
| check | `20261013-213913766-1436325` | exit 0, 0 compiled, 62 fresh |
| test | `20261013-213917356-1436666` | 404 passed, 0 failed, 3 ignored; 0 compiled, 62 fresh |
| strict all-target clippy | `20261013-213927754-1437567` | exit 0, 0 compiled, 62 fresh |
| repeated check | `20261013-213928341-1437621` | exit 0, 0 compiled, 62 fresh |

Real hosted Desktop reference workflow:

- Warm/preparation: `20261013-213943897-1438944`, three passes, 23 compiled
  units, 723 fresh. This is not an edit-latency comparison sample.
- One-line body edit: `20261013-214047661-1443635`, three passes,
  one compiled unit, 745 fresh.
- Unchanged repeat: `20261013-214056105-1444107`, three passes,
  zero compiled, 746 fresh.
- Exact restored source: `20261013-214103304-1444830`, three passes,
  one compiled unit, 745 fresh.
- Original and restored fps_counter source SHA-256:
  `24de30f7556c2e126bb52f685377f656c6352dfdf83e1bc9f9d03f037d877e8d`.

Local-only compiler-boundary fixtures, intentionally tiny and isolated:

- Recorded first check: `20261013-214136103-1453889`.
- Recorded repeat after unrecorded Fresh check:
  `20261013-214136356-1453972`, zero compiled, one fresh.
- Real expected nested E0308 with successful outer tests:
  `20261013-214356941-1470803`, exit 0, two passed, error count one,
  compile_failed false. The nested invocation did not append a failed unit to
  this outer run. Unit-only behavior is tested separately, not claimed here.
- Real outer E0308 failure: `20261013-214408640-1471875`, exit 101,
  error count one, compile_failed true.
- Initial unsupported `justrust cargo --local check` fixture invocation:
  `20261013-214116565-1445687`, exit 1 before compilation. Corrected to the
  documented local routing environment; this is a harness error, not a gain.

The original successful local run with the misleading compile verdict remains
`20261013-211707833-1300731`: 397 passed, three ignored, exit 0, one intentionally
failed reconnect_fixture compiler unit, original compile_failed true. It has not
been rewritten to pretend that the old client was correct.

All 21 cited IDs exist in `data/runs/hosted-west-measurement/2026-10.jsonl`.
`bench/results/measurement-acceptance-20261014.json` includes exact observation
counts, original summary hashes, cache-log hashes and restoration provenance.

## Remaining limits

Per-process sampled CPU is still an approximation: sampling can miss CPU between
an exit and the last observation, ancestry reads are not atomic, and a newly
observed process can already have performed work. The fix does not create query
DAG edges, a mathematical span, or a perfect external-contention classifier.
Short runs and the existing unweighted noise average still need conservative
interpretation. Neither old ABBA noise flags nor old timing records were changed.

This host remains the accepted west m8azn.6xlarge. No resize, permissions change,
worker deployment, compiler activation or Cargo scheduler promotion was made.
The scheduler candidate still has no demonstrated full-loop benefit and stays
default-off. Further compiler parallelism remains an unfinished optimization
track, not a proven 96-core utilization ceiling.
