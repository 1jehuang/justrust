# Rough-draft full Rust swarm (2026-10-19)

Goal: a working rough draft of full Rust support as fast as possible. Measure
progress by `tests/fixtures/full_programs/manifest.tsv` (20 programs, stock
oracle stdout) and by feature tags disappearing from it, not by slice docs.

## Rules for this push (override slower legacy process in AGENTS.md)

1. Unit of work = a whole feature working end to end (check, build, run, stock
   matching output), not a "bounded slice". Implement the general form of the
   feature in one go. Never narrow scope to make evidence easier.
2. Light evidence: one test file per feature with stock-matching programs and
   a few invalid inputs. No receipt docs, no per-run JSON, no hand-recorded
   hosted run ids, no review rounds. Update the feature's line in
   IMPLEMENTATION.md and the manifest `needs` tags when a feature lands.
3. Freeze resource accounting / ownership capability / lease / paid storage /
   receipt work. Use existing budget APIs as-is. Do not add new accounting.
4. Keep `tests/graph_integrity.rs` passing (no new whole-program nodes, no
   shared mutable state in facts). Do not raise baselines.
5. Put new code in new files/modules where possible. Keep edits to the big
   shared files (`frontend.rs`, `ir.rs`, `type_check.rs`, `cfg.rs`,
   `ownership.rs`, `traits.rs`, `backend.rs`) to small hooks. Commit small,
   `git pull --rebase` and push to `main` often (at least every hour of work)
   so others see your hooks. Resolve conflicts, never discard others' work.
6. Unsupported constructs must still produce an explicit diagnostic. Never
   miscompile silently.
7. Before pushing: `cargo fmt`, `justrust check`, the tests for your area,
   `justrust test --test graph_integrity`, `justrust test --test full_programs`,
   AND `justrust test --no-run` (every lib/test target must compile: adding a
   struct field or changing a signature breaks other test files). Also run
   `justrust test --lib` if you touched shared types. If a test outside your
   area breaks because of you, fix it before pushing. Never push red main.
8. No worktrees, no branches. Work in `~/justrust-clean-room` on `main`.
9. The checkout is shared: files you edit may also hold other agents'
   uncommitted hunks. Never `git add` a whole shared file blindly. Stage only
   your hunks (`git add -p`, or a private `GIT_INDEX_FILE` built from HEAD),
   check `git diff --cached`, and verify the staged tree compiles in isolation
   (e.g. `git stash`-free: export the index with `git checkout-index -a
   --prefix=$JCODE_SCRATCH_DIR/verify/` and `cargo test --no-run` there) before
   pushing. Commit your WIP often so other agents' commits can't depend on it.

## Streams

| Agent | Area | Done when |
|---|---|---|
| macros | `macro_rules!` (matching, repetition, hygiene), `vec!`, `format!`/`write!`/`assert!` family expansion, `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]` | tags derive, format-macro gone; 04/08/13 past macro/derive |
| patterns | or/@/range/slice/ref patterns, guards, match ergonomics, range indexing and subslices, `?` operator, labeled control flow, place-expression assignment generality | tags or-pat, match-ergonomics, try, slice (syntax side) gone |
| traits | generic traits/methods, where clauses, associated types, operator traits (ops-traits, ref-ops), autoderef/Deref, `dyn Trait`, unsizing, `Box<dyn>`, generic bounds, coherence basics | tags ops-traits, ref-ops, autoderef, dyn, unsize, generic-bounds gone |
| closures | closures with all capture modes, `move`, Fn/FnMut/FnOnce, function pointers, `impl Trait` arg/return, closures passed to generics | tags closure, fn-traits, impl-trait gone |
| runtime | heap allocator, Box lowering, drop glue/Drop elaboration with drop flags, panic (message + exit 101), bounds/overflow checks, real calling convention for aggregates | tags box gone; heap types usable by library agent |
| library | our own std written in Rust source compiled by our compiler: Option/Result + methods, Vec, String/&str methods, slices methods, Iterator + adapters + collect, HashMap/BTreeMap, Rc/RefCell, fmt (Display/Debug/Formatter, width/precision/fmt-spec, float printing), primitive methods/consts, `std::` paths | tags vec, iter, string, str, option-result, hashmap, rc-refcell, fmt-*, prim-*, std-paths, array-iter gone |
| projects | Cargo.toml/workspaces/targets/profiles, multi-crate builds, `extern crate`/dependency resolution between our crates, `#[test]` discovery and runner, `cfg(test)`, integration tests | a real multi-crate workspace with tests checks/builds/tests natively |
| consts | const/static items, const fn evaluation, const generics, array length expressions, promoted constants, `const` blocks; plus unsafe/raw pointers/unions basics | tag const gone, 06 past its boundary |

Coordinate with DMs when you need a hook from another area. The library agent
depends on runtime (heap) and traits/closures; it should write the library
source now and enable pieces as the compiler supports them, filling small
compiler gaps itself when blocked.
