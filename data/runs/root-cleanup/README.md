# Owner-local cleanup demand integration

Source snapshot `ea45cd17f` adds typed per-owner ROOT_CLEANUP_DEMAND125 for closed nongeneric roots, derived independently from canonical Source publication. ANALYZE and OBJECT read owner demand instead of a whole MONO_PLAN output solely for Drop admission. Final CHECK still validates sealed cleanup demand and executable inventory membership. Generic instance paths are unchanged.

## Central acceptance

- Forced hosted check passed: `20261018-214338802-444448`.
- Eight authority/accounting unit tests passed: `20261018-214342404-445411`.
- Thirty-seven graph, stock Drop, heap and trait-object regressions passed: `20261018-214349550-447194`.
- Frozen full local JOBS4/no-fail-fast: **3,424 passed, zero failed**, 189 binaries, `20261018-214446774-463960`. No isolated retry warning appeared.
- Strict all-target clippy passed explicitly on hosted: `20261018-215216556-462874`. Formatting passed before the full suite.

Public graph assertions follow recorded task-to-node relationships and verify each tested closed root proof/object depends on125 but does not transitively await113. Authority negatives preserve foreign/same-snapshot owner rejection, exact type arguments and source publication, missing/swapped demand, resource admission, and original final-inventory checks. Graph integrity ratchets are unchanged.

This is a bounded dependency cut, not complete graph unification. PARSE/RESOLVE/SIGNATURES/MONO_PLAN, generic target authority, and immutable PlanCtx work remain open. Full Rust is incomplete and real-program corpus coverage remains3/20. Contended correctness runs are not speed evidence.

Selected stable-client redacted exports retain aggregate verdicts, resources, diagnostics and run ids; per-binary lists are replaced with `tests.binary_count`. The isolated prototype scope and its original failed-fixture history remain documented in `docs/root-cleanup-125.md`.
