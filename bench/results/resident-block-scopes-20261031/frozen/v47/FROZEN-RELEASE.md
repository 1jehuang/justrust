# FROZEN SOURCE RELEASE: block-scopes-v47

Released to root coordinator for native compilation and validation. Source is frozen as of this manifest. No native acceptance or performance claim.

Patch SHA-256: 505b5423136c53d0a5113e5e7db3aa91a0fa34f30c4692839a381effd702b1d8

Changed Rust source SHA-256 (relative to hir-def/src):
- lib.rs: 0c3ed12a5099a10f0de51622b8d24cf9eab2a9705e7acdc2cd0acf49bc42d8d6
- expr_store/lower.rs: 41e23847ee083445bc5cb746e36918765d92c1070d0814f6f28fbf100a26ee87
- expr_store/body.rs: 2d9d06361837f4ca05ad61580a693ce398cef87e74a1ddbe7b4ae3f4347100fa
- expr_store/lower/format_args.rs: d953598d0ad49355a9680bdb77b8aa836602dde75473dc7799c619805a1aa5cb
- nameres/frozen.rs: f42b2088a8100be8032fa03d0aede099b67c39bc9e3fd22271be30d5683f7e65

Complete originals: original/ (8 authorized Rust files).
Complete candidate: candidate/ (8 authorized Rust files plus doc).
Full manifests: original.sha256, candidate.sha256.
Patch: candidate.patch (5 changed Rust files and added doc).
Source checks: rustfmt-check.txt empty, exit 0. diffcheck.txt empty, no whitespace diagnostics. no-index diff exit 1 denotes changed files.

Canonical construction:
- lib.rs canonical_block_id is tracked, returns(copy), keys full AstId + immediate ModuleIdLt.
- Only BlockIdLt::new_ call remains inside that query.
- Existing token-requiring wrapper delegates, and both materializing and ScopeOnly paths use it.

Codec disposition:
- KEEP original owner + AstId + tableindex wire and schema6.
- Encoder block_owner and decoder current_block use Body::block_scopes.
- Missing or duplicate full AstId membership rejects decoding.
- Serve errors return None for live fallback. Verify errors remain strict. Existing Check behavior unchanged.

Scope sink:
- Same collector branches, cfg/macro/resolver/langitem semantics and input selection, not copied visitor.
- Private dummy Expr/Type/Pat handles cannot escape in a Body or scope result.
- Empty expr/pat/type arenas are asserted at return, normal Body publication asserts non-scope mode.
- Coroutine and formatting read/materialization boundaries assert non-scope mode.
- Scope returns after exact child visits, before output-only rewrites.
- Binding/label/lifetime arenas and semantic state remain real. Temporary constructor values, child vectors, diagnostics and some maps remain. No claim of eliminating all semantic work.
- Full access inventory is in docs/resident-block-identity.md.

Native filter: resident_block_scopes. EXACT 12 tests, full names in native-tests.txt:
1 nested_macros_and_closures
2 codec_identity_without_body_providers
3 semantic_early_exit_and_signature_traps
4 moves_edits_reparent_and_removal
5 macro_written_and_macro_revision
6 cfg_and_skip_revision_dependencies
7 serving_decode_errors_fail_open
8 verification_decode_errors_remain_strict
9 format_args_visits_before_output
10 const_static_variant_and_inline_module
11 positive_lang_item_gates
12 local_owner_descriptor_is_finite
(all prefixed resident_block_scopes_)

Strict oracles:
- Existing Body::of.blocks full materialization.
- Test-only Body::original_fullwalk_blocks preserves original independent whole-file input selection and full materialization.
- Exact BlockId order/module parents/decoded ModuleIds, not only AstId equality.
- Main fixture expects 13 blocks and four macro files. Cold scope/decoder expects one provider execution; local owner codec expects two.
- Candidate windows require zero Body::of/relative/with_source_map executions before oracle runs.

NOT RUN by this lane: builds, tests, native jobs, install, commit, push, benchmarks.
All compile validity, Salsa event-name assumptions, fixture counts and runtime correctness remain pending actual root gate. No fixture skipped, no existing assertion softened.

Historical block-identity-v46 archive remains the original investigation-only release, not this implementation.
