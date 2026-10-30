# Resident typed programs-only planning candidate

Status: REJECTED / NOT PROMOTED. Native v8 correctness gates passed, but repeated total performance and matched planning observations did not establish reliable benefit. The two production files were restored byte-for-byte to HEAD, and the new live test was removed only after exact archival. This is licensed Blaze-derived experimental resident code, not the clean-room compiler. No speedup or default promotion is claimed. The following implementation description concerns the archived candidate only.

## Exact scope and motivation

Coordinator-reported v4 native trace client planning elapsed was 18.541 ms. That is a motivating observation, not a measurement of this change's savings. Previously `cargo/metadata.rs::programs` requested `blaze_metadata::programs(...).1`, discarding the returned metadata JSON. The API first exported every reached package and the complete resolve graph into a JSON string, parsed that string into a `serde_json::Value`, then selected a small vector of programs.

The new `blaze_metadata::programs_only` keeps the resolver and metadata graph, but selects programs from native manifests and graph feature vectors instead. Only that resident caller uses it. This is bounded typed dataflow, not caching, manual invalidation, approximate resolution, or removal of semantic work. The resolver still constructs serialized package representations and converts individual packages to values for its existing summary reader. This candidate removes only the final whole-metadata JSON string and its parse/Value tree from the programs-only caller. It does not claim a completely serialization-free resolver.

Owned files:

- `toolchain/resident/crates/blaze-metadata/src/lib.rs`
- `toolchain/resident/crates/ra-jit/src/cargo/metadata.rs`
- `toolchain/resident/crates/blaze-metadata/tests/resident_typed_planning.rs` (new)
- `docs/resident-typed-planning.md` (new)

## External behavior contract

The old public `programs` API and its JSON-based target selector remain unchanged as the differential oracle. Its `made`/`described` path still exports through the same serializer, with the same package vector and resolve object. Only the existing graph construction and package-order computation were extracted into shared helpers, `described_graph` and `described_order`. There are no JSON formatting, field order, or serialization configuration changes. The old returned JSON must remain byte-identical, which root must also check against its baseline binary or receipt.

Both paths use the same `resolving` entry point, feature resolver, feature-unification checks, selected roots and graph reachability. Both iterate reached packages sorted by the same native `PackageId` ordering, then targets in the manifest's existing order. The typed selector independently mirrors the old serialized target-kind classification, even for unusual library crate-type strings. It retains linked-library output ordering (`cdylib`, `staticlib`, `dylib`), named-target behavior, required-feature order and message bytes, dependency-qualified required-feature treatment, default test/example selection, and integration-test binaries appended in their previous order. Paths are the same manifest target paths used by `Target::serialized`, without normalization changes.

Targets with no source path are omitted just as the metadata exporter omits them. Harness selection deliberately uses the same workspace-wide unharnessed root-path set as the old API, not a tempting target-local reinterpretation. Error propagation at the resident caller is unchanged. Existing command refusal formatting and status 101 handling remain outside this change.

## Source-only differential fixtures

The repaired integration test file contains five tests, all matched by `typed_programs`:

- Target/feature/package matrix: default members, explicit package selection, workspace/exclusion selection, default/no-default/all/explicit features, lib/bin/example/test selection, missing targets, required-feature errors, linked-library order, proc-macro library, untested bins, integration binaries, harness-false library/tests/examples, library examples and benches excluded from program output.
- Resolver configuration/platform/errors: selected/workspace/package feature unification, the multi-package package-unification refusal and single-package success, platform selection, dev-unit selection, asking from a member directory, exact missing-feature diagnostic text, unknown package/feature, and asking without a build.
- Distinct legal linked-library revisions: `rlib` + `cdylib` + `staticlib` with exact linked-output order, then `rlib` + `dylib` with default/build/test assertions. The original illegal `dylib` + `cdylib` combination remains a deliberate negative fixture comparing full error-chain bytes and the exact manifest diagnostic.
- Typed-first cold resolution followed by the JSON oracle, manifest/target/harness edits without manual invalidation, and malformed-manifest error equality.
- Resolver 1/2/3 comparisons and old JSON byte equality before/after typed calls and against the normal metadata API. This is an in-candidate byte-stability check, not a substitute for root's old-binary versus candidate JSON comparison.

Complete ordered `Vec<Program>` values or full anyhow error chains are compared. The test helper pairs errors with the resident's existing status-101 convention, but does not launch the CLI and cannot independently verify actual subprocess exit status or rendered diagnostics. Root's native CLI gates remain necessary. The legacy JSON selector and typed selector are independent, sharing only graph/ordering preparation and the existing target-picking policy.

