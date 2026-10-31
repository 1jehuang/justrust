# Resident block identity and scope-only recovery, candidate v47

## Status and artifact boundary

**Source-only candidate, not accepted or promoted.** No builds, tests, native jobs, installations, commits, pushes, or performance measurements were run by this lane. Source rustfmt and diff checks are the only exercised checks. The coordinator owns native correctness and performance gates.

The initial no-op investigation is preserved separately at `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/block-identity-v46`. The implementation is **v47**, at `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/block-scopes-v47`. Its `original/` and `candidate/` contain complete authorized source files, with patch, hashes, source-check output, and an explicit frozen release. Do not identify v47 as the original v46 no-op.

Changed Rust files, relative to `toolchain/resident/vendor/rust-analyzer/crates/hir-def/src/`:

* `lib.rs`: sole canonical tracked constructor query.
* `expr_store/lower.rs`: one shared semantic traversal with a private output sink.
* `expr_store/body.rs`: shared input selection, tracked scope provider, independent test-only whole-file oracle.
* `expr_store/lower/format_args.rs`: output-only boundary after original argument/template visitation.
* `nameres/frozen.rs`: exact scope membership recovery, encoder scope search, serving fallback, and strict native fixtures.

Also authorized and preserved, but unchanged: `expr_store/lower/path.rs`, `expr_store/lower/generics.rs`, and `toolchain/resident/crates/ra-jit/src/frozen.rs`. No other source paths are edited. No declaration cache is introduced.

## What the source investigation established

`BlockIdLt` is a tracked struct with a full `AstId<ast::BlockExpr>` and its containing `ModuleIdLt`. Before this change, `BlockIdLt::new_` was called by body lowering's constructor wrapper, while frozen decoding recovered the identical tracked block by walking `Body::of(owner).blocks(db)`.

The owner does **not** directly supply the target block's parent. For example:

```rust
fn outer() {
    struct A;
    {
        struct B;
        {
            struct C;
        }
    }
}
```

Let the item-bearing blocks have roots MA, MB, MC. The first block captures `outer.module(db)`, the second captures MA, and the third captures MB. All are in `outer`'s body. Replacing the parent with `owner.module(db)` would misidentify the second and third blocks.

`collect_block_` obtains an optional AstId from the current expander. For an allocated block, it creates the identity from the **current** module, obtains that block's def map, switches to the map's root for recursive collection, and restores the previous state afterward. An item-free block without an allocated AstId leaves the module unchanged. Arbitrary nearest syntax-block ancestors are therefore not interchangeable with allocated lexical parents.

Macro entry changes the expander's current file and maps, but does not replace the collector's lexical module. Consequently a macro-written block combines an expansion-file AstId with the current call-site lexical module. Repeated calls must retain distinct macro-file identities. `AstIdMap::relative_to` preserves file-local identities for relative body lowering. These facts are why a syntax-range memo or unchecked raw AstId reconstruction is insufficient.

## Implementation: one constructor and one semantic traversal

`canonical_block_id(db, full_ast_id, immediate_module)` is a tracked query returning a `BlockIdLt` by copy. It contains the **only** call to generated `BlockIdLt::new_`. The existing token-requiring wrapper delegates to it. Full body lowering and scope-only traversal use that same wrapper, so they create the same tracked identity under the same canonical query key instead of unrelated producer queries. `BlockIdLt` remains tracked, not interned.

`Body::block_scopes(db, owner)` selects the same current source input as body lowering, then invokes the same collector in private `scope_only` mode. It returns only the ordered `Vec<BlockId>`. It never calls `Body::of`, `Body::relative`, or `Body::with_source_map`. Relative and whole-file input selection are shared with the production body paths. A separately retained test-only original whole-file selector does not use those candidate helpers.

This is an output sink, **not a second copied syntax visitor**. Existing cfg decisions, skip handling, type/const/pattern traversal, macro grammar, sequential legacy macro counts, resolver decisions, and lang-item early exits execute in the original collector. It still does semantic work. The bounded hypothesis is avoided expression/type/pattern arenas and selected output-only desugaring, not elimination of all body traversal.

### Output invariants and retained state

In scope-only mode, expression, pattern, and type allocation helpers return private placeholder handles without inserting HIR into their arenas. These handles only connect temporary output values and must never be indexed or published. The scope entry returns only real tracked block identities. Hard assertions require all three output arenas to be empty at return. Normal `lower_body` asserts that a scope sink cannot publish a `Body`, and the placeholder constructor asserts scope-only mode.

