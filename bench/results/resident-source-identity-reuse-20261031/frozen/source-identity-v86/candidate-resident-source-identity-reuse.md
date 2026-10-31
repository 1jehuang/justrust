# Resident source identity reuse, v86

## Scope and lineage

This source-only experiment modifies `toolchain/resident/crates/blaze-metadata/src/lib.rs` in the imported Blaze lineage (`indexable-inc/blaze`). The resident tree retains its MIT and Apache-2.0 license files and existing attribution. This is a justrust integration experiment, not an independent clean-room implementation or a claim of upstream authorship.

## Contract

`RA_JIT_SOURCE_ID_REUSE=1` is the only enabling value, sampled on first use by a process-local `OnceLock<bool>`. Missing, non-Unicode, empty, and other values stay OFF. OFF dispatches to the full frozen original fetch body. The accepted `manifest_source` remains unchanged, and a separate full original source resolver is retained in the fixture module.

ON allocates a fresh successful-resolution map inside each fetch, after the original fetch batch completes. Its key is the exact original formatted source string, without normalization. It inserts only successful `manifest_source` results. Hits clone the full manifest SourceId, including kind, URL, git reference and registry key. No result survives the invocation. Formatting PackageId sources, source replacement, checksum lookup, fetch batching, root zip order, manifest reads, package pushes and error exits remain in their original positions. There is no failure caching or manifest/package caching.

## Configuration audit

`blaze-manifest/src/source.rs::SourceId::crates_io` calls the public `Config::check_registry_index_not_set` and then parses the constant crates.io index and constructs an alternate registry SourceId. `check_registry_index_not_set` calls `get_string("registry.index")`, which can read configuration, produce warnings, reject a typed value, or reject an obsolete index setting. Consequently each cached crates.io occurrence repeats that exact public validation before returning the cached ID, at the original resolution position. Only its constant URL parse and ID construction are shared. Other registry/sparse/git arms have no configuration access and share successful pure resolution. Future changes to these constructors require reauditing this assumption.

The cache never changes Config's own lifetime/caching rules. Fixtures deliberately load new Config instances after filesystem configuration changes, and additionally exercise a previously populated reuse map against a newly invalid Config to verify hit validation remains live.

## Marker

Only an actual successful repeated-source hit, after crates.io validation succeeds, triggers the once-per-process fixed stderr line `ra-jit: source-identity-reuse-v86 batch`. Writing uses `write_all` with ignored I/O errors. Flag entry, first resolution, invalid sources, and failed hit validation do not trigger the marker. The marker is intentional additional ON stderr and is not an error/warning suppression mechanism.

## Fixtures and acceptance boundary

Unexecuted source fixtures cover repeated/mixed exact registry, sparse and git strings, distinct slash variants, branch/tag/rev/default and precedence, full SourceId equality, ordered config warnings, obsolete and invalidly typed registry.index errors, uncached invalid URLs and path rejection, and fresh calls after config edits. A real vendored-directory replacement fixture compares complete exported package observations, manifest paths, locked source strings, ID order, errors and configuration warnings against the original fetch. It also places a semantic manifest error at the first requested package. No new dependency or lockfile change is required.

The full original fetch body and manifest_source body are statically compared to the accepted HEAD. The ON fetch body is statically compared to that full original with only map construction and the individual resolution call substituted. These checks establish source structure, not Rust type correctness or runtime correctness. Fixture coverage does not substitute for native acceptance, stderr/marker subprocess checks, malformed filesystem ordering permutations, or performance measurements.

Only exact-file rustfmt and Python static checks are performed by this lane. No builds, tests, native probes, dependency resolution, installation, commits, or performance claims are made. Root owns native fixture execution, OFF bytewise observations, strict/pinned acceptance, profiling, and any measured decision. The source handoff is frozen under `<EXPERIMENT>/source-identity-v86` with accepted originals, candidate sources, patch, hashes, static checks and explicit RELEASE. No speedup is claimed.
