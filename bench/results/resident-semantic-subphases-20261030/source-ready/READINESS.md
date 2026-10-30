# Semantic subphases v25 source readiness

READY FOR ROOT NATIVE GATES, NOT NATIVE-VALIDATED.

Base HEAD: 1cb15994a5ecc88bd67605156c42d4cae2e9dbb8. Three backed-up Rust originals match HEAD byte for byte. New document original absence is recorded separately. Ownership is exactly four paths, listed by frozen.sha256. No rejected selective guard restored. No dependencies, locks, queries, caches, filter policy, or semantic logic changed. Existing descendants filter remains exact.

Frozen full inputs: frozen/<repo-relative path>. Originals: originals/<repo-relative Rust path>. Patch: four-paths.patch, SHA256 e97c7c63f5c7819c60d324d2236804c909109ee8ba7d32c84003c26d7ce59955. frozen.sha256 lists all four full-file hashes. originals.sha256 lists the original Rust files.

New actual API: semantic_diagnostics_of_bodies_with_timings, same existing selective arguments plus Option<&mut Vec<(&'static str, std::time::Instant, std::time::Instant)>>. Existing APIs all pass None. No new clock reads on None. Main only invokes timed API when exact startup diagnostics gate is enabled, then converts actual intervals on the same caller thread into existing 4000-cap collector with unchanged drop counts. Existing outer phase and callback sums are inclusive and preserved. Early no-crates return emits only three reached phases.

Native test filter: resident_semantic_subphases. EXACTLY FIVE #[test] functions:
- tests::resident_semantic_subphases_all_bodies_exact
- tests::resident_semantic_subphases_no_bodies_exact
- tests::resident_semantic_subphases_selective_bodies_exact
- tests::resident_semantic_subphases_disabled_diagnostic_exact
- tests::resident_semantic_subphases_disabled_clockfree_api_exact

The similarly named _compare helper is NOT a test. Four tests compare old/new actual API complete diagnostic Debug output and exact callback visits, asserting five actual labels, enclosing Instants, adjacency, appended sentinel, positive collect duration and actual callback containment. Fifth covers old API versus optional API None and absent disabled recorder output/start state. Diagnostic has no PartialEq, so all Debug-visible fields are compared, including fixes and main-node pointers.

Worker ran only read-only source inspection, rustfmt on three owned sources with skip_children=true, git diff --check and original/hash comparisons. No native builds, native tests, benchmarks, workloads, lock resolution, installs, commits or pushes. Root must use its exact 2444-file snapshot/private Cargo.lock for actual owned ide-diagnostics API tests, then positive profiling before commit. No speedup claim.

All source writes are finished. Frozen readiness releases these four paths to root for the assigned gates. Unrelated initial dirty files were preserved.
