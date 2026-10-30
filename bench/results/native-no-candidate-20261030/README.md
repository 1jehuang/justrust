# Native no-candidate installed acceptance evidence

See `../../../docs/daemon-native-no-candidate-installed.md` for interpretation,
exact installed identity, run IDs and limitations.

- `baseline-*`: saved previous installed binary, new flag ignored, zero native
  stamps. Probe compiled. Not a matched performance baseline.
- `installed-*`: installed new binary, concurrent actual Cargo projects, two
  native stamps, zero coalescing, successful unattached probe, owner cancelled
  only after its own requesting socket disconnected. Probe artifact was fresh,
  so do not compare its wall time with baseline compilation.
- `shared-workspace.log`: actual normal-checkout local Clippy. The initial
  implementation rebuild is excluded from the two warm observations.
- `shared-daemon-selected.log`: narrowly retained source-validation attribution
  for those three local requests, without unrelated projects' log entries.
- `edited-restore.log`: complete intentional compile error and restored success.
- `accept.py`, `blocker/`, `probe/`: real socket harness and independent Cargo
  source fixtures. To rerun, copy the fixtures and script to a fresh scratch
  directory and pass an absolute candidate executable, expected native-stamp
  count (2 for the new flag), and result label. The harness requires the normal
  installed `~/.justrust/toolchains` and nightly-2026-10-08, creates an isolated
  state home, and stops only that test daemon. This is local-only acceptance,
  not a recommendation to disable hosted routing normally.

Full-process wall observations under contention are not a comprehensive
session-weighted speedup or grounds for default promotion. No completed Cargo
result is replayed by this path. Run timestamps are October 29 local PDT,
corresponding to October 30 UTC.
