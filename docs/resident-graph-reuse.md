# Resident declaration graph reuse candidate

Status: native v4 graph, listed-root, and telemetry unit filters PASS per coordinator report on 2026-10-30. Prospective suites pass their gates. Coordinator accepted this as experimental tracked declaration reuse, not a performance promotion. No attributable speedup is claimed. No builds or benchmarks were run by this lane.
This changes the licensed Blaze-derived resident implementation, not the separate clean-room compiler or shipping client.

## Hypothesis and boundary

The supplied frozen re01 measurements show all 3990 roots (+40583 instances) visited on one-function edits, 36-56 ms reported compilation, under 1 ms actual codegen, roughly 1 ms graph resolution, 12-34 ms apply, and 70-114 ms edit-to-binary. The supplied five-edit aggregate is 0.121279609 s candidate versus 0.108226630 s source-built reference. These are coordinator observations, not measurements made by this lane.

`Listed::touch` deliberately invalidates affected crate listings and reverse dependencies. Rebuilding those listings previously walked every module scope and every body's block scopes anew in `declared_in`. This candidate memoizes the declaration graph nodes with Salsa:

- `module_declarations(crate)` retains the module/impl/trait declaration traversal.
- `body_declarations(body, under_provided)` retains each body's nested declaration traversal, including macro-expanded block scopes.
- `Declared::extend` combines all five declaration/queue vectors without dropping edges or reordering them.

The boolean key preserves provided-method classification, including nested functions under provided methods. Both queries use native database identities, not paths, source hashes, outline hashes, or a manually maintained epoch. All dependency reads occur synchronously within the tracked query. Existing parallel `lower` is outside these queries, so dependencies are not accidentally collected on detached snapshots.

This is intentionally narrower than a complete reachability cache. It does not eliminate the 3990-root codegen reach traversal. It does not bypass `lower`, body validity checks, `Listed::touch`, macro expansion, extern-input detection, diagnostics, trait/impl preparation, rejection revalidation, or compilation. If telemetry attributes the cost elsewhere, reject this candidate rather than claim the reported compile count proves a win.

## Correctness fixture supplied

`tests::resident_graph_reuse_tracks_nested_items_and_macro_edits` changes one database over eight revisions and compares the tracked traversal with the original uncached algorithm after every revision:

1. Initial nested function, declarative macro expansion, provided-method nested function, static-initializer nested function.
2. Executable-only literal edit.
3. Nested item replacement with an additional nested level.
4. Macro definition change affecting emitted nested items.
5. Provided-method nested item rename.
6. Static-initializer nested item rename.
7. Nested function removal.
8. Restoration to the original text.

The comparison covers function/static vectors and both work queues, preserving exact traversal order and classification. Only the final traversal-state boolean is normalized, since it is not a graph property. The existing listed-root fixture must also pass.

## Root acceptance contract before commit

The coordinator owns all native builds/bootstrap and workloads. Run the new `resident_graph_reuse` unit-test filter and existing listed-root fixture, then the exact frozen four-test native gates. Keep their diagnostics assertions and proc-macro external-input gates unchanged. Include structural nested-item changes and macro-definition changes in retained-worker testing. Run cold startup and body-edit samples with identical source identity, execution mode, fixture, edit sequence and sample count before/after. Compare phase distributions and end-to-end time, including memory or cold-start regressions from additional Salsa entries. Require measured benefit before committing this as a performance improvement. If no benefit, revert only this lane's main.rs diff and document the negative result.

Source-only checks performed here: rustfmt on the owned main.rs and git diff whitespace checking. No build was performed by this lane. Commit authorization followed coordinator-run native acceptance, as recorded below.

## Coordinator-reported prospective outcome, 2026-10-30

The v4 native graph, listed-root, and telemetry unit filters all passed after fixing the fixture setter to borrow `&source`. The earlier production v3 build passed, but its unit compilation failed and is not an acceptance result.

The actual prospective `rsb01` baseline to `rsa01` candidate suites passed all gates. Engine time changed from 0.119944310 s to 0.109617051 s (a descriptive reduction of 8.61%). The source-built reference also improved, from 0.124489961 s to 0.110923157 s. These paired observations do not establish an attributable graph-reuse speedup or robust faster-than-Blaze performance. The candidate combines graph reuse and telemetry changes, further limiting attribution. Actual v4 retained body-edit traces report roots 27.732 ms, apply 23.548 ms, check 12.490 ms, client planning 18.541 ms, and total 117.588 ms. These are nested elapsed measurements, not additive CPU costs or demonstrated time saved. All actual unit/runtime gates passed per coordinator report. On that evidence the coordinator authorized committing this as experimental tracked declaration reuse, with shipping defaults unchanged. The requested attributable performance improvement beyond Blaze-derived parity remains unmet. Production source remains frozen.
