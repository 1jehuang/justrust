# Selective diagnostics syntax traversal

## Bounded source change

`semantic_diagnostics_asked` now checks `bodies.is_none()` before constructing
and consuming the syntax descendants iterator. Previously its iterator filter
rejected every node for a selective request but still visited the entire tree.
The full request still visits the same descendants in the same order and calls
`useless_braces`, `field_shorthand`, and `json_in_items` with unchanged arguments
and ordering. Selective requests already omitted those syntax-only diagnostics.

No diagnostic filtering policy, body selection predicate, query demand policy,
cache contents, cache keys, source hashing, or flat-text helper changes are part
of this patch. Parsing and all later semantic processing remain in place.

## Native regression coverage

Three tests in `ide-diagnostics/src/tests.rs` use real fixture databases and the
actual exported diagnostics APIs. Their shared filter is
`resident_selective_traversal`:

- `resident_selective_traversal_full_api_preserves_syntax_warnings` checks that
  `full_diagnostics` retains unnecessary import braces and redundant field name
  warnings, while the all-bodies selective API omits both syntax-only warnings.
- `resident_selective_traversal_all_bodies_matches_full_semantic_errors` requires
  two real type mismatch diagnostics and compares the complete debug output of
  full semantic and all-bodies selective diagnostics, including ranges and fixes.
- `resident_selective_traversal_predicate_keeps_only_requested_body_errors`
  selects one named body, checks its exact erroneous expression range against
  the full semantic results, and verifies that selecting no bodies returns no
  diagnostics for that fixture.

These are API contract regressions, not a copied traversal algorithm or proof
that a specific number of syntax nodes was skipped. The source-only lane has
not executed these tests or any build or workload. Root owns native compilation,
the positive three-test count gate, diagnostics equivalence, measured workload
gates, and any commit. No speedup or acceptance claim is made here.

## Source freeze

Original committed Rust sources, complete candidate files, the scoped patch,
SHA-256 manifests, and a receipt are retained under
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/selective-traversal-v24/`.
The receipt identifies the precise baseline commit and source-only checks.
