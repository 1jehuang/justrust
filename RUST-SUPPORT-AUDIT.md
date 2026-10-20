# Full Rust support audit

Date: 2026-10-20. Inspected source revision: `910ca024c09c5d9ef5a53c8f93829bd1c844f74a`.
Scope: native clean-room compiler, not the installed stable justrust bootstrap client.
This is an assessment, not a full conformance certification or an implementation change.

Follow-up: `RUST-SUPPORT-FOLLOWUP.md` records subsequent fixes, their acceptance
receipts, and still-open compatibility failures. The findings below remain the
historical baseline for the inspected revision.

## Bottom line

The compiler has substantial working slices, but is not close to an evidenced claim of arbitrary Rust project compatibility. The immediate bottleneck is not another isolated syntax feature: production inference, trait selection, associated-type normalization, ownership transport and the source library must work together. The new general semantic core exists but is not yet the production checker. Project configuration also has observable correctness bugs, not merely explicitly unsupported features.

The current 20-program run confirms only five native successes and 15 explicit rejections, with no wrong-output executable or timeout in that corpus run. That ratio is a result for this corpus, not a percentage of Rust implemented. Many individual features required by the other programs now exist in bounded forms, so the historical feature tags overstate some current gaps. Only unchanged source CHECK/BUILD/run results establish promotion.

## Confirmed bugs and immediate blockers

### 1. Ignored overflow policy changes program behavior

**Reproduced on the inspected revision.** Manifest:

```toml
[package]
name = "audit-overflow"
version = "0.1.0"
edition = "2024"

[profile.release]
overflow-checks = false
```

```rust
fn increment(x: u8) -> u8 { x + 1 }
fn main() { println!("{}", increment(255)); }
```

`jr-rust run --release` builds successfully, advertises an optimized release profile, then aborts with `attempt to add with overflow` (driver exit 134). Stock `rustc --edition=2024 -C opt-level=3 -C overflow-checks=no` prints `0` and exits 0.

Cause: `src/project/manifest.rs:440-446` parses `overflow_checks` and `opt_level`, but `src/project/driver.rs:328-334` applies only `debug_assertions` to default source options. The profile summary at lines 376-380 calls release optimized even though this path does not select an optimizer mode. This is a wrong-behavior bug on accepted input. Until implemented, unsupported behavioral settings must reject rather than be silently ignored.

### 2. Invalid edition accepted, edition semantics not selected

**Reproduced:** a package containing `edition = "2099"` and `fn main() {}` passes native project CHECK with exit 0. Stock `cargo metadata --no-deps` rejects the same manifest with exit 101 and lists supported editions.

Cause: edition is retained as a string in `src/project/manifest.rs`, but no production reader of `Package.edition` selects or validates language behavior. This also leaves a larger unverified gap for valid earlier-edition programs. The declared initial native contract is edition 2024, so other editions should be implemented and tested or explicitly rejected.

### 3. Self-hosting fails in dependency discovery

**Reproduced:** `justrust run --bin jr-rust -- check --manifest-path Cargo.toml --lib -j 1` builds the bootstrap executable, then the native compiler exits 101:

```text
unicode-ident-1.0.24/Cargo.toml: target-specific dependency tables are unsupported
```

This is a native project-discovery limitation, not a failure to bootstrap with stock Rust. It occurs before establishing whether the compiler's own Rust source can be checked. General self-hosting remains unproven.

### 4. Strict lint gate is red

**Reproduced:** `justrust clippy --all-targets -- -D warnings` exits 101; its library compilation reports 109 errors. Examples include unused/dead code, collapsible conditionals, and an unfulfilled `#[expect(dead_code)]` at `src/compiler/mod.rs:264`. This is maintenance/gate debt, not evidence that 109 Rust semantics are wrong.

### 5. Test workflow can report an incomplete success

**Source inspection, not a fresh runtime reproducer:** `src/project/driver.rs:445-464` warns that doctests are unsupported, then prints a successful zero-test result. Unsupported test execution is visible as a warning, but the enclosing successful workflow is not equivalent to running all requested tests. It needs an explicit incomplete/unsupported outcome or compatible doctest execution.

## Current unchanged-program failures

Fresh hosted run `20261020-025130436-3537503`, at the inspected source revision, checks and builds all 20 fixtures and executes successful builds:

