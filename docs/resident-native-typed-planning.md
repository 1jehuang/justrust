# Resident native typed planning v83

Status: **REJECTED / NOT PROMOTED** on October 31, 2026. The candidate passed root's native correctness and protected runtime gates, but the descriptive pinned factor comparison was below the requested 10% floor. All six existing native Rust files were restored byte-for-byte to accepted `c8829bae0`, and only the new nested test module was removed after full archival. This is licensed Blaze-derived resident code, not the clean-room compiler. The implementation descriptions below concern the archived candidate, not active production behavior. No speedup or default promotion is claimed.

## Root observations and rejection

The evidence bundle is `bench/results/resident-native-typed-planning-20261031/`. Its standalone validator replays preserved text only, not native jobs.

- Native named gates passed: 22 manifest, 4 metadata, 53 codegen, and 373 resident tests. The metadata gate includes the four authored complete-state/program/error/source-erasure fixtures, not only a package-ID comparison.
- Eight actual typed activation/profile rows and 28 protected strict transition rows passed. Profile intervals are inclusive diagnostics from the ON candidate, not a matched baseline comparison or an attribution of savings. Summary preparation ranged from 1.968 to 3.344 ms. The separate motivating 3.85 to 5.29 ms interval is not a controlled before/after claim.
- Same candidate ELF, factor OFF/ON/ON/OFF, five samples per segment: all 54 calls, 20 measured samples, 24 fresh warmup/sample nonces, worker PID/start-tick/executable/socket checks, and all first touches were retained. Whole-generation and watcher behavior remained ON in both candidate arms, all three experimental flags OFF in the reference.
- OFF median 0.054038845002651215 s, ON median 0.05164486599096563 s, descriptive delta -0.002393979011685585 s, or -4.43%. This is below the requested 10% floor. Noise is unknown. Combined same-byte manifest and lock retouch only, not separate manifest/lock attribution, significance, a general speed claim, or a broad default-promotion gate.
- Five ABBA stop receipts and four later validated strict-namespace stop receipts returned status 0. Stores were retained. No user data deletion is part of this result.

The strict state retains its inherited `parallel manifest syntax` scope label verbatim as provenance. Actual source, executable identity, explicit factor controls and raw stderr identify the v83 typed experiment. This stale descriptive label is not evidence that the parser experiment was activated.

The private adapter derives from the immutable v81 harness. Sampling, edit recipes, original semantic oracles, PID checks, nonces, and cleanup stay unchanged. The first mock attempt failed because a wrong-version negative still replaced `v81` after the positive marker had become `v83`. The corrected adapter changes that test-only mutation to replace `v83` with `v80`. The assertion is not weakened. Both the initial failure and corrected 18-test mock log are preserved, with zero native jobs in either mock run.

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

The private immutable root and runtime stores remain locally retained. Public paths use role tokens rather than private absolute locations.

The handoff preserves all six complete accepted originals, eight complete candidate files, cumulative and incremental patches, SHA-256 manifests, static-check script/output, RELEASE and explicit STOPEDITS. Rejected live bytes were compared against all eight frozen candidates before any restoration writes. Each of the six restored production files passed exact original comparison and a clean HEAD diff. The new nested test was removed only after preservation. No Cargo manifest or lock file changes.

The public export uses explicit per-record field allowlists, records each private input's original SHA-256 and byte size, rechecks original inputs after export, and omits ambient environments, private absolute paths and binaries. Portable validation joins original/candidate patches, replays unchanged original semantic oracle definitions against raw stdout/stderr, recomputes raw timings and medians, validates fixture/inventory/worker identities, and checks coherent-resealed negative fixtures. Isolated empty-HOME/PATH validation denies outside reads/listings, writes, subprocesses, sockets and native library loading. This is offline evidence replay, not a native rerun or full registry/git fetch proof.
