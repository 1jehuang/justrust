# Resident parallel manifest syntax experiment (v81)

## Status and hypothesis

This is an implemented, **default-off** candidate, not a measured speedup or a
promotion. The motivating v80 profile attributed 16.723 to 19.843 ms of eight
actual calls to package loading, compared with 3.853 to 5.293 ms for summary
preparation and 2.876 to 3.974 ms for version resolution. Those observations
identify an experiment, not its outcome. The previous typed JSON projection
experiment is not reused.

The candidate overlaps only independent manifest file reads, UTF-8 conversion,
and native `TomlManifest` deserialization. Semantic work is deliberately not
parallel. No dependency or lock-file changes are needed.

## Mode and eligibility

`RA_JIT_PARALLEL_MANIFEST_PARSE=1` is the sole enabling value. The first invocation
of the fetched-package mode check freezes the value in a process `OnceLock`.
Unset, empty, `0`, `true`, or any other value is off. Tests call an explicit
private boolean helper rather than modifying a cached process environment.

The fetched batch must contain at least eight entries, all original source texts
must be registry/sparse registry sources, and every effective source after source
replacement must be a remote registry. Git checkouts, vendored directory sources,
local registries, path packages, unknown sources, and mixed batches retain the
original sequential path. `sources.fetch` still executes in its original place
before any manifest read. An eligible batch is checked with `symlink_metadata`:
every entry must be a regular file named `Cargo.toml`. Missing paths, directories,
symlinks, and special files decline the batch without speculative file reads.

The current nonblocking-open implementation is intentionally limited to Linux
x86_64, the ABI for its explicit `O_NONBLOCK | O_NOFOLLOW` flags. Other platforms
use the original path. The opened file's metadata is checked again before reading.
This avoids waiting for a FIFO writer during speculative open if a checked path
is replaced with a FIFO. A last-component symlink introduced after preflight is
also declined. Failed opens or handle checks produce a per-entry fallback to the
original reader, invoked only when that entry is reached in sequential order.
The fallback retains the original behavior, including any blocking behavior of an
actual special file reached by the original reader. This is not a filesystem
snapshot, a defense against arbitrary hostile filesystem mutation, or a promise
that concurrent edits produce the same read interleaving as sequential reading.

## Ownership and ordering

The original public `read_manifest` signature and its extension/read/UTF-8/schema
error ordering are retained. The original semantic body is moved unchanged to
`normalize_manifest`. Both the public reader and parsed-syntax completion call
that body. Its configuration access, workspace-root `RefCell`, inheritance,
normalization, cfg-dependent dependencies, target discovery, warnings, features,
and diagnostic context remain on the calling thread.

Each worker owns a path and an `Option<Result<TomlManifest>>`. A scoped-thread
return of the real native schema requires Rust to prove `Send` through the actual
spawn operation. There is no unsafe implementation, substituted schema, manual
identity assertion, shared `Context`, or assertion that configuration is thread
safe. Native compilation is a root-owned gate, not something the source-only
worker has run.

There are two to four scoped threads, bounded by available parallelism (clamped
to that range). Chunks and join results retain input order. Thread-creation
failure declines the speculative batch. Every invocation reads fresh bytes.
There is no syntax cache, persistent artifact, timestamp shortcut, invalidation
protocol, or cross-invocation reuse.

After preparsing, source/config conversion and complete semantic normalization
execute one manifest at a time, in the original vector order. Source/config errors
precede the same manifest's stored syntax error. Earlier semantic errors precede
later syntax errors, even when the latter were discovered first by a worker.
The original `has no package` error remains in the metadata layer. Schema errors
retain `ManifestDiagnostic` filename, source contents, and span. UTF-8 and read
errors retain their original path-bearing text and error-chain boundaries.

## Diagnostics and attribution

On the first actual parallel batch chosen in a process, the candidate attempts
one fixed stderr line:

```text
ra-jit: parallel-manifest-parse-v81 batch
```

Writes are best effort and errors are ignored. The marker is not a cache hit,
proof of completed parsing, proof of successful normalization, or proof of a
speedup. Tiny/ineligible batches do not emit it. It is not path-dependent.

Native spans use the existing `ra_jit::planning` target and existing subscriber
and threshold policy. `resolver sources.fetch` covers fetching in either mode.
On a selected candidate path, `resolver fetched manifest syntax` covers eligibility,
worker creation, file reads and syntax parsing, and `resolver fetched manifest
normalize` covers ordered source/config conversion and semantic completion.
These spans do not add a tracing dependency to blaze-manifest.

## Native differential fixtures and pending gates

Two narrowly scoped unit-test modules live below each crate's
`tests/parallel_manifest_syntax/mod.rs`. They are included only in native test
builds, not automatically discovered as standalone integration tests.

The manifest module contains the full frozen accepted v80 `read_manifest` body,
independent of the new split helpers. Six fixtures compare the original reader,
new sequential reader and parallel syntax completion using full native debug
structures, exported metadata, ordered warnings, workspace-root state,
configuration warnings, diagnostic details, rendered diagnostics and complete
error chains. They cover regular registry batches, cfg dependencies, real target
files, UTF-8/schema/syntax/semantic errors, virtual manifests, inheritance,
configuration, repeated semantic/syntax/target edits, extension/missing-file
errors, and preflight fallback for symlinks, directories and a writerless FIFO.

The metadata module contains six fixtures around the frozen original ordered
fetch tail, using real local registry-shaped files rather than a network mock.
They cover vector order, full native manifest fields, warnings/targets,
multiple malformed manifests in reversed order, an earlier semantic error versus
a later syntax error, virtual-package rejection, registry-config precedence over
UTF-8 failure, explicit off/threshold/source fallback and fresh workspace,
configuration and target-file edits. No fixture weakens an error into success.

At source freeze, only rustfmt and static source/patch checks have been run.
**No native build, test, probe, workload, install, lock update, or commit was run
by this worker.** Root owns native schema `Send` verification, these crate tests,
the existing 53/373 gates, source pinning, workload/error gates and controlled
before/after timing after both source and harness freezes. A source-only static
pass is not native acceptance, and there is no speed claim.

Frozen originals, candidate copies, complete owned patch, SHA256 inventory,
static checks and `RELEASE` are under
`<native-root>/parallel-manifest-syntax-v81`.
The worker's final `STOPEDITS` handoff marks the exact point at which root may
start those gates.