All expression/pattern/type arena operations were inventoried across lower.rs, lower/path.rs, lower/generics.rs, lower/format_args.rs, and lower/asm.rs:

| Access | Scope-only boundary |
| --- | --- |
| Type allocation helpers | Return a private sink handle before arena insertion and source-map allocation |
| Expression/pattern allocation helpers | Same guard, including the original direct `Pat::Expr` insertion now routed through the helper |
| Coroutine parameter-pattern reads | `collect` returns after visiting the body, before coroutine rewriting; rewrite entry asserts materializing mode |
| Closure coroutine expression/pattern reads | Return after exact parameter/type/return-type/body visits and restore coroutine/try state, before rewriting; read boundary asserts materializing mode |
| Try-block tail mutation | Scope path returns after block visits and required result-type visits; materializing branch owns the arena mutation |
| Formatting literal HIR reads | Scope returns after original explicit arguments and template macro expansion; literal reader asserts materializing mode |
| Direct formatting argument allocation | Both formatting-output entry and argument constructor assert materializing mode |
| Arena lengths in inference-root bookkeeping | Allowed: length observation does not dereference a placeholder |
| asm.rs | Uses the guarded collector allocation helpers; no direct expression/type/pattern arena reads or insertions |
| lower/path.rs and lower/generics.rs | Lifetime arena reads remain real; no expression/type/pattern arena indexing |

Small binding, label and lifetime arenas remain, as do state required for exact branching: lexical module/def-map stack, expansion/maps, cfg, macro counters, binding bookkeeping, awaitable/coroutine/try state, and path/lifetime information. Temporary `Expr`/`Pat`/`TypeRef`/`Path` values, child vectors, some diagnostics and source-map bookkeeping are still constructed and discarded. The sink is deliberately bounded and makes no measured performance claim.

Output-only returns occur after original child visitation. This includes while conditions, for-loop head/pattern/body, try-operator operands, closures, function bodies, block contents, and format arguments. Typed try-block result types are visited after the block and after restoring the outer try state, as in original lowering. Formatting expands template macros after explicit argument expressions, before skipping parse/materialization of formatting output.

### Semantic traps preserved, not approximated

* `for` lowering does not visit head, pattern or body without its four required language items.
* `?` lowering does not visit its operand without Try/ControlFlow language items.
* Record fields are skipped if path lowering fails.
* Destructuring assignment consults `resolve_path_locally` and can take expression-as-pattern macro grammar.
* Function body lowering visits parameter patterns, not ordinary parameter type annotations. Extern functions and bodyless trait functions have a bare-identifier route. Closure parameter and return types do traverse.
* Type const arguments, type macros, pattern macros, statement macros and cfg-controlled nodes retain the original traversal choices.
* `#[rust_analyzer::skip]` may leave a block in syntax but no collected block in the body.

No fake ancestor synthesis, weakened membership check, new declaration hash cache, or global untracked block map is used.

## Frozen codec and compatibility

The wire remains **exactly owner + full AstId + module-table index**, with the existing block-module tag. `Dec::module` asks `current_block` for current scope membership, rejects missing or duplicate full AstIds, and queries that exact block's def map. The parent is already established by authoritative collector traversal. It is not reconstructed from `owner.module(db)` or a cached tuple. `Enc::block_owner` searches `Body::block_scopes` rather than materialized `Body::of` results.

Because this shares authoritative identity discovery, no recursive parent descriptor is needed. The earlier proposed parent-only format was investigated but is not implemented. Its finite lexical-parent relation is representable, but retaining the existing body owner supplies the membership-query key without a second owner-discovery algorithm or an unnecessary format change.

The cache namespace remains `field(b"ra-jit frozen def map 6")` in `ra-jit/src/frozen.rs`. No serialized field, tag, grammar, or schema key is changed. Impl and lang-item namespaces derive from that crate key and remain unchanged too. Native roundtrip/fullwalk equality remains a required gate, not an assertion that compatibility has already been tested.

### Source revisions and decoder errors

The canonical constructor does not independently prove current source membership. `Body::block_scopes` does: it reads the same current relative/whole-file source, expansion, cfg, attributes, lang items and resolver inputs as the shared semantic traversal. The decoder only returns an identity in that current set. Removed or skipped blocks are rejected, and reparenting uses the new immediate module. It does not cache old `(owner.module, AstId)` tuples.

