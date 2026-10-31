# Resident native typed planning v83

Status: source-only candidate, default OFF, not promoted. This is licensed Blaze-derived resident code, not the clean-room compiler. No build, Rust test, native probe, install, benchmark or commit was run by this worker. Root owns compiler/runtime/strict and same-binary pinned-factor gates. Broad performance benefit is unproven.

## Distinct dataflow

The motivating measured summary-preparation interval is coordinator-reported 3.85 to 5.29 ms, not a measured saving. Rejected v8 removed only the final metadata projection. This candidate additionally removes both initial and newly picked packages' successful `serde_json::to_value` plus `summary::read` round trips from the opted-in caller. Borrowed views expose the already-exported fields, after normalization, ordering, formatted requirements and locked-source substitution. They do not reinterpret raw manifests or change serde implementations.

`RA_JIT_NATIVE_TYPED_PLANNING=1` is the sole opt-in, latched on first use with `OnceLock`. Unset, `0`, and other values remain OFF. Only ra-jit's programs caller chooses the new public endpoint. All existing public JSON APIs, `programs`, `summary::read`, and `described` retain their independent original implementation. Existing callers still select the original summary conversion. No format is sent to stock tools.

The new summary reader retains dependency sequence, source precedence, prerelease wildcard handling, renamed names, kind, optional/default features, feature order, platform reparse, package ID, feature-map construction, links, library-name normalization, proc macro flags, build-script presence, binary required-feature vectors, license and repository. Rust-version and publication time stay unset exactly as in the JSON oracle, rather than being newly imported from the manifest. Existing caller code installs the lock checksum at the same two sites for both paths. All resolution, lock merging/writing, fetching, manifests, config, feature resolution, build-script/native semantics and graph reachability remain.

The typed final selector consumes serialized package/target views and the complete native feature graph, not raw target interpretations. It preserves ID sort order, target order, unusual serialized kind strings, linked-library order, required-feature diagnostics, dependency-qualified features, test filtering, workspace-wide unharnessed paths, and appended integration-test binaries. It does not export or parse final whole-metadata JSON. No cache or invalidation scheme is added.

Non-UTF-8 exported paths are explicitly checked before actual typed conversion. Those exceptional inputs invoke the original package serializer and JSON reader to preserve the exact serialization error and chain. No success path intentionally serializes packages. This exceptional compatibility path is not counted as typed entry. Other serializer failure modes and equivalence remain runtime gate obligations.

On actual typed conversion, a process-local `Once` writes exactly `ra-jit: native-typed-planning-v83 entered` and newline to locked stderr. Write errors are ignored. The flag check itself emits nothing. No dynamic data is printed.

## Independent oracles and source checks

The new narrowly nested `blaze-metadata/tests/native_typed_planning/mod.rs` is wired as a private unit module so it can compare the old private reader and new conversion directly. It creates real temporary filesystem manifests, leaving existing fixtures unchanged. `same_read` explicitly compares package ID, full ordered dependencies (whose Eq is structural), full feature map, links, checksum, rust-version, publication time, and every `Read` field. It never uses Summary Eq/Hash as evidence. Tests cover renamed optional/path/registry/git declarations, pinned exported sources, cfg/dev/build dependencies, feature modes, required-feature errors, multiple target types, proc macros, legal separate dylib and cdylib cases, revisions, program vectors/order, program JSON bytes, original full metadata JSON bytes, and error chains. Registry/git package conversion is tested via exported source substitution, not a claim of full offline registry/git fetch integration coverage. Root must still exercise those full integration paths with its existing pinned native fixtures.

Source-erasure guards check the typed final selector has neither serde_json nor `described`, and that original selectors remain JSON based. Private static checks compare original `programs`, `summary::read`, `described`, serializer structs/implementations and existing fixture bytes against preserved accepted originals or HEAD. Rustfmt only parsed/formatted the exact owned files, with child traversal disabled. This is not Rust typechecking or test execution.

## Frozen handoff

Private root: `<native-root>/native-typed-planning-v83`.

The handoff preserves all six complete accepted originals, complete candidate files, cumulative and incremental patches, SHA-256 manifests, static-check script/output, RELEASE and explicit STOPEDITS. No Cargo manifest or lock file changes. Root must reject or repair any compiler/runtime mismatch before accepting. No speedup, repeatability, full integration coverage, or default promotion is claimed here.
