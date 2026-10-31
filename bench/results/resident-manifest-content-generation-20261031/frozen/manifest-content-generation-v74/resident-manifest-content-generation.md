# Resident manifest content generation, v74

## Status and scope

Source-only, default OFF experiment dated 2026-10-31. No build, test execution,
probe, lock acquisition, installation, or commit was performed by this worker.
Only `toolchain/resident/crates/ra-jit/src/cargo/whole.rs` and this document are
owned here. Root must validate the frozen candidate before any promotion.

The hypothesis is narrow: `whole::checkout` originally takes the maximum
`written_at` timestamp of workspace member manifests plus root `Cargo.lock`.
An identical-byte rewrite can change that discriminator and cause a cold
checkout unnecessarily. This experiment replaces only that discriminator when
`RA_JIT_MANIFEST_CONTENT_GENERATION=1` exactly. The decision is cached at the
first call to checkout generation in each client process. Other values, absence,
and non-Unicode values are OFF. Runtime environment changes after first use do
not change this decision.

OFF executes the original manifest iterator chained with the lock, followed by
`filter_map(written_at)`, `max()`, and `unwrap_or(0)`, without reading file bytes.
The call site no longer carries an intermediate `Option`, but its resulting
protocol `u128` is unchanged. Worker configuration, worker key, ABI/native
configuration guards, build scripts, `kept: false`, source collection, program
reload and fallback guards are not edited. Rejected v64 retention is not revived.

## Identity and protocol domains

The existing sha2 dependency hashes deterministic length-framed fields. Every
length is a little-endian u128. Domain/version, host OS/architecture, canonical
checkout root identity, file count, original native path label, file role,
presence state, canonical target path and full file bytes are framed separately.
There is no lossy path conversion. Native `OsStr::as_encoded_bytes` is scoped to
this host/toolchain, not a portable cross-platform cache format.

The file set is exactly the original workspace member manifests and root lock,
not dependencies or unrelated manifests. Entries are sorted by native path bytes
and role, with identical labels/roles deduplicated. Sorting deliberately does
not use `Path::Ord`, whose normalized components can alias distinct raw labels.
Canonical target paths do not replace original watched path labels: symlink or
`./` aliases remain distinct labels. A checkout-root change changes identity.

A known absent root lock is framed as `missing-lock`, distinct from a present
empty lock. Missing manifests, dangling symlinks, non-regular files,
canonicalization/read failures and observed read races return errors. The root
must canonicalize successfully. Conservative fallback is also used if the
missing lock's spelling is not the canonical root lock spelling.

Each file is read afresh twice with a canonical-path recheck, rather than trusting
mtime, size or a metadata memo. Same-path, same-size, same-mtime sequential
rewrites therefore change identity when bytes change. This is not an atomic
multi-file snapshot, and does not claim to solve concurrent edit races that the
original metadata/load path also lacks a transaction for.

The first 128 SHA-256 bits become a u128 with bit 127 forced to one, leaving a
127-bit probabilistic content discriminator. All original timestamps are
`Duration::as_nanos()`, bounded by u64 seconds plus subsecond nanoseconds and
therefore strictly below 2^94. Their bit 127 is always zero. Thus an original
fallback timestamp cannot equal any known content tag, including unusual future
filesystem timestamps. This is a structural domain separation, not an assumption
about today's epoch value. SHA truncation still has the usual finite-hash
collision risk and is not mathematical proof of byte equality.

On I/O uncertainty the helper returns to the original maximum timestamp
computation over the original file set. Unknown is never assigned a known
content tag or encoded as empty bytes/missing manifest. Transition from a known
content generation to fallback necessarily changes domains. Repeated unknown
states retain the original timestamp behavior and its limitations: an unchanged
maximum timestamp can compare equal to a previous unknown's fallback timestamp.
There is no deterministic fresh-on-every-error guarantee in this patch. If root
requires that stronger invariant, this candidate is blocked pending a protocol
cold/reject signal or an explicitly approved unique-generation mechanism. Do not
replace unknown with a constant content digest or silently claim guaranteed cold
loads for repeated unknown states.

## Marker semantics

At actual opt-in helper entry, once per process, an ignored-error stderr write
attempts exactly:

```text
ra-jit: manifest-content-generation-v1 entered
```

The marker precedes hashing and may accompany conservative fallback. It proves
entry only, not successful identity computation, retention, cache hits, same PID,
or ABI safety. OFF never enters this marker site.

## Source tests and root acceptance gates

Five helper test sources operate on real temporary manifests/locks and cover:

- Identical-byte rewrite/touch with deliberately changed timestamp and stable tag.
- Changed bytes then restored bytes for both files, including same-size mutation
  with mtime restored through `FileTimes` and checked explicitly.
- Set reordering and duplicate elimination, added/removed paths, root identity,
  raw `./` path aliases and independently framed delimiter-like content.
- Present/missing/empty lock separation and errors for missing manifest,
  directory-as-manifest and unavailable root.
- Unix non-UTF-8 native paths versus replacement-character names, symlink aliases,
  dangling manifests and dangling locks.

These tests are written but unexecuted in this source-only lane. No full checkout
API fixture was available in this file. Invocation parsing is private and the
normal metadata helper invokes the external cargo path, so constructing a
synthetic metadata/CLI fixture here would not replace the required real workflow.
Root should run the helper tests and existing actual resident/compiler suites.

Root must then run the actual whole-checkout workflow with explicit OFF/ON arms:

1. Warm a program and record worker PID, runtime output and marker count. Touch
   each member manifest and root lock without changing bytes. ON must preserve
   the generation and demonstrate same worker PID plus correct runtime, not
   merely report the entry marker. Compare OFF baseline behavior and timings.
2. Actually change a valid manifest, introduce an unused `E0308` source error,
   and require the diagnostic to appear. Repair it and require successful runtime
   recovery. Do not require or infer same-PID reuse across content changes.
3. Exercise the native build-script environment and file inputs through A/B/A,
   requiring each expected runtime value and existing configuration guard
   behavior. Include changed bytes restored to A, not only byte-preserving touch.
4. Exercise missing/present lock and I/O uncertainty as feasible. Check known
   content and fallback tags never alias. Record the repeated-unknown limitation
   rather than describing timestamp fallback as fresh-on-every-error.
5. Check process-first-use opt-in behavior with separate processes. Ensure OFF
   has zero entry markers and ON has at most one attempted marker per client
   process, including an ON error fallback. No claim about stderr delivery if
   the output stream itself fails.

A content identity alone does not establish full toolchain ABI safety. Existing
configuration/native/script/worker-key/fallback guards remain the contract.
No speedup or runtime correctness result is claimed by this source-only patch.

## Frozen handoff

Private root:
`<EXPERIMENT>/manifest-content-generation-v74`.
The handoff contains complete original/candidate source, this document, a unified
patch, SHA256SUMS, static source-check evidence, exact-file rustfmt parse/check
output, diff whitespace check and RELEASE. After RELEASE is written, this worker
stops edits. Root owns all subsequent build/test/probe and promotion decisions.
