# Warm registry fetch v84: safe no-change rejection

Date: 2026-10-31. Scope: licensed Blaze-derived registry download implementation.

## Decision

Do not ship the proposed preflight. `download.rs` is unchanged byte for byte.
No environment switch, activation marker, tracing, dependency edit, or trust change
has been introduced. This is the explicitly permitted safe no-change outcome,
not an implemented optimization or a performance result.

## Scope issue

`Registry::fetch` acquires `PackageCacheLock` and starts scoped workers even for
already-unpacked packages. Each original `fetch_one` calls `is_unpacked` once.
However, `is_unpacked` is not an observational predicate: when `.cargo-ok` is
readable but invalid, it calls `remove_dir_all(dst)`. That call may partially
modify the directory and then fail. Its error carries `failed to remove` context,
which the original scheduler retains beneath `failed to download` context.

Calling that exact predicate in a speculative preflight, discarding its error,
and re-entering the original scheduler is not a proven semantics-preserving
transformation. In particular, a failed recursive removal can have removed the
invalid marker before failing elsewhere. The second predicate call then sees a
missing marker and returns `Ok(false)`, rather than the original removal error.
The original fetch path can consequently proceed to checksum/cache/network work
that the first original call would not have attempted. This is a source-level
failure-mode analysis, not an executed filesystem reproduction. The same cache
lock does not make a filesystem operation transactional or roll back partial
recursive removal.

Returning the probe error instead would violate the required original scheduler
fallback and can suppress other packages' original worker side effects. Caching
or replaying probe results would likewise change the requested original fallback.
Adding an independent read-only marker implementation is not the requested exact
existing-predicate preflight and would require a separately agreed correctness
contract, particularly around changes between observation and use.

Therefore this lane cannot honestly claim exact original errors and side effects
for the proposed direct preflight. The public `fetch_one`, OFF body, checksum
validation, ordering, original worker scheduling, and cache lock remain intact.

## Evidence and gates

Accepted originals and the final identical source candidate are frozen under:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/warm-registry-fetch-v84`.
The evidence bundle contains original/candidate files, patch, SHA-256 manifest,
static checks, RELEASE, and STOPEDITS. It is independent of v83 evidence.

Only source inspection and static Python checks were performed. No build, Rust
test, native probe, installation, lock acquisition, commit, or timing experiment
was run. Real filesystem differential cases (warm, mixed, corrupt, missing marker,
recovery, native bytes, empty/single/multiple batches, and concurrent locking)
were not executed or claimed. No speculative test module was added for a rejected
production change. The independent frozen original is available for any future
oracle comparison.

The coordinator owns compilation, correctness gates, and any greater-than-10%
timing floor. There is no speed claim and no activation or latency evidence.
