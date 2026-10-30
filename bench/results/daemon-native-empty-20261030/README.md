# Raw candidate-free daemon diagnostic evidence

See `../daemon-native-empty-20261030.json` for the paired experiment manifest,
all sample run IDs, wall times, hashes, flags and limitations. See
`../../../docs/daemon-native-empty.md` for the safety boundary and acceptance
interpretation.

- `noop-*.log`: six off and six on `check --lib` samples.
- `clippy-noop-*.log`: three off and three on `clippy --lib` samples.
- `warm-*.log`, `clippy-warm-*.log`: final pinned warmups, excluded from medians.
- `failure-*.log`, `restore-*.log`: actual source failure and byte restoration,
  including full error output. Restoration wall times are not matched speed
  comparisons because the first invocation rebuilt and the second was fresh.
- `daemon.log`: actual daemon source-validation and lookup timings. Its first
  two requests at 22:48 local time are exploratory system-toolchain warmups,
  excluded from the final pinned results. Final pinned measurements begin at
  22:52 local time, following coordinator source/build freeze confirmation.
- `cancellation-result.json`, `cancellation-daemon.log`: an actual sleeping
  Cargo build script was terminated after killing only its requesting CLI.
- `overlap-result.json`, `overlap-daemon.log`: actual overlapping agent and
  explicit prebuild socket requests were refused without replacing or sharing
  the native owner. Disconnecting the owner then stopped its build script.

All log timestamps are America/Los_Angeles local time, October 29, 2026.
The experiment took place October 30, 2026 UTC. The machine remained subject
to unrelated CPU/memory load. These are focused diagnostics only, not the
comprehensive session-weighted suite or grounds for default promotion.
