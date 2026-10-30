# v27 source readiness and ownership release

Frozen at 2026-10-30 21:21 UTC. Source-only preparation is COMPLETE.

Ownership of ALL THREE paths is explicitly RELEASED to coordinator skunk:

- toolchain/resident/crates/ra-jit/src/outline.rs
- toolchain/resident/crates/ra-jit/src/main.rs (root explicitly expanded ownership after 3b13e250b)
- docs/resident-test-diagnostic-skeleton.md (NEW, absent initially)

I will not edit these paths after this handoff unless root explicitly reassigns
ownership. No builds, tests, native workloads or commits were run. Only rustfmt
with skip_children=true on the two owned files and git diff --check were run.
Both formatting and whitespace checks returned successfully.

## Exact frozen SHA-256

```text
fa945a16ca1878917281192cac100d57b97f77f05fbd7ea41798e4c2d8693b8f  toolchain/resident/crates/ra-jit/src/outline.rs
f943040dc0c8ec283ece3b9f79c2c860e5817feaaceb03d378a6879640b89c83  toolchain/resident/crates/ra-jit/src/main.rs
212490fd40c3e7c674c05e9cbe7d982746061d9a7388b03d264a47955dc240c2  docs/resident-test-diagnostic-skeleton.md
c69ffebf8e3e8389af16d3bbcbc415c3c1bda66b82239d24d424bd8bb96ba178  source.patch
```

`originals/` contains full initial outline.rs and full main.rs at expanded
ownership. `originals.sha256` identifies both. There was no original docs file.
`candidate/` contains all three exact frozen candidate files.
`candidate.sha256` hashes those relative paths. `source.patch` includes all Rust
source changes and the new documentation patch. No other repository path was
edited by this worker.

## Gate filters and expected counts

`resident_diagnostic_skeleton`: EIGHT new native tests, seven outline tests and
one real errors_of cache oracle. Existing outline::tests remain intact.

1. outline::tests::resident_diagnostic_skeleton_omits_only_resolved_builtin_test_bodies
2. outline::tests::resident_diagnostic_skeleton_keeps_test_signatures_and_attributes
3. outline::tests::resident_diagnostic_skeleton_rejects_nested_test_items_and_item_macros
4. outline::tests::resident_diagnostic_skeleton_keeps_unresolved_and_proc_shadowed_test_bodies
5. outline::tests::resident_diagnostic_skeleton_preserves_all_body_visibility_guards
6. outline::tests::resident_diagnostic_skeleton_refuses_multiple_module_contexts
7. outline::tests::resident_diagnostic_skeleton_resolves_within_active_test_crate
8. tests::resident_diagnostic_skeleton_cache_matches_fresh_errors_after_each_edit

All fixtures are unexecuted. A native positive proof is REQUIRED before claims
or promotion. Minimal positive builtin fixture is in the new documentation.
The actual cache oracle requires positive cold/full, unchanged/zero, body/one
selection and exact FileRange/code/message equality with a fresh per-key CLEAN
check after every revision. RAII restores both sequence and original key state.
Nested recovery legitimately expects zero because a None skeleton preserves the
last clean SOURCE cache under existing behavior. Production eviction is unchanged.

## Conservative proof and wiring

Only errors_of is wired to diagnostic_skeleton(db,file,within.as_deref()).
Global INERT, global skeleton and codepatch policy are unchanged.
Actual attribute path resolution supplies hir::Macro -> hir_def::MacroId ->
lookup.expander -> BuiltInAttr(expander) if expander.is_test(). Neither simple
spelling nor Function::is_test alone is accepted. Proc IDs, unresolved attrs,
other expander kinds and unknown attrs remain body-visible. Argument-bearing or
qualified test attributes are not newly accepted.

Resolution is performed in every relevant ACTIVE within crate separately.
Each needs exactly one file module from that crate, checked explicitly because
Semantics::new_within can otherwise fall back. Same edition required; no selected
context, multiple modules or edition disagreement returns None. Proven source
ranges are intersected across active contexts. All existing ID/nested-item/
item-macro/const/async/gen/implTrait/body-attribute/inlinealways safeguards remain.
Expression-position macro behavior is intentionally inherited, not newly banned.

Root owns all next actions: build/check, eight new fixtures, existing outline and
prior focused/full native gates, prospective measurement, findings and commits.
No speedup or passing runtime result is claimed by this readiness receipt.
