# Native Cargo recursive input-validation cache

## Scope

Cargo already caches compiler/version probe outputs on disk, and keeps exact-path
mtime and checksum observations in `BuildRunner` for one invocation. This change
adds an experimental stronger native layer, enabled only by the exact opt-in
`JUSTRUST_CARGO_SUBTREE_CACHE=1`: a complete recursive directory walk publishes
the maximum mtime for every directory subtree it visited. Later overlapping
`rerun-if-changed` roots can use those observations instead of walking again.

Implementation lives in `cargo-util::paths::mtime_recursive_with_cache`, called
from Cargo's native fingerprint validation. It is not an interception wrapper,
a persistent freshness certificate, manifest/resolution replay, or compiler
query reuse. A depth stack accumulates directory maxima without per-file hash
lookups or allocations. Only directory observations are added to the cache.

## Activation and default behavior

The native gate is **default off**. Only the exact value `1` enables it. Unset,
`0`, empty, `true`, and every other value use the original recursive walker.
Cargo reads the setting through tracked `GlobalContext::get_env`, not an
untracked direct process-environment lookup. The client forwarding path does not
itself activate the setting. Neither this candidate binary nor its experimental
algorithm is globally installed or selected by this work.

A comprehensive same-workload before/after suite is required before making the
experimental algorithm a default. The focused fixture below is not that gate.

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
justrust test -p cargo --lib subtree_cache_gate
justrust test -p cargo-util --test recursive_mtime_cache
justrust clippy -p cargo-util --test recursive_mtime_cache -- -D warnings
justrust test -p cargo-util --test recursive_mtime_cache -- --ignored --nocapture
./build-jr.sh
```

The tests compare independently walked sibling subtree maxima, empty directories,
direct files, changes between invocations, missing/deleted inputs, symlink loops,
dangling symlinks, and symlink roots. The ignored paired diagnostic compares the
old exact-root algorithm and new subtree algorithm in the same test process.

`bench/cargo-subtree-cache.py --cargo <candidate>` exercises one actual Cargo
binary with `JUSTRUST_CARGO_SUBTREE_CACHE=0` and `=1` against the same generated
3,072-file project, rustc, and artifact directory. It also checks artifact reuse
with the flag unset. It alternates invocation order, measures single-root and
97-overlapping-root cases,
rejects any warm sample that recompiles, and requires actual build-script and
compiler reruns after an input edit, addition, and deletion. This controlled
local baseline invokes the Cargo executables directly, without changing the
installed toolchain or justrust defaults.

The exact-value gate unit test covers unset, empty, `0`, `true`, `yes`, `01`, and
trailing whitespace as disabled, with only `1` enabled. Final gate unit-test run:
`20261029-205444836-2453389`.

A full Cargo-library `clippy -- -D warnings` attempt
(`20261029-205451121-2454994`) failed on ten existing warnings outside this
change: cutoff environment reads, an existing fingerprint `HashSet`, and daemon
stderr prints. This is not a claim that the whole native Cargo tree is
warning-free. The narrower cargo-util warnings-denied check above passed.

## Final same-binary default-off/on evidence

`bench/results/2026-10-30-cargo-subtree-native-gated.json` records the same final
Cargo SHA-256 (`62eecc8e8a6a54e3ea992c7fdf86eb58f416f0e81ebcd1ea837f9427a64eab93`)
for both modes, with 32 samples per mode per case. No executable or compiler
change is mixed into this comparison.

| Watched inputs | Gate `0` | Gate `1` | Observed delta |
|---|---:|---:|---:|
| One directory root | 35.37 ms | 39.16 ms | **+10.7% regression** |
| Root plus 96 overlapping descendants | 44.99 ms | 31.11 ms | -30.8% |

The single-root regression is a concrete reason not to enable this by default.
The native opt-in is useful only as a focused experiment for overlapping watched
directory roots pending broader measurement and further optimization.

All 128 timed warm invocations preserved artifact reuse when switching `0`/`1`.
Additional unset-default invocations stayed fresh in both cases. With the gate
on, watched edit/add/delete acceptance checks passed, including verification of
the new generated constant. Final native release build run:
`20261029-205538887-2472182`. Normal Cargo-library Clippy passed with the inherited
warnings described above, run `20261029-205527188-2468576`.

Reproduce from the repository root:

```sh
python bench/cargo-subtree-cache.py \
  --cargo toolchain/cargo/target/release/cargo \
  --rustc "$HOME/.justrust/toolchains/nightly-2026-10-08/bin/rustc" \
  --out "$JCODE_SCRATCH_DIR/cargo-subtree-gated.json" -n 32
```

## Historical two-binary diagnostic, October 30, 2026

The initial two-binary diagnostic predates the required default-off gate. It is
retained for provenance, not final activation or component-attribution evidence.

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