| Programs | First observed rejection |
|---|---|
| 04 sieve, 10 string processing, 18 tokenizer/calculator | `invalid metadata: non-field selector in nominal recipe` |
| 06 matrix, 08 enum state machine, 09 Vec stack, 14 generic container, 16 sorting, 19 operator overloading | `nominal source constraints cannot progress from the exact available graph inputs` with unresolved constraints |
| 11 closures/iterators | `implicit associated constant lookup requires a matching retained caller trait bound` |
| 12 linked list | `recursive nominal field realization requires an explicit recursive layout protocol` |
| 13 trait objects | `trait object types are only admitted directly behind a reference` |
| 15 error handling | `slice elements containing references or slices, or nested slice carriers` |
| 17 word count, 20 inventory | unresolved/cyclic import `crate::HashMap` |

Programs 01 hello, 02 fizzbuzz, 03 collatz, 05 gcd/fib and 07 bank account pass. These first rejections supersede the older first-error matrix in `FULL-PROGRAMS.md` for this revision. They do not enumerate every blocker behind the first one.

The three `invalid metadata` failures are particularly important: ordinary stock-valid source is reaching an internal recipe/authority rejection. The test ratchet counts any explicit nonzero rejection as an expected `fail`; it does not establish that the rejection is correctly classified or user-actionable. Minimize those sources and fix the producer/consumer mismatch without weakening authority validation. A green corpus test therefore does not mean all 20 programs work, and it does not certify diagnostic quality.

## Highest-leverage missing integration

### Production semantic core adoption

`src/compiler/sema/` contains general unification, obligation solving and a body checker. `src/compiler/frontend/sema_lower.rs` lowers source bodies into its HIR. However, the calls to `WorldDb::build`, `sema_lower_body` and `sema::check::check_body` are currently in tests. The corpus shadow checker is under `#[cfg(test)]` and explicitly ignored (`src/compiler/sema/adapter.rs:571,635-707`). Landing this library did not replace production checking.

`SEMANTIC-CORE.md` describes the resulting failure pattern: separate local-literal, call-replay, method-hint and iterator inference paths disagree. Its recorded probes fail on commonplace combinations such as `iter().enumerate()`, `map(...).collect()`, inferred `Vec` indexed updates, and constructor types learned from later method calls.

Required next step: adopt the checker one body at a time through typed graph facts, preserve canonical source/ownership authority, normalize trait projections and derive closure signatures from obligations, then lower the resulting checked body into the existing CFG. Do not bypass move/loan/forged-fact checks or publish a second competing semantic authority. Delete superseded special-case inference only after equivalent positive and negative acceptance tests pass.

### Reusable source library and iterator machinery

The native prelude contains real source implementations of bounded Vec, String, Box, Option/Result, formatting and related primitives. These are meaningful progress, not complete upstream core/alloc/std compatibility. `src/compiler/source_prelude.rs` embeds a selected source set; it does not activate a complete Iterator adapter library or a collections module.

The retained corpus identifies adapter inference/dispatch, nested slice/reference shapes, opaque return capture transport, owned `dyn` unsizing and collections as important integration gaps. A method or trait existing in isolation is not evidence that `Vec -> iter -> map -> collect` works. Prioritize unchanged pipelines and their invalid-input controls over increasing isolated feature counts.

## What remains for full Rust support

All G0-G12 completion gates in `IMPLEMENTATION.md` remain open or partial. Some unchecked bullets include work now implemented in narrower forms; they are not reliable evidence that a feature is wholly absent.

| Area | Missing completion evidence / remaining boundary |
|---|---|
| Language contract and diagnostics | Complete version/edition matrix, requirement-to-case ledger, consistent invalid vs unsupported vs resource/infrastructure errors, original semantic spans and expansion provenance. |
| Source and macros | Complete namespaces/imports/block items, edition-sensitive parsing and resolution, general hygienic declarative expansion, derives/attributes and procedural macros. Existing bounded macro support does not close this category. |
| Types and inference | General constraint propagation across uses, coercion sites, closure/callable inference, recursive associated projections, generic associated types, higher-ranked bounds and const generics. The parser explicitly rejects async functions and const generic parameters. |
| Ownership and lifetimes | General nominal lifetime arguments/reference-containing aggregates, loan interactions across arbitrary loops/captures, temporary scopes, region constraints, variance/outlives rules, complete drop checking and aggregate/generic drop glue. |
| Traits and suspended execution | Complete coherence/orphan/auto-trait behavior, broad trait object/unsizing support, general closures and callable traits, async/await/Future state machines and Pin-related behavior. Some bounded forms already work. |
| Constants and statics | General const evaluation, const-dependent types/layouts, promotions, const generics and full static/thread-local semantics. Scalar and bounded source constants are not full CTFE. |
| Runtime and ABI | General target Rust layout, repr variants, niches, DST metadata, pointer validity/provenance, ABI-compatible aggregates/callbacks, complete allocation/drop, unwinding, threads/TLS/atomics, debug information and target support. Direct ELF emission and private carriers are not a public layout contract. |
| Libraries | Independently compile and run an inventoried core, then alloc and target std; cover collections, I/O, synchronization, formatting, iterators and error handling with unchanged clients and library tests. |
| Project workflow | Target-conditioned dependency resolution, resolver/feature semantics, build scripts, proc macros, native build inputs, registry/git/patch support, crate artifacts and reuse, doctests and full harness behavior. Several of these are explicit rejection paths in `src/project/manifest.rs`. |
| Graph and persistence | Remove recorded mutable-fact debt and coarse joins, use per-item completeness facts, stable cross-edit identity, sound persistent invalidation, crash/cancellation behavior and sandbox/authorization policy. |
| Code generation/performance | Complete lowering beyond the scalar SSA subset, real ABI integration, reachability-driven emission, measured incremental/parallel performance and runtime/size comparisons. Optimizer success on a scalar corpus is not full backend support. |

