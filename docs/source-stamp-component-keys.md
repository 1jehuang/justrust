# Fresh source-stamp component sorting, 2026-10-30

## Scope and external contract

This changes only how the local daemon orders the freshly enumerated files
inside each source root before hashing. Every request still performs the full
existing traversal, reads fresh metadata, follows the same metadata/symlink
behavior, and applies exactly the existing directory rules. There is no watcher
trust, ignored-directory shortcut, source/result replay, routing-policy change,
Cargo flag change, new runtime diagnostic environment switch, or cache identity
change. Native Cargo still resolves and validates its own inputs.

The hash input remains each root's files in `Path::cmp` order, then the same
ancestor/CARGO_HOME configuration files. Each file contributes its original
encoded path bytes, length, mtime seconds, mtime nanoseconds and file ID, with
the same `H128::field` framing. Root order and configuration-file order are
unchanged.

## Attribution before choosing an algorithm

An ignored diagnostic observes the actual `source_stamp` traversal through a
generic callback. Production supplies an empty callback, without timers or
trace I/O. The optimized local command was:

```sh
JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 justrust test --release --lib \
  buildd::tests::source_stamp_profile_checkout -- --ignored --nocapture
```

Run `20261029-221243684-3305169` used local `jr-ac20de5e7921`, enumerated
171,114 files per invocation and produced the identical stamp
`06ea0b2b920a75ff7c1c79d17bd0e40a` in all five samples.

| Sample | Enumeration/metadata ms | Original Path sort ms | Hash ms | Remaining/configs ms | Total ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 389.413 | 231.718 | 53.252 | 11.916 | 686.303 |
| 1 | 340.978 | 275.391 | 53.097 | 11.051 | 680.520 |
| 2 | 340.077 | 274.537 | 53.057 | 11.497 | 679.172 |
| 3 | 325.690 | 203.012 | 33.148 | 6.464 | 568.317 |
| 4 | 309.832 | 272.512 | 52.930 | 11.304 | 646.582 |

Enumeration includes directory reading, file-type inspection, path allocation
and metadata reads. The initial remaining/configs interval also included entry
buffer destruction. The final diagnostic reports `drop-entries` separately.
These are optimized release-test measurements, not debug sorting observations.
The substantial repeated component-parsing sort cost justified a narrow
algorithm change without weakening enumeration.

## Reversible component keys

On Unix, all paths in one root's enumeration share the same root spelling.
Every suffix component comes from a real directory entry, so it is neither
`.` nor `..`, and it cannot contain NUL or `/`. The common root prefix may
contain repeated separators or `.`/`..`, which are identical across that root's
entries and cannot affect their relative order.

Temporarily replacing `/` bytes with NUL in the owned path buffers gives raw
byte keys with the same order as `Path::cmp` on these paths. A component end
must sort before any byte that extends the component, and NUL is below every
possible filename byte. Ordinary raw pathname sorting would be wrong, for
example for `a/b` versus `a.b`, and is not used.

The helper sorts the raw keys stably, restores every separator byte, then
hashes the original paths with their original metadata. It reuses the existing
owned path buffers, with no extra per-file key allocation. Nothing reads these
private temporary keys as filesystem paths. Non-Unix systems retain the
original `Path::cmp` sorter.

## Exact ordering and same-captured-metadata oracle

A normal unit test compares the complete sorted `(path, payload)` vectors with
stock `Path::cmp` across seven root spellings and 28,049 generated/special paths.
It covers deep prefixes, component-prefix punctuation, relative and absolute
roots, root `.`/`..` and repeated separators, byte values outside UTF-8, stable
payload association, and restoration of the original raw path bytes. The
existing real-file/path-dependency fixture also compares the complete old and
new stamp/count before exercising its edit and build-output checks.

An additional ignored optimized diagnostic freshly traverses the actual
checkout, clones each root's captured `(path, Metadata)` vector, sorts one copy
with stock `Path::cmp`, sorts the other with component keys, compares every raw
path, and asserts identical field-by-field hashes. Both sort timers include
their complete respective sorting work. The new timer includes key preparation
and restoration.

```sh
JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 justrust test --release --lib \
  source_stamp_ -- --ignored --nocapture --test-threads=1
```

Run `20261029-221643570-3356348` passed both ignored diagnostics. Each oracle
sample contained 171,114 files. All samples produced the same exact old/new
captured-metadata hash, `9a440584e238fc755d26963cbebd6c5c`.

| Sample | Original Path sort ms | Component-key sort, including conversion ms |
| --- | ---: | ---: |
| 0 | 359.413 | 121.724 |
| 1 | 345.259 | 120.711 |
| 2 | 406.731 | 150.115 |
| 3 | 471.456 | 218.516 |
| 4 | 489.162 | 179.873 |

This is an isolated component diagnostic on identical captured inputs, not a
whole-command or comprehensive-workload speedup claim.

The five complete new traversals in the same run had enumeration 437–567 ms,
sort 124–150 ms, hash 68–83 ms and total 665–806 ms. Other compiler activity
was heavier than in the earlier baseline. Thus the separate full-traversal
before/after totals are not evidence of an end-to-end improvement. No such
claim is made. The changed stamp values between build revisions are expected:
the actual source files changed, while equality within each captured oracle
pair is required and was verified.

## Validation and limits

Hosted check `20261029-221614603-3337306` and the generated exact-order oracle
test `20261029-221618206-3339762` passed before the final fixture/phase-label
additions. The final candidate passed hosted check
`20261029-221925372-3374277`, the complete test suite
`20261029-221928265-3374535` (839 passed, zero failed, twelve ignored), and
strict all-target Clippy `20261029-221940394-3374219`. `cargo fmt --check`
and scoped `git diff --check` also passed. Source is frozen after these gates.
The coordinator owns final installation, installed acceptance and run-summary
export.

The coordinator attempted a pinned `desktop-noop-test` baseline with the daemon
enabled, but it timed out after 60 seconds during a cold Desktop compile. No
result JSON or matched `bench/suite.py compare` exists. Its smaller snapshot
also did not represent this 171k-file checkout. This does not replace the
session-weighted comprehensive gate, which was not run for this change. No
default routing or general performance promotion follows from these results.
