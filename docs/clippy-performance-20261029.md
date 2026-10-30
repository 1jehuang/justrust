# Clippy latency investigation, 2026-10-29

## Two separate costs

The recorded 7.71 s Clippy run `20261029-195010450-1694918` spent
7.35 s in one local crate, with 61 dependencies fresh. This is not evidence
that dependencies were missing from cache. A later unchanged invocation
`20261029-200246333-1835009` spent only 0.03 s inside Cargo, but 1.70 s
client-visible, including 1.06 s of sync. Another unchanged invocation
`20261029-200155409-1829481` spent 4.77 s syncing and 5.3 s overall.
These runs have different source/cache/host-load states, not a controlled
before/after compiler comparison.

## Source synchronization defect

Ignored lockfile discovery previously used `git ls-files -o -i` with
`Cargo.lock` and `**/Cargo.lock` pathspecs. Git recursively traversed ignored
build directories before Rust discarded every result under `target`, `.git`
or `.justrust`. On the active Jcode checkout this took seconds for a scan
that found no changed source.

The fix requests collapsed ignored directories with `--directory`, then
walks only eligible ignored directories without following symlinks. The same
build-directory exclusions now prune traversal, rather than merely filtering
its results. Only files named `Cargo.lock` are added to the source manifest.
Untracked source discovery and the authoritative full scan before each build
are unchanged. Nothing relies on watcher completeness or skips Cargo's
fingerprint checks. An oracle-based test compares lockfiles against the old
Git query, including nested ignored directories, build-tree exclusions,
unrelated ignored files and deletion.

Read-only diagnostic on the same local Jcode checkout, five scans per build:

| Scanner | Individual scan seconds | Files / oversized |
| --- | --- | --- |
| Before | 2.239, 1.890, 2.087, 2.176, 1.953 | 2655 / 7 |
| After | 0.036, 0.038, 0.035, 0.036, 0.035 | 2655 / 7 |

The before diagnostic used the existing compiled test binary. The after
run is `20261029-201053352-1916653`.
The tree was not frozen between diagnostics, so these are focused scan
observations, not a comprehensive or Clippy compiler speedup claim.
Local execution is necessary because this diagnostic reads local source and
local ignored build outputs. It does not benchmark the remote filesystem.

## Remaining frontend work

A forced unchanged-source lint rerun, `20261029-200459209-1858316`, measured
1.84 s compiler time, including 0.593 s expansion, 0.495 s crate lints,
0.088 s type checking and 0.033 s borrow checking. A real edit to
`src/remote_sync.rs`, `20261029-200741491-1880584`, measured 2.25 s compiler
time, including 0.595 s expansion, 0.181 s resolution, 0.171 s type checking,
0.056 s borrow checking and 0.366 s crate lints. Phase profiling itself can
add overhead. Neither run establishes a reduction from the original 7.35 s.

The driver retains `-C incremental`. Clippy's combined late pass is a
crate-wide walk outside rustc's incremental module-lint queries. Simply
registering it as a module-local pass would change diagnostics: `Methods`
initializes configured unwrap types in `check_crate`, and `unused_async`
collects cross-module function/value uses until `check_crate_post`.
A correct future change must separate module-local and crate-stateful
passes, track shared initialization and early-pass data dependencies, and
compare the complete UI diagnostics plus cross-module incremental edit
sequences. No speculative compiler switch or lint suppression was shipped.

## Measurement scope

The pinned `desktop-noop-test --remote -n 3` suite is the focused
client-overhead comparison. `bench/suite.py compare` reports **0.92 s to
0.83 s, -10%, within noise**. The snapshot does not contain the active
checkout's accumulated build outputs, so this does not establish an
end-to-end speedup for the pathological tree. Both result JSON files are
committed. After installing, cached Clippy run
`20261029-201238735-1925361` took 1.57 s client-visible, 0.04 s inside Cargo,
0 compiled units and 62 fresh units. The preceding first invocation after
daemon replacement took 14.65 s, including 13.00 s of initial rsync for this
68k-file checkout. Warm scans do not remove cold mirror initialization.

Hosted `justrust check`, `justrust test` (831 tests passed, six ignored),
and `justrust clippy --all-targets -- -D warnings` passed. The test run is
`20261029-201102931-1917870`; the all-target lint run is
`20261029-201114713-1917869`.

The focused suite is not a substitute for the comprehensive
session-weighted gate. The hosted `bench/suite.py run --synth` path rejects
execution and directs callers to `bench/versus.py --remote`. An attempted
local comprehensive baseline encountered an existing benchmark lock and
was interrupted by a server reload. No comprehensive speedup is claimed.
