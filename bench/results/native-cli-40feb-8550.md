# Native CLI paired measurement: TypeWalk growth

## Verdict

**No wall-time speedup demonstrated on these four supported programs.** The 63-pair repeat has per-cell median paired after/before ratios from 0.993 to 1.022. Most cells are slightly slower. Small added overhead is possible and is not hidden or classified as a win. The initial 21-pair run's dyn Drop ratios of 1.02-1.04 shrank to 1.003-1.017 in the repeat. Variability overlaps substantially; these runs do not establish a reliable regression magnitude either.

There is a separate, reproducible **resource-accounting improvement** on the original growing-type CLI control. At workers 1 and 8, before fails with 32,134,173 accounted work after 129 applications. After reaches the intended depth129/limit128 diagnostic with 14,640,602 accounted work after 134 applications. More of the required admission executes without a premature work failure. This is neither a successful compilation of an unbounded generic program nor a wall-time speed comparison, and no limit changed.

## Exact scope and provenance

- Before `40feb23c8`; after `8550f407a`. `git diff --stat` over `src`, `libraries`, Cargo.toml and Cargo.lock shows only `src/compiler/traits.rs`: 109 additions/10 deletions, including the TypeWalk regression test. Thus this isolates paid geometric TypeWalk memo growth, **not** early-root scheduling or all graph-unification work.
- Both release binaries built from separate `git archive` trees using the installed stable `justrust build --release --bin jr-rust`, no experimental installation. Both hosted builds used pinned `nightly-2026-10-08`. Redacted receipts: `data/runs/graph-final-acceptance/native-pair-builds.jsonl`, IDs `20261018-210639833-3450836` and `20261018-210701541-3450829`. Build duration is provenance only, not the result being compared.
- Before binary SHA256 `3202c5ae4fdf41696e192802fc95ecbb73f67cf4c61fb81bdfd5276d5f173af5`; after `0d4c14303df322eff49dc3c205577b7ca29a74f0c1e33f546b6d567dc8dcb8fd`.
- Actual local `jr-rust SOURCE --workers 1|8 --check` and `jr-rust SOURCE --workers 1|8 -o NEW_OUTPUT` processes, unchanged hello/Collatz/GCD-Fibonacci corpus inputs and dyn Drop source. Every generated executable ran and matched a freshly compiled stock oracle's exit0/exact stdout/empty stderr. Native JSON summary schema/mode/workers, complete capture, zero dropped events, zero failures and check-only absence of executable were validated.
- Fixed random seed1 shuffles before/after order per pair; two warmups per cell; 21 measured pairs then an independent 63-pair repeat; 16 cells each. Total 2,816 native compiler processes including warmups. Source and binary hashes checked before and after. The measurement process did not build compiler code or alter main target artifacts.
- Warm filesystem, fresh compiler process each time. No cache flush, CPU pinning or controlled-frequency claim. Native reports label measurements `contended/uncontrolled`. No local full suite was intentionally run during the repeat, but host load and scheduler noise remain. This is not the legacy Cargo session-weighted workload, a cold-build score, or a general Rust compiler comparison.

## Larger repeat, milliseconds

Ratios are medians of paired ratios, not ratios of independently computed medians. Spreads are empirical p10-p90 order statistics, not confidence intervals.

| Source | Workers | Mode | Before median | After median | Before p10-p90 | After p10-p90 | Paired ratio |
|---|---:|---|---:|---:|---:|---:|---:|
| 01_hello | 1 | check | 9.384 | 9.398 | 8.71-11.37 | 8.86-11.39 | 1.014 |
| 01_hello | 1 | build | 9.819 | 9.992 | 9.04-11.60 | 9.01-11.89 | 1.013 |
| 01_hello | 8 | check | 8.587 | 8.537 | 7.97-9.77 | 8.05-9.84 | 0.996 |
| 01_hello | 8 | build | 8.468 | 8.610 | 8.15-9.71 | 8.19-9.77 | 1.011 |
| 03_collatz | 1 | check | 11.218 | 11.261 | 10.84-11.67 | 10.53-11.69 | 1.009 |
| 03_collatz | 1 | build | 11.131 | 11.224 | 10.41-13.39 | 10.43-13.37 | 0.993 |
| 03_collatz | 8 | check | 9.243 | 9.323 | 8.70-10.16 | 8.80-10.27 | 1.022 |
| 03_collatz | 8 | build | 9.684 | 9.887 | 8.82-11.12 | 9.02-11.20 | 1.019 |
| 05_gcd_fib | 1 | check | 15.303 | 15.265 | 12.82-16.28 | 12.73-16.24 | 1.000 |
| 05_gcd_fib | 1 | build | 13.104 | 13.084 | 12.52-15.70 | 12.64-15.83 | 1.006 |
| 05_gcd_fib | 8 | check | 10.587 | 10.573 | 10.08-11.59 | 10.17-11.78 | 1.018 |
| 05_gcd_fib | 8 | build | 10.131 | 10.125 | 9.77-10.55 | 9.84-10.56 | 1.004 |
| dyn_drop_owner | 1 | check | 12.683 | 12.859 | 10.87-13.46 | 11.05-14.08 | 1.017 |
| dyn_drop_owner | 1 | build | 12.786 | 12.691 | 11.22-14.98 | 11.27-15.12 | 1.009 |
| dyn_drop_owner | 8 | check | 10.502 | 10.740 | 9.61-11.30 | 9.86-11.65 | 1.016 |
| dyn_drop_owner | 8 | build | 10.165 | 10.231 | 9.71-11.31 | 9.93-11.49 | 1.003 |

Raw pair/order timing evidence is in `native-cli-40feb-8550.json` and `native-cli-40feb-8550-repeat.json`; bounded resource observations are in `native-cli-40feb-8550-resource.json`. Each timing result stores the runner hash and input hashes. Per-pair before/after timings are retained; execution order is reproducible from seed1.

## Reproduce

Build the two named revisions separately with identical release flags, then invoke `python3 bench/native-cli-pair.py --before BEFORE_BINARY --after AFTER_BINARY --before-revision 40feb23c8 --after-revision 8550f407a --scratch SCRATCH_DIRECTORY --output NEW_RESULT_JSON --repeats 63`. Existing output files are refused. The script requires stock rustc and the repository's original fixture paths. It does not wrap the compiler with stock fallback.

The runner's first trial incorrectly required empty compiler stdout and stopped before collecting results. The real CLI intentionally prints a JSON summary. The runner was corrected to validate that public summary and rerun from scratch. This was a harness error, not a compiler failure, and the aborted trial contributes no timing samples.
