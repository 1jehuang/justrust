# Recorded slow-run inventory

`slow_runs.py` reads every local run directory's `meta.json` and `summary.json`
without invoking Cargo, changing runs, or exporting thousands of summaries.
Only an explicitly requested output file is written. Python 3.9+ and the standard
library suffice.

```sh
python3 bench/slow_runs.py --threshold 10 --recent-days 7 --limit 12
python3 bench/slow_runs.py --as-of 2026-10-26T21:25:00Z \
  --output bench/results/slow-runs-20261026.json
python3 -m unittest discover -s bench -p test_slow_runs.py -v
```

The committed snapshot counts **36,667 run directories**, including **36,404
measured summaries**, **244 missing summaries**, and **19 malformed/unreadable
summaries**. All measured summaries had valid wall numbers. Metadata coverage is
reported separately. Counts can change while builds run: this is a complete
traversal, not an atomic snapshot. Compared with an earlier CLI inventory, more
runs have completed. No absent summary is treated as a fast or successful run.

## Priorities as of October 26, 2026, 21:25 UTC

All-history totals: **13,111 runs strictly over 10 seconds**, with **786,672.814
seconds of aggregate excess**. Seven-day totals: **5,294 slow runs**, with
**382,876.680 seconds of excess**. These sums include overlapping concurrent
runs, experiments, failures, and benchmarks, not elapsed user waiting time.

Prefer current workloads over historical outliers:

| Last 24h real-workload candidates | Slow runs | Aggregate excess seconds |
| --- | ---: | ---: |
| clean-room compiler | 237 | 5,307.556 |
| jcode | 159 | 2,500.144 |
| shipping justrust | 49 | 971.461 |
| Total | 445 | 8,779.161 |

1. **Current priority: compilation in real-workload candidates.** The largest
   measured phase is compile for 367 last-day slow runs, accounting for
   6,024.120 excess seconds. Example `20261025-234813569-416231` records
   141.020 seconds in clean-room tests, including 137.778 compile seconds and
   3.138 test seconds. `20261025-205537709-3403713` records a successful local
   jcode build at 260.220 seconds, including 259.096 compile seconds. Inspect
   rebuild reasons and per-crate telemetry before selecting a compiler/cache
   optimization. These examples establish cost, not a proven fix or speedup.
2. **Investigate failed test tails separately.** jcode run
   `20261025-220129313-3793128` records 389.862 remote seconds, 362.271 labeled
   tail, but only 18.556 labeled test execution. The reported client wall is
   399.020 seconds. Inspect output and process lifetimes before interpreting
   tail as idle time, linking, shutdown, or a hang. Last-day failures contribute
   3,151.606 excess seconds across 138 slow runs.
3. **Seven-day clean-room test problems remain the largest historical bucket.**
   Clean-room contributes 207,805.562 of 223,345.907 excess seconds among recent
   real-workload candidates. Run `20261022-042033839-2216985` records 18,361.016
   remote seconds and 607 failed tests. Its measured `build_gaps` is
   17,321.812 seconds, but test-binary lifetimes extend throughout much of that
   interval. Do not mistake incomplete phase attribution for 17,321 seconds
   of reclaimable idle time. Runs `20261022-093454990-3331888` and
   `20261022-094502510-3383256` record failed test runs of 4,332.801 and
   3,517.857 seconds. Reproduce current tests before optimizing old failures.

## Interpretation and privacy

- `wall` means the recorded build-side wall clock. It is **not full agent wait**.
  Remote `client_wall` is separately counted when available, never added to
  build wall or silently substituted. The snapshot has 13,822 client-wall
  measurements, including **1,031 runs where client wall exceeds 10 seconds
  but recorded build wall does not**. Build-only selection misses these waits.
- Each scope ranks workspace, command, dominant measured phase, outcome,
  workload category, and execution place by aggregate excess and total wall.
  Group rankings concern slow runs only. Scope totals include fast runs too.
  Each ranking retains at most 20 groups with omitted-group totals, three
  example IDs per group, and at most `--limit` detailed runs per scope.
- Dominant phase is a deterministic attribution heuristic, not root cause.
  Compiler unit sums, test timings, link timings, remote sync, and top-level
  phases can overlap. The tool does not sum these into a fabricated timeline.
- Path/argument heuristics separate benchmark and scratch workloads. Ordinary
  workspace runs remain candidates, not certified production use. Failures
  are retained and ranked separately. Unknown dates are excluded from recent
  scopes. The fixed last-24h scope is independent of `--recent-days`.
- Paths, arguments, diagnostics, and machine identities are not emitted.
  Known workspace names are retained; others use stable hashed labels.
  Run IDs permit local investigation without publishing raw summaries.

This is an optimization queue and coverage audit, not evidence that an
optimization has shipped. No before/after performance claim is made.
