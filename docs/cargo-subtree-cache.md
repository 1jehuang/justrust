# Native Cargo recursive input-validation cache

## Scope

Cargo already caches compiler/version probe outputs on disk, and keeps exact-path
mtime and checksum observations in `BuildRunner` for one invocation. This change
adds a stronger native layer: a complete recursive directory walk also publishes
the maximum mtime for every directory subtree it visited. Later overlapping
`rerun-if-changed` roots can use those observations instead of walking again.

Implementation lives in `cargo-util::paths::mtime_recursive_with_cache`, called
from Cargo's native fingerprint validation. It is not an interception wrapper,
a persistent freshness certificate, manifest/resolution replay, or compiler
query reuse. A depth stack accumulates directory maxima without per-file hash
lookups or allocations. Only directory observations are added to the cache.

## Correctness contract

- The cache has the same lifetime as Cargo's existing `BuildRunner::mtime_cache`.
  Every new Cargo invocation reads the filesystem again.
- The complete walk succeeds before publishing any subtree observation.
- Any symlink or walk/metadata error falls back to the existing recursive walker,
  publishing no subtree entries. This retains its symlink target/link mtimes,
  ancestor-sensitive loop handling, and error behavior.
- Missing explicitly watched paths remain dirty. Directory maxima include the
  directories themselves, so nested additions and deletions remain visible.
- Checksum freshness and the persistent artifact fingerprint formats are unchanged.
- Ordinary exact-path cache hits now use borrowed lookups, avoiding a path
  allocation on every hit.

## Validation and reproduction

From `toolchain/cargo`:

```sh
justrust test -p cargo-util --test recursive_mtime_cache
justrust clippy -p cargo-util --test recursive_mtime_cache -- -D warnings
justrust test -p cargo-util --test recursive_mtime_cache -- --ignored --nocapture
./build-jr.sh
```

The tests compare independently walked sibling subtree maxima, empty directories,
direct files, changes between invocations, missing/deleted inputs, symlink loops,
dangling symlinks, and symlink roots. The ignored paired diagnostic compares the
old exact-root algorithm and new subtree algorithm in the same test process.

`bench/cargo-subtree-cache.py` exercises the actual before/after Cargo executables
against the same generated 3,072-file project, rustc, and artifact directory. It
alternates invocation order, measures single-root and 97-overlapping-root cases,
rejects any warm sample that recompiles, and requires actual build-script and
compiler reruns after an input edit, addition, and deletion. This controlled
local baseline invokes the Cargo executables directly, without changing the
installed toolchain or justrust defaults.

## Measured result, October 30, 2026

`bench/results/2026-10-30-cargo-subtree-native.json` retains executable hashes,
versions, and all 128 timed samples (32 per executable per case). Paired local
Cargo medians were:

| Watched inputs | Before | After | Observed delta |
|---|---:|---:|---:|
| One directory root | 63.91 ms | 56.00 ms | -12.4% |
| Root plus 96 overlapping descendants | 80.83 ms | 61.51 ms | -23.9% |

All warm calls reused artifacts across the two Cargo executables. Input editing,
nested addition, and nested deletion each reran both build script and compiler.
The edited input produced the expected new generated constant. These are paired
observations under concurrent machine activity, not isolated component
attribution or a whole-session performance claim.

Hosted justrust validation run IDs:

- Four utility tests: `20261029-204218392-2269938`.
- Clippy with warnings denied: `20261029-204230438-2272038`.
- Complete release Cargo build: `20261029-204237707-2273312`.

This focused fixture is not the session-weighted synthetic suite or evidence for
a general compile-time speedup. No installed-toolchain/default promotion is part
of this change. The comprehensive before/after suite remains a separate gate.
