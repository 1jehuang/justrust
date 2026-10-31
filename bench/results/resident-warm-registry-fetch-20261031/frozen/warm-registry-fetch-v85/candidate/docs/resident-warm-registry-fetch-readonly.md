# Warm registry fetch read-only v85

Date: 2026-10-31. Licensed Blaze-derived source. Source-only candidate, uncompiled.

## Revised authorized contract

The v84 direct `is_unpacked` preflight was rejected because that function removes
invalid package directories. Its immutable no-change evidence and historical
`resident-warm-registry-fetch.md` are preserved. The coordinator subsequently
explicitly authorized an independent read-only sufficient proof under the same
package cache lock. This document describes only that distinct v85 candidate.

`Registry::fetch` reads `RA_JIT_WARM_REGISTRY_FETCH` exactly once on first use.
Only the exact native environment value `1` opts in. Missing, non-Unicode,
whitespace, alternative spellings and all other values remain OFF. It acquires
`PackageCacheLock` before any package observation, exactly as before.

The opt-in probe handles at most 4096 packages and reads at most 4097 marker bytes
per package. Empty and larger batches fall through. For each package in input
order it computes the original `package_dir`, opens `.cargo-ok` read-only, and
uses the exact original serde shape `LockMetadata { v: u32 }` and predicate
`serde_json::from_str(...).is_ok_and(|it| it.v == 1)`. Only complete UTF-8 markers
of at most 4096 bytes proving v1 for every package return ordered
`(path, From::Unpacked)` entries. Duplicate input entries remain duplicated.
Unknown fields have the same serde behavior as the original predicate. Oversized
otherwise-valid markers simply use the original path, not a new error.

No speculative call to the mutating `is_unpacked` occurs. Every missing, invalid,
non-UTF-8, oversized or unreadable marker causes the entire original scheduler to
run under the still-held lock. Probe results and errors are neither cached nor
replayed. Invalid markers and native file bytes are not removed or rewritten by
the probe. The fallback scheduler body is bytewise the original after removing
the inserted opt-in branch. Public `fetch_one`, `is_unpacked`, checksum, archive,
cache, unpack and network code remain bytewise unchanged. Both the original and
candidate already skip checksum work when a valid unpacked marker is present.

As in the original implementation, the package-cache lock coordinates compliant
writers. No stronger snapshot or atomicity guarantee against unrelated filesystem
mutation is claimed. The probe adds bounded reads but no new trust criterion,
download, deletion, content validation exemption, or filesystem proof cache.
Bounded byte count does not promise a time limit for filesystem I/O.

## Marker and instrumentation

Only successful actual nonempty all-warm fetches reach a process-wide `Once`
that attempts to write exactly `ra-jit: warm-registry-fetch-v85 batch` plus one
newline to stderr. Write errors are ignored. Rejected and empty batches do not
activate it. It is an activation observation, not completion or latency proof.
This crate has no existing tracing dependency, so none was added and no owning
span was invented. Root owns external profiling and scheduler-versus-probe gates.

## Test sources and evidence

The nested `warm_registry_readonly_v85_tests` module contains a separately frozen
original scheduler and `fetch_one` oracle, verified against accepted original
source with only the oracle method-call rename and whitespace normalization.
They do not call the candidate scheduler. Unmodified downstream archive and
checksum routines are shared. Real filesystem fixtures compare normalized ordered
paths or full error chains and complete directory/file-byte snapshots for both
OFF and ON across empty, single and multiple packages, warm, missing marker,
corrupt marker, mixed, read-error, invalid checksum, corrupt archive, missing
archive, and ordered errors with cleanup of later packages. Additional source
tests cover malformed/non-UTF-8 markers retained by preflight, duplicate JSON
fields, native payload bytes, marker/batch bounds, removal and recovery without
memoization, duplicate/order preservation and the original warm checksum skip.
A lock-contention source test checks that an actual warm candidate waits until
the same package-cache lock is released. Its timeout is a correctness assertion,
not a speed measurement.

These tests were written, NOT run. No build, Rust test, native workload/probe,
installation, package lock acquisition, commit or timing was performed by this
worker. Only source inspection, assigned-file rustfmt, and static Python checks
were run. The coordinator owns compilation, actual subprocess flag/marker checks,
filesystem tests, full correctness gates and any greater-than-10% timing floor.
There is no speed claim, native activation result, or test-pass claim.

Full original/candidate files, patch, SHA-256 manifest, reproducible static checks,
RELEASE and STOPEDITS live under
`<EXPERIMENT>/warm-registry-fetch-v85`.
This bundle is separate from v83 and immutable v84 evidence.
