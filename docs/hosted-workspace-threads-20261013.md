# Hosted workspace frontend thread trial, October 13, 2026

## Decision

Reject a six-thread default. Keep the existing eight-thread project wrapper
and default nightly compiler. The optimized JR compiler remains staged opt-in.
This trial does not establish a speedup or meet the 10x stock-local target.

## Controlled setup

- Frozen installed client: source `7096694`, SHA256
  `df08132d448939a857127f2fe8b039fa3996cee65b27f75d109415378a2caa41`.
- Own authenticated hosted M8azn.6xlarge, existing persistent target,
  optimized compiler `jr-b7907f1fe8f7`.
- Pinned Desktop snapshots: jcode `abff94eeb`, Desktop `7d38fb7`.
- Backend placement unchanged, accepted US-west. Actual client ingress is in
  the current KIX/SYD epoch, not the earlier SJC stock-comparison epoch.
- `JUSTRUST_BUILDD=0`, `JUSTRUST_CARGO_DAEMON=0`,
  `JUSTRUST_TEST_FILTER_COMPILE=0`, actual billing admission for every build.
- Two warmups then seven samples each for body check and test edit.
  Cargo config forces `JCODE_FRONTEND_THREADS`, without changing registry flags.
  Each measured UI rustc invocation independently verifies the actual value.
- The eight-thread recheck follows six threads and restores the warmed state.

## Results

`python3 bench/suite.py compare bench/results/hosted-workspace-threads8-recheck-20261013.json bench/results/hosted-workspace-threads6-20261013.json`

| Scenario | Eight-thread recheck | Six threads | Compare verdict |
| --- | ---: | ---: | --- |
| Body check | 2.419 s | 2.581 s | +7%, within noise |
| Reference test edit | 5.008 s | 4.968 s | -1%, within noise |

Host compile medians are effectively unchanged: body 1.128 versus 1.127 s,
reference 3.658 versus 3.638 s. Every reference sample runs three passing tests,
body checks run no tests, all 28 timing samples have `remote.machine=hosted`
and zero failed tests. Samples each compile one local UI unit, with 743 fresh.

## Real diagnostics acceptance

Separate one-sample type-error probes at each setting both produce the intended
`E0308` and exit 101. Six-thread first diagnostic is 1.441 s, eight-thread
1.492 s. These are correctness observations, not statistical timing wins.
`justrust show` confirms actual hosted execution and compile-failed verdicts:

- `20261013-131739026-2778643111`, six threads.
- `20261013-131742503-2778870902`, eight threads.

All four raw result files and exactly 30 measured timing/diagnostic summaries
are committed. Fourteen benchmark provenance tests pass. No production
compiler/thread configuration changes, private-cache deletion, host restart,
identity change, quota change or billing bypass occurred.
