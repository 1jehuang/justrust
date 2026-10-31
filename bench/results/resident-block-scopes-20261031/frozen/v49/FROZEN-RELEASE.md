# FROZEN SOURCE RELEASE: block-scopes-v49

Explicitly released to root for native gate. Hold further source edits pending diagnostics. No native job, test, build, commit or performance measurement run by this lane.

Full patch SHA256: f02ceb5cb3a5e27627321502dadeb2fd4dc4fb0a7e61f8167ba14b7a1f8ea62f
v47-to-v49 patch SHA256: 9b420a99674daa9551b9833f147bfe586fd04d0085e2a8d5cb59e7dca4a7ca4d

Changes since v47 ONLY:
- nameres/frozen.rs fixtures: exact observed scope provider name Body::block_scopes_, retaining strict1/2 counts. Materializing matcher exact Body::of_/relative_/with_source_map_, shared with cold original Body positive control requiring of_/relative_ each1, total2. No wildcards. Positive lang-item fixture enables feature(lang_items), required by crate_lang_items feature gate, and independent fullwalk must first produce exactly3. Twelve focused tests unchanged in number, none skipped.
- expr_store/tests/body/block.rs: ONLY expected internal BlockId numeric881 ->901, matching actual v48 output. All other semantic/module fields identical, verified actual failure log. No broad snapshot updates.
- docs/resident-block-identity.md: actual v48 failures and narrowly scoped corrections, no success claim.

Hashes:
- nameres/frozen.rs: 9e33646a05e4bdf388131784128d9bc10949da03b459cf83e5afee672c7a7c3e
- expr_store/tests/body/block.rs: 0e2a818ebe384ac3300233eb6d9ccd9ed5e650a1612315ebbaa94250fe2c6644
- docs/resident-block-identity.md: 545a4de11d78ef8a4195d57719d8024abcda63320c281f39c8fbfd0206d0d3b6

Original/candidate full9Rust paths and candidate doc preserved with manifests. v47 historical archive untouched. v48-native-failure.log and v48-full-hir-def-failure.log retain actual failures. Root observed focused8PASS4FAIL and full540PASS5FAIL1ignored, NOT acceptance.

Production implementation identical to v47, canonical query/empty output arena invariants unchanged. Original wire/schema6 unchanged. Source-only rustfmt check exit0, diffcheck no whitespace diagnostics (no-index exit1 denotes differences).

Root must validate exact materializing event names/counts, enabled positive fixture original3/candidate3, all12 fixtures, full regressions and performance. Nothing here asserts native pass or speedup.
