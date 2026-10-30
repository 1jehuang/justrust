# Resident root classification experiment

This is an experiment in the licensed Blaze-derived resident engine. It is not part of the clean-room compiler rewrite and is not a demonstrated performance improvement.

## Change and ownership

Authorized implementation paths are `toolchain/resident/crates/ra-jit/src/main.rs` and `toolchain/resident/crates/hir-ty-codegen/mod.rs`.
Three native Salsa tracked queries retain per-function decisions:

- `exported_function`: original short-circuit order of exported symbol, linked symbol, allocator service.
- `rooted_function`: const-evaluator-only exclusion, then the original function/container generic test, returning `None` or `Some(generic)`.
- `main_function`: original HIR main classification. The existing first-match scan and crate-level `no_main` exclusion remain outside it.

Separate providers preserve original demand distinctions. Provided nested functions do not request exported classification. Compile-time-only functions do not request generic classification. Main discovery stops at the same first match. `is_generic_function` and `serves_allocator` accept `dyn HirDatabase`, using the same HIR operations and predicates as before.

V12 additionally makes the existing `hir_ty_codegen::compile_time_only` primitive a native `#[salsa::tracked(returns(copy))]` query. Its body remains exactly `AttrFlags::has_word(db, func.into(), "rustc_comptime")`. This is intended to backdate an unchanged boolean after raw attribute AST dependencies change, without bypassing invalidation on genuine attribute changes.

All invalidation and cross-item dependencies remain native Salsa dependencies. There is no manual revision counter, edited-file allowlist, or syntactic identity assumption. The original module and body declaration queries, body traversal, parallel lower warmup, export partitioning and collection order are unchanged. Static retention is deliberately unchanged.

## Source fixtures prepared for root-owned native validation

- `the_roots_of_a_listed_crate_are_its_functions_but_those_only_the_const_evaluator_runs`: original expectations, full original-classifier comparison, repeated-listing provider reuse assertion.
- `resident_root_classification_reuses_providers_after_body_only_edits`: per-provider counters in `[export, root, main]` order, exact positive cold count `[2, 2, 1]`, unchanged-repeat zero delta, and v12 body-edit zero delta expectation. Counts print per revision. Counting is thread-local and compiled only under `cfg(test)`. This small fixture stays below the existing parallel export threshold.
- `resident_graph_reuse_tracks_nested_items_and_macro_edits`: retains the original full declaration-walk comparison and adds the full original-classifier oracle after each revision. Transitions cover body/nested/macro edits, export name/no-mangle, panic-handler and personality linked symbols, compile-time-only attributes, type/const/container generics, provided nested functions, main rename/no-main, and used/link-section/exported statics. A real core-origin trait fixture covers allocator-to-local-trait transitions and external trait rename/removal/restoration without editing the implementation file.

The oracle repeats the original expressions directly, including export short circuits, kept statics, generic tests, main selection, all ordered vectors, and the provided distinction. Crucially, both compile-time-only filters call the original direct `AttrFlags::has_word(db, it.into(), "rustc_comptime")`, not the newly tracked primitive. It never calls the three new classification queries. Attribute toggles and restoration remain real database revisions.

## Native feedback retained from previous candidates

The root reported that v9 failed native checking because scalar Salsa queries defaulted to reference returns. V10 explicitly used `returns(copy)` for all three providers and passed root-owned check/build. Its original all-provider body-edit zero-reexecution assertion failed with cumulative counts 7 versus 5. That failure remains evidence against complete body-edit reuse in v10, not a performance result.

Source inspection identified a concrete invalidation path: `rooted_function` called `compile_time_only`, which called untracked `AttrFlags::has_word`, directly reading the source AST through `collect_attrs`/`attrs_source`. V11 changed only test instrumentation and explicitly tested selective reuse. The root observed exact native counts `[2, 2, 1]`, `[2, 2, 1]`, `[2, 4, 1]` for cold, unchanged, and body-edit revisions. Its graph/full-classifier oracle, listed roots, and telemetry filters passed. This established export/main reuse only, not all-provider reuse.

The root reported v11 runtime/error/recovery gates passed, but its prospective timing comparison did not establish a reliable total improvement. V11 remained uncommitted and was not promoted. The v12 primitive hypothesis must pass new native checks and its own matched prospective performance comparison. Prior v10 failure and v11 selective-reuse evidence are retained rather than rewritten as v12 results.

## V12 hypothesis and acceptance protocol

V12 strictly expects `[2, 2, 1]` for all three fixture revisions, meaning no downstream classification provider reexecution after the body-only edit. The primitive itself may reexecute its exact raw-attribute read and backdate its unchanged boolean. These v12 counts are unverified expectations pending root-native observation. A different breakdown fails the test. No assertion skips the independent original classifier oracle or accepts arbitrary invalidation.

Only source formatting and whitespace/diff inspection are performed by this worker. No native compilation, tests, or timing workloads are run here. Native diagnostic preservation, differential edits, the full test gate, and raw matched prospective before/after performance remain root-owned requirements. A source fixture is not evidence that its native test passed. Provider reuse is not evidence of an end-to-end speedup.

After SOURCE READY with exact hashes, these three paths are frozen until the root requests changes. Do not commit or promote this experiment before native correctness and prospective performance acceptance. This is the last bounded primitive hypothesis. Reject and remove a nonwin. No speedup is claimed by this document.
