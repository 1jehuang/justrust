# Parallel native declaration query candidate (v72)

Status: source-only, default OFF. Native typechecking, fixtures, full-suite correctness, timing, restoration and promotion belong to the coordinator. No performance claim or passing native-test claim is made here.

## Distinct hypothesis

The v69 final tracked graph reused green results but its `&dyn HirDatabase` provider traversed bodies sequentially. This candidate keeps the native final graph and restores the existing cold-lowering scheduler. It does not flatten traversal or revive the v70 listing scheduler.

`RA_JIT_PARALLEL_DECLARED_QUERY=1` enables the candidate. A process-local `OnceLock` captures the exact first-use value. Missing, empty, `0`, `true`, `01` and `1 ` remain OFF. The original `declared_in` implementation is retained byte-for-byte except for its name, `declared_in_original`, and the default path calls it.

## Actual API investigation

The installed dependency is Salsa 0.28.5. Its `salsa-macros/src/tracked_fn.rs`, around line 405, requires the first tracked-function parameter to be `&dyn Db`, with `Db` a Salsa database trait. A concrete `&RootDatabase` tracked provider would not satisfy that macro contract.

The local `#[salsa::db] trait DeclarationDatabase: HirDatabase` adds only `fn declaration_snapshot(&self) -> RootDatabase`. Its local implementation for the external `RootDatabase` returns `self.clone()`. A local trait may be implemented for that external type. Both trait and implementation use the normal Salsa database-view macro registration. There is no handwritten unsafe cast, custom downcast, global database pointer, or fake snapshot mechanism.

The vendored `ide-db/src/lib.rs` implements `RootDatabase::clone` by cloning storage, file and crate-map handles and preserving its nonce. Salsa `src/storage.rs` implements `Storage::clone` with a shared `StorageHandle` and a fresh `ZalsaLocal`. `StorageHandle::clone` shares the `Arc<Zalsa>` and cancellation coordination. The clone therefore accesses the same database revision but does not inherit the parent's active query stack. The candidate makes no input writes and drops its snapshot before returning.

## Dependency and ordering contract

`parallel_crate_declarations(&dyn DeclarationDatabase, base_db::Crate)` is a native `#[salsa::tracked(returns(ref))]` query returning the full existing `Declared` value. It reads crate name resolution and `module_declarations` on the parent database.

Each traversal wave processes ordinary bodies and then provided bodies in the original order. It calls the unchanged `lower(&snapshot, bodies)`, including that function's actual `Body::of` prewarm, threshold, scoped threads, interleaved scheduling, attached databases, probing state and cancellation behavior. It then reads `body_declarations(db, body, under_provided)` on the parent database for every returned body, in order. Those reads are mandatory even after child queries warmed bodies. No child read is presumed to become a parent dependency.

The merge uses the existing `Declared::extend` and preserves every field, ordering, nested items, provided-method distinction, cfg and macro expansion, statics, consts and traversal-state semantics. Signature and dependency changes are left to actual Salsa revalidation rather than classified by a handwritten cache key.

The fixed `parallel-declared-query-provider-entry pid=...` stderr marker is attempted once per process, only inside the real provider. Write errors are ignored. It is not a hit counter, completion claim or speed measurement. Test-only TLS counts actual provider entries on the requesting thread, independently of that marker.

## Fixtures supplied, not executed by this worker

Five real-RootDatabase fixtures cover:

1. Exact/default-OFF flag parsing without process environment mutation.
2. Cold provider count 1, repeated count 0, full `Declared` equality against both the original parallel traversal and an independent direct scope/body walker. Revisions cover executable-only edits, nested and macro items, provided methods, consts, statics, closures, cfg, signatures and restoration.
3. Dependency macro/signature edits and unrelated-crate green reuse.
4. The local trait's typed snapshot and shared native memo reuse between cloned and original databases.
5. A 96-body, two-wave graph, large enough to select the original parallel threshold on multicore hosts. After cold prewarm, a nested body's block gains a function and static. The final graph must recompute, contain both new declarations, equal both walkers, and drop the declarations after restoration. This explicitly exercises parent dependency recording after parallel child warmup. Single-core hosts retain the original scheduler's single-thread fallback.

Static source checks also require the unchanged OFF traversal body and unchanged `lower` function and restore the entire original file by removing only the new blocks and undoing the original function rename.

## Source-only validation and handoff

Exact-main `rustfmt --edition 2024 --config-path toolchain/resident/rustfmt.toml --config skip_children=true` completed with exit 0. The matching exact-main `rustfmt --check` completed with exit 0. No child module was formatted. These are syntax/style checks, not macro expansion, typechecking or behavioral validation. The source delta is additive-only relative to the accepted original. Scoped `git diff --check` completed successfully.

The full original, full candidate, document, patch, static-check script/results, formatting logs, SHA256 manifest and `RELEASE` are frozen under `<PRIVATE_EXPERIMENT_ROOT>/parallel-declared-v72`. The original main hash is `0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176`.

No builds, native tests, runtime probes, locks, installs or commits were performed by this worker. The coordinator must compile the native trait/query expansion, run these fixtures and the full relevant suite, then measure identical OFF/ON workloads before deciding whether this hypothesis improves agent wait.
