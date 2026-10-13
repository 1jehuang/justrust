# Hosted caller-umask fidelity, 2026-10-13

Commit `7096694` fixes an execution-semantics difference, not a claimed speedup.
The real Ubuntu hosted agent inherited SSH umask `0002`, whereas the local
caller used `0022`. Seven unchanged pinned Desktop package/security fixtures
failed because their newly created directories were group-writable. The guards
correctly reject that mode. Neither those guards nor the pinned source was edited.

## Implementation and compatibility

The Linux client reads its mask from procfs without a process-global mutation.
Other Unix clients use an isolated shell. The mask travels as reserved metadata
through the existing Run environment field, with no protocol-version change.
The agent validates only octal permission bits, removes this metadata from the
Cargo environment, and applies it only in the forked run child before exec.
Concurrent children can therefore use distinct masks without changing the agent
or a legacy run's inherited mask. Invalid metadata fails before execution.
No compiler flag, Cargo cache key, billing, quota, tenant identity, or SSH/PAM
configuration was changed. A missing mask preserves legacy behavior.

## Actual hosted acceptance

All IDs below are committed in `data/runs/xps14/2026-10.jsonl` and say
`remote.machine=hosted`. The installed binary is built from `7096694` with the
existing unrelated seed-toggle working-tree hunk still excluded from this commit.
Frozen binary SHA-256: `df08132d448939a857127f2fe8b039fa3996cee65b27f75d109415378a2caa41`.

| Gate | Exact run | Observed result |
|---|---|---|
| Source check | 20261013-125941803-2676561 | success |
| Real subprocess/protocol tests | 20261013-125945235-2676755 | 21 pass, including concurrent mask isolation, invalid metadata, atomic replacement and recorded/raw/recorded artifact reuse |
| Source full suite | 20261013-125950463-2677479 | 351 pass, 0 fail, 2 ignored |
| Source all-target lint | 20261013-125956653-2676534 | success, no warnings |
| Installed plain default check | 20261013-130405588-2703785 | 0 compiled, 56 fresh |
| Installed plain default test | 20261013-130420296-2705417 | 351 pass, 0 fail, 2 ignored, 0 compiled, 56 fresh |
| Installed all-target lint | 20261013-130423685-2705593 | success, no warnings, 0 compiled, 56 fresh |
| Unchanged Desktop package/security tests | 20261013-130424881-2705665 | all 16 pass, 0 compiled, 744 fresh |

First connection after install costs 14.7 seconds, mostly 14.16-second sync.
The security-fixture call also includes 2.29-second first-project sync. These
are correctness acceptance calls, not warmed latency or speed comparisons.

## Full UI remains genuinely failing

Pinned snapshots: Jcode `abff94eeb`, Desktop `7d38fb7`.
Before the fix, default nightly full UI run `20261013-124730712-2603219`
reported 1295 pass, 8 fail, 10 ignored. After the installed fix,
`20261013-130428175-2703751` reports 1302 pass, 1 fail, 10 ignored.
The remaining viewport-position assertion fails in isolation too, default
run `20261013-130450230-2708373`. The optimized compiler
`jr-b7907f1fe8f7` passes the UI check (`20261013-130451633-2708430`) and
reports exactly the same full-suite 1302/1/10 (`20261013-130518595-2710929`).
A separate hint-chip failure passed the runner's isolated retry on both full
runs and remains visible in their logs. Neither full UI run is called clean.
The optimized compiler remains staged, not the default. This fidelity fix does
not establish the 10x target or justify ignoring baseline failures.
