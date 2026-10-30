# Query retention benchmark protocol, 2026-10-30

## Scope

This experiment asks whether persisting three already serializable, proven-green
query values improves the complete agent workload. It does not measure a global
maximum of reusable compiler IR. The compiler option is tracked and default-off:
`-Zjr-cache-small-query-values=no` versus `yes`. Native implementation is commit
`88936fa84`. The correctness and mechanistic acceptance evidence is described in
`query-value-retention-experiment.md`.

Both modes use one frozen compiler, matching standard library, rustdoc and
Clippy. Bootstrap completed `./x build --stage 1 compiler/rustc library
src/tools/rustdoc src/tools/clippy`. The resulting sysroot was copied with
`justrust toolchain jr-install --no-default` into a private scratch
`JUSTRUST_HOME`, taking Cargo from the installed `nightly-2026-10-08` toolchain.
No shipping compiler default was changed. Local execution is necessary because
the candidate compiler exists only locally, not on the hosted fleet.

The staged ID is `jr-50f30aea8dbb`, but that version string alone is not a unique
identity for our modified compiler. Exact frozen artifact SHA-256 values are:

| Artifact | SHA-256 |
|---|---|
| rustc | `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144` |
| rustc driver | `502401b14a878674fddf862e4fe7c7dca0948da8a7b8e8aefd3543ffd57a0eb7` |
| rustdoc | `d1f729df3439f673ab2cc355137b4cffc65043bfdbe672e1eb1b7f97c047b995` |
| clippy-driver | `a053ad57cc123544ba694d4735aa565803d865d2079af9f6b4b6941ea9c367d6` |
| cargo-clippy | `99ef9b1854d35b8a091098c16510ee15d99f88123a227e4e9dd672765a130369` |
| jr-cargo | `ea7f219a5283f26b2dfe59938c9706b9ab9233cb03d13ff9ab5f77e37027b0a6` |

These artifacts are private experimental inputs, not published under an existing
shipping compiler cache identity. The dependency artifact cache and local result
cache are disabled. The benchmark disables public prebuilt artifacts and forces
local execution. Native incremental compilation remains enabled wherever the
unchanged pinned workload enables it. No source snapshot, Cargo manifest,
command weights, or generator is changed to make a candidate pass.

## Comprehensive comparison

Use seed 1, all 40 points, one repetition per point, the same `retention` stack
name, and the same compiler, Cargo, work roots and execution mode. Only the
experimental compiler flag changes. Each point's own prebuild warms its exact
inputs before its edit and measured command. Cold prebuild wall is not a
session-weighted edit-loop sample and must be reported separately if cited.

```sh
export JUSTRUST_HOME="$HOME/.jcode/scratch/query-retention-bench-20261030/home"
export JUSTRUST_BENCH_WORK="$HOME/.justrust/bench/work"
export JUSTRUST_VERSUS_WORK="$HOME/.justrust/bench/versus"
python3 bench/versus.py run --synth 1 -n 1 --only none \
  --stack 'retention:JUSTRUST_TOOLCHAIN=jr-50f30aea8dbb,JUSTRUST_DEPCACHE=0,JUSTRUST_LOCALCACHE=0,JUSTRUST_REMOTE=0,RUSTFLAGS=-Zjr-cache-small-query-values=no' \
  --no-report --no-instructions
# After the complete baseline, repeat the exact command with the flag set to yes.
# Then use: python3 bench/suite.py compare BASELINE_JSON CANDIDATE_JSON
```

The private home must be set on the benchmark process, not merely in its stack
overrides: the runner reads quiet-mode test summaries from its own
`JUSTRUST_HOME`. An initial launch with a stack-only home was cancelled during
its first unmeasured prebuild to correct that provenance issue. It produced no
accepted comparison. Reusing completed dependency artifacts from that cancelled
prebuild does not replace any point's required prebuild.

## Current evidence and decision

The 22-case native acceptance matrix passed for both modes with incremental
fingerprint verification, matching diagnostics, exit statuses and runtime
output. A separate small native marker fixture demonstrates that the selected
values are actually retained rather than merely accepting an unused flag:
`hir_owner_parent_q` green-provider recomputations fell from 8 to 0, `constness`
from 8 to 0, and `coroutine_kind` from 13 to 0. Those are fixture scope-entry
counts, not percentages of all IR or a wall-time improvement.

The comprehensive baseline is in progress. There is no before/after performance
result yet and no default promotion. All points, coverage errors, command/change
breakdowns, noise and cache/storage costs must be reviewed before a decision.
Even a successful result would establish only a measured frontier for this
candidate and workload, not the theoretical maximum reuse.