## Graph status: use current tests, not stale numbers

`GRAPH-INTEGRITY.md` still lists five coarse nodes and a 1,208-function ceiling. Current `tests/graph_integrity.rs:638-668` instead records:

- Four coarse nodes: `parse`, `resolve`, `signatures`, `mono_plan`.
- Independent-leaf closure growth ceiling 1.02.
- CHECK work-growth ceiling 1.08; optimizer BUILD ceiling 1.81.
- Required generated program size 2,000 functions.
- No recorded whole-compilation failing-size sentinel. The comment says the larger generated main hits a per-function ownership operation cap, not the previous global work cap.

The graph gates passed in this audit, including the 2,000-function case. The ratios above are encoded ceilings, not a fresh speedup measurement. Mutable-state debt still exists in the checked-in baselines, including frontend/IR/trait state and token/source machinery. Do not repeat the obsolete claim that all programs over 1,208 functions fail, and do not interpret passing ratchets as proof of a fully graph-native architecture.

## Documentation problems

- `COMPILER.md` still calls floats, i128, labels, for loops, grouped imports and other delivered slices unsupported.
- `FULL-PROGRAMS.md` has a 5/20 headline but a later old paragraph says 3/20.
- `IMPLEMENTATION.md` has both later project support and an older statement that no manifest/workspace/feature model exists; its optimizer section says nothing is implemented immediately before listing implemented components.
- Several evidence entries apply to old or split parent trees. A passing historical corpus must not be presented as current HEAD validation.

Maintain one current compatibility matrix generated or checked against acceptance tests. Keep historical receipts explicitly historical.

## Recommended order

1. Fix or fail closed on ignored behavioral configuration, starting with overflow policy and edition validation. Add stock-differential project-level regression tests.
2. Restore warning-free lint and record current graph/corpus results without relaxing ratchets.
3. Integrate the general semantic checker with per-item graph dependencies and existing authority/ownership validation. Target the next unchanged iterator/closure programs, not handwritten substitutes.
4. Complete iterator adapters and the ownership/layout/opaque-return prerequisites exposed by those programs; then collections and richer source libraries.
5. Unblock unchanged dependency manifests, build scripts and proc macros. Keep incomplete test workflows visibly incomplete.
6. Drive core/alloc/std and independent real-crate bring-up. Feed every failure into a minimized, classified regression.
7. Broaden lifetimes, Drop/unwind, async, ABI and targets with corresponding acceptance evidence; measure performance on compatible workloads only.

There is no defensible percentage-complete or completion date from the current evidence. Twenty green example programs would still be a milestone, not full Rust compatibility.

## Validation receipts

- Hosted `justrust check`: passed, run `20261020-024646169-3522632`.
- Hosted strict clippy: failed, run `20261020-024714830-3523727`.
- Native self-host probe: failed at target-specific dependency tables, run `20261020-024746019-3524835`. The stable client executes `run` locally; this was observed, not a forced local override.
- Hosted graph/full-program/project integration suite: 25 tests passed across three binaries, run `20261020-024659144-3522631`. This includes 14 graph-integrity tests, two corpus tests and nine project tests. The corpus test expects 15 programs to reject.
- Hosted explicit corpus capture: five native programs pass and 15 reject, run `20261020-025130436-3537503`.
- Exported run summaries and standalone bug reproducers are retained under `data/runs/rust-support-audit-20261020/`.
- Direct native project reproducers used the binary built by the self-host probe. SHA-256: `5edbf31e23cad6ed827e2be84408220d6668290ec22b8522f26ebf53caddbdba`.
- External runtime oracle: `rustc 1.98.1 (48a229cea 2026-09-01)`.
- No complete all-target test suite, pinned full standard-library build, fuzz campaign or performance comparison was performed. No speed claim is made.
