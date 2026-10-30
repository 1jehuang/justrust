# Linear ordered remote-sync manifest diff

`remote_sync::diff` previously traversed each manifest and performed one BTree
lookup in the other map for every entry. Both maps are already ordered. The new
implementation merges the two iterators, advancing at least one iterator per
comparison, then appends either remaining tail. Tree traversal is linear in the
combined entry count rather than two sets of logarithmic lookups.

The `changed` and `deleted` vectors remain independently path-sorted, with exactly
the original paths and stamp equality semantics. `UNKNOWN` is not special-cased:
old UNKNOWN versus a real new stamp is changed, old UNKNOWN versus no new entry
is deleted, and UNKNOWN versus UNKNOWN remains equal exactly as before.

The preserved lookup oracle compares 343 possible manifests over three ordered,
prefix-containing paths, with seven absent/stamp states. All 117,649 ordered pairs
must have exactly equal `Diff` vectors, not just equal sets. This covers empty
maps, exhausted tails, interleaving, additions, deletions, timestamp/size/mode
changes, UNKNOWN, and extreme integer fields.

## Focused measurement, 2026-10-30 UTC

Run the ignored diagnostic against the real local checkout:

```sh
justrust test --local remote_sync::tests::diff_real_manifest_diagnostic -- --ignored --nocapture
```

Local mode is intentional: it measures the full vendored local source manifest,
not the hosted mirror. Scanning and manifest construction are outside timing.
Each algorithm is measured seven times, reporting its median in the debug test
profile. The timed interval includes result construction and destruction.

| Case, same 68,219-entry input pair | Lookup oracle | Linear production diff |
|---|---:|---:|
| Unchanged | 319.28 ms | 11.05 ms |
| Sparse mode changes | 284.12 ms | 11.41 ms |
| Add/delete plus mode changes | 319.36 ms | 10.37 ms |

An earlier production-baseline run on 68,218 entries measured 328.47, 278.33, and
284.81 ms respectively. One source file was added before the after scan. The
controlled comparison above therefore uses the preserved original lookup
algorithm and new production diff on identical manifests in the **same after
run**, not the cross-session measurements. Build contention and debug profiling
also limit interpretation. This is focused component evidence, not a comprehensive
session-weighted benchmark, release-profile claim, or end-to-end build speedup.

Hosted `justrust check`, all `remote_sync::tests` (14 passed, three intentionally
ignored), and `justrust clippy --all-targets -- -D warnings` passed. The ignored
real-manifest diagnostic also checks exact oracle equality before timing each
case. Exact values and run IDs are in
`bench/results/remote-sync-linear-diff-2026-10-30.json`.
