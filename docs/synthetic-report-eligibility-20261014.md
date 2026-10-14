# Synthetic report eligibility and retained observations

Date: 2026-10-14. This reporting fix does not change compilation, workload generation, measurements, caches, or defaults.

## Observed defect

The actual published partial under `bench/results/scheduler-comprehensive-2172-20261014/` contains one noisy OFF point out of 40 requested points and no ON epoch. Wrapping its unchanged synthetic payload in the suite's ordinary result envelope allowed the old public `show` command to print a bold expected wait per call of 20.53 seconds. Comparing that envelope to itself printed a +0% comparison from one point without a partial/noise warning.

The pre-fix behavior was reproduced from immutable Git source `e1f772d`, not inferred from code inspection. It incorrectly presented an incomplete, uncontrolled subset as the sampled-session estimand. It did not establish any compilation speedup.

## Report contract

- Preserve raw point observations and annotate noisy or unknown noise provenance.
- Emit comprehensive expected-wait aggregates and comparative ratios only when every requested point and replicate is present, all noise markers explicitly say false, outcomes match the planned successful or expected-compile-error command, and timing summaries agree with the actual samples.
- Block malformed identities, missing outcome evidence, duplicate or out-of-range point IDs, measurement errors, suspect harness artifacts, and incomplete runs. Do not silently drop inconvenient points and call the surviving subset comprehensive.
- Compare identical generator, seed, sampling information, point plans, source snapshots, execution mode and machine context. Toolchain or optimization versions can intentionally differ, but changed commands or incompatible source/machine context are not a matched compiler comparison.
- Require complete valid counter coverage across all required replicates before emitting instruction ratios. Missing counters stay unknown rather than disappearing from the median. Otherwise valid wall comparisons may remain available without an instruction claim.
- Preserve positional stack renaming by mapping sample, statistic and error keys through the original stack identities, not dictionary value order. Do not mutate loaded input results.
- Retain valid complete clean reports and expected compile-error workloads. Malformed report data should produce an actionable blocked message rather than a traceback or empty-mean arithmetic.

`bench/synth/run.py` owns report eligibility and raw detail. The synthetic branch of `bench/suite.py compare` owns outer envelope identity checks and positional stack mapping. Neither changes the generator or executes new compiler jobs.

## Acceptance scope

Regression tests exercise the unchanged real noisy partial through public `show` and `compare`, complete clean fixture controls, expected E0308 outcomes, missing/noisy samples, incomplete coverage, incompatible plans, malformed provenance, stale summaries and renamed/reordered stacks. Clean fixture output is a reporting control, not a new benchmark or speedup result. Tests-first failure logs and final public command outputs are retained separately with source and artifact hashes.

Final acceptance passed all 13 reporting regression tests and all 199 benchmark tests. Fourteen independently rerun public commands cover the immutable pre-fix reproduction, fixed actual partial, clean renamed controls, intentionally faster synthetic control, malformed data, partial/full counters and public help. Source hashes were stable across those checks. Independent review reproduced the partial-counter fix and complete-counter positive control. Evidence, red/green logs and raw/published hashes are under `bench/results/synthetic-report-guard-20261014/`. The collector's `.py.txt` copy is redacted audit text, not a directly runnable script until operator paths are supplied.

An AST/source audit confirms that seven actual measurement/runner functions, the generator, weights and versus engine are unchanged relative to `e1f772d`. This audit establishes unchanged code boundaries, not completion of the performance gate.

The three actual compile command records and the partial payload remain unchanged. No new compile run, account-host call, installation, volume operation or security change is made by this repair. The full OFF/ON performance comparison still requires sufficient verified isolated scratch capacity. See `docs/synthetic-scratch-capacity-20261014.md` and `docs/scheduler-comprehensive-2172-20261014.md`.