Raw AstIds can be reused after edits. Existing wire semantics identify a current block by its full AstId within the named body, not immutable historical source text. This candidate preserves that external behavior and does not claim to detect every historical node substitution. Strict tests compare current identities against original lowering, including structural edits, instead of assuming source offsets are permanent identities.

Before v47, `served` panicked on every decode error. It now returns `None` on a decoding error in **Serve** mode so its caller can resolve the live map. **Verify** still panics, and Check's difference machinery remains unchanged. Missing/ambiguous block errors therefore cannot authorize synthesized identities or hide verification failures. This fallback handles returned decoder errors, not arbitrary panics in lower layers. Native tests must exercise this contract.

## Native oracle and fixtures prepared, not run

The original materializing `Body::of(...).blocks(db)` remains an independent output path. The new test-only `Body::original_fullwalk_blocks` retains original whole-file body-input selection and full materialization, without using the candidate input helper or scope output. Strict fixtures compare exact ordered `BlockId`s, immediate modules, block-map parents and decoded `ModuleId`s, not merely equal AstIds. Both materializing oracles share the canonical identity constructor but do not execute scope-only traversal.

Filter: **`resident_block_scopes`**, twelve tests in existing `nameres/frozen.rs`:

1. `resident_block_scopes_nested_macros_and_closures`
2. `resident_block_scopes_codec_identity_without_body_providers`
3. `resident_block_scopes_semantic_early_exit_and_signature_traps`
4. `resident_block_scopes_moves_edits_reparent_and_removal`
5. `resident_block_scopes_macro_written_and_macro_revision`
6. `resident_block_scopes_cfg_and_skip_revision_dependencies`
7. `resident_block_scopes_serving_decode_errors_fail_open`
8. `resident_block_scopes_verification_decode_errors_remain_strict`
9. `resident_block_scopes_format_args_visits_before_output`
10. `resident_block_scopes_const_static_variant_and_inline_module`
11. `resident_block_scopes_positive_lang_item_gates`
12. `resident_block_scopes_local_owner_descriptor_is_finite`

Hard fixture counts include 13 blocks and four macro files for the main nested/closure case, one cold owner scope execution for its decoder, and two scope executions for the block-local function owner descriptor. Cold candidate windows assert zero executions of each materializing Body provider before running the independent oracles. Tests exercise repeated same-revision reads, revision source moves, body/signature edits, reparenting, removal, macro expansion edits/removal, cfg and skip changes, formatting, positive/negative language-item gates, constants/statics/variants and inline modules. No tests are skipped, and no pre-existing assertions or fixtures are weakened.

### v48 native observations and v49 fixture-only repair

The root coordinator ran the v47 bytes in its private v48 snapshot: eight focused tests passed, four failed. Three failures observed exact Salsa event `Body::block_scopes_` rather than the fixture matcher without its suffix. v49 corrects only that exact name and retains strict expected execution counts of one/two. The positive language-item fixture failed with zero scopes versus three: `crate_lang_items` explicitly returns no items unless the crate enables `#![feature(lang_items)]`. v49 enables that required fixture feature and independently requires the original fullwalk to discover exactly three blocks before comparing the candidate. No production source changes address these fixture defects. The root also ran the unchanged v48 full hir-def suite: 540 passed, five failed, one ignored. The fifth failure was existing `nested_module_scoping`, whose actual output differed solely in internal tracked BlockId allocation number 881 versus 901. With narrow coordinator authorization, v49 changes that single expected numeric line to the observed 901, preserving every module/parent field and all semantic identity assertions. The full failure log and original snapshot are archived. This is not a full-suite pass.

The original materializing absence matcher also omitted generated suffixes. v49 uses the exact set `Body::of_`, `Body::relative_`, `Body::with_source_map_`. A separate cold real Body execution in the codec fixture must execute `Body::of_` and `Body::relative_` exactly once each, and the SAME absence matcher must count exactly two materializing executions. Candidate windows still require zero. Thus wrong exact names cannot make the zero-provider oracle pass vacuously. The positive control names/counts remain subject to the next actual native run, not assumed passed.

Full v47 bytes and the actual v48 failure log are preserved. v49 contains only fixture/doc repairs relative to v47, with unchanged production behavior, wire/schema6, twelve tests and strict membership counts. Native runtime correctness and performance remain **unaccepted** until the coordinator completes the candidate gate. This lane ran source-only formatting/diff checks, no native jobs. A source-only release is not a native pass and not a speedup claim.