Intended root native test invocation from the resident workspace: `justrust test -p blaze-metadata --test resident_typed_planning -- typed_programs`. Also run the existing metadata/resolver tests and actual resident CLI semantic gates. No test or native workload was run by this lane. Only scoped rustfmt, formatting checks and whitespace diff checks were performed.

## Native v7 fixture rejection and v8 repair

Root reported v7 `check --tests` PASS, but all four original native integration tests FAILED before useful success assertions: the shared fixture illegally combined `dylib` and `cdylib`, which the manifest reader correctly rejected. The original root log is retained at `candidate-unit-v7/typed_programs.log`; no v7 performance samples were admitted. The shared fixture now uses legal `rlib` + `cdylib` + `staticlib`. A fifth differential test covers a separate valid `rlib` + `dylib` revision, preserves `Dylib` assertions, and retains the rejected combination as an exact-error negative. No production source changed in this repair. This lane performed only source inspection, scoped rustfmt and whitespace checks. Root must run fresh v8 gates before admitting performance.

## Root-reported v8 evidence, October 30, 2026

At 15:46 UTC, root reported native v8 gates PASS: all five typed integration tests, 49 existing metadata/manifest tests, resident graph/listed/telemetry tests, and RSA04 runtime/failure/recovery gates. Root also reported six actual old-v4/new-v8 native metadata stdout-byte and exit-status cases PASS using the correct `jr-engine cargo metadata` dispatch. An initial attempt omitted the `cargo` dispatch and failed; root retained that failed attempt separately rather than treating it as parity evidence. These reports concern those exact exercised cases, not every possible manifest, flag or diagnostic. Root owns the retained logs and detailed case inventory.

Root reported RSA04 engine elapsed 0.102257965 s versus matched reference 0.118546415 s. This single prospective comparison is descriptive only, not a robust speedup or default-promotion result. Repeat RSA05 and the matched phase observations subsequently failed to establish a reliable total benefit, as detailed below. Native build/test receipts and source hashes are retained with the candidate. No native workload was run by this lane.

## Rejection decision and portable retention

RSA01 baseline engine/reference medians were 0.10961705099907704 / 0.1109231570007978 seconds. RSA04 candidate medians were 0.10225796500162687 / 0.11854641500394791 seconds. RSA05 repeat medians were 0.1134313939983258 / 0.1157766399992397 seconds, with all runtime/failure/recovery gates passing. Against RSA01, repeat engine elapsed was **3.4797% slower raw**, and reference-normalized change was **-0.8583%**, below the 10% comparison floor. RSA04 remains preserved rather than selected as proof of improvement. This is scoped native diagnostic evidence, not the comprehensive session-weighted benchmark.

Root then captured both warmups and ten alternating actual edited observations from warmed old/new native engines. All 12 traces and raw outputs are retained. Edited-observation median client planning was 21160 / 20599 microseconds, client metadata 529 / 523 microseconds, and client command 210462 / 105761 microseconds (old/new). Client-command intervals were highly variable. These nested elapsed intervals do not establish causal savings and must not be added as CPU costs. The small planning difference did not demonstrate reliable total benefit.

Full rejected candidate source, exact baseline backups, reproducible candidate patch, original failed v7 fixture receipts, repaired five-test and 49-test native receipts, nine dedicated slim recorder exports bound to raw runs, exact source/native tool hashes, RSA01/RSA04/RSA05 summaries and raw outputs, original failed metadata dispatch attempt, six corrected exact status/stdout/stderr parity cases, and all 12 matched trace observations are retained under `bench/results/resident-typed-planning-20261030/`. The corrected metadata parity script explicitly uses `jr-engine cargo metadata`. There are no native binaries or caches in the bundle.

The stdlib-only portable validator audits bundle-local integrity and semantic consistency without original scratch paths, native tools or repository imports. Twenty-three positive/mutation tests exercise moved-bundle validation and adversarial rehashed contradictions. This is an evidence audit, not a replacement for native semantic testing or a proof of causation. Commands and limitations are in the bundle README.

Only `toolchain/resident/crates/blaze-metadata/src/lib.rs` and `toolchain/resident/crates/ra-jit/src/cargo/metadata.rs` were restored from exact HEAD backup snapshots captured at `23a1490cb84fa0aaca872dcf6a430f35e19a3863`; `cmp` and `git diff --exit-code` verified equality. The live uncommitted test was deleted only after its archived bytes were checked identical. Other workers’ source changes and unrelated FINDINGS/global run records were not touched. Root authorized an evidence-and-documentation-only commit after the portable audit, not candidate promotion.
