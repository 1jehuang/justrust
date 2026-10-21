# Semantic core: general inference and trait solving

Status: in progress (2026-10-20). Library landed under `src/compiler/sema/`;
frontend adoption is open work. No corpus, compatibility or speed claim yet.

## Why

The corpus admits 5 of 20 short programs. The rejections are basic Rust:
literal inference across uses, closure parameter inference from a callee
bound (E0282), method lookup on generic receivers, associated type
projection through adapters. Each was being handled by a separate feature
slice (`local_literals`, `source_replay`, `trait_method_hint`,
`iterator_loop`, `nominal_manifest`, ...). Every slice adds its own partial
unifier and its own selection rules, so cost per feature grows and the slices
disagree. Probes on 2026-10-20 (HEAD `519a7ff4d`): `vec![..].iter().enumerate()`,
`v.iter().map(|x| x + 1).collect()`, `v[i] += 1` on an inferred `Vec`, a
source-defined `Iterator`/`Map` pair, and `Wrap::new()` with a later `add(5)`
all fail, each at a different special case.

rustc rests on three general engines. This module provides the first two
as one pure library, in the same style as `src/compiler/ssa/`:

1. **Inference context** (`infer.rs`): union-find type variables with
   general/integer/float kinds, structural unification with occurs check,
   undo-log snapshots for probing, deep resolution and literal defaulting.
2. **Trait solver** (`solve.rs`): a fulfillment context over trait and
   projection obligations. Candidate assembly from caller bounds (elaborated
   through supertraits), impls and builtins (Sized, Copy/Clone for scalars,
   Fn traits for closures and fn pointers). One candidate confirms, several
   stay ambiguous until more is known, none is an error. Projections are
   normalized lazily: an unresolved `<T as Trait>::Item` becomes a fresh
   variable plus a pending projection obligation.
3. **Method probe** (`method.rs`): autoderef steps (references, raw
   Deref impls, final array-to-slice unsize), by-value then `&` then `&mut`
   autoref, inherent impls before in-scope traits.
4. **Coercion** (`coerce.rs`): never, reborrow, `&mut` to `&`, deref coercion
   through the same autoderef, array-to-slice unsizing, else unification.
5. **Body checker** (`check.rs`): bidirectional checking of a resolved body
   IR (`hir.rs`). Callee predicates are registered before arguments are
   checked, so a closure argument's signature is deduced from pending
   `F: FnMut(A) -> B` obligations. That is the general fix for E0282, not a
   closure-specific path.

## Graph contract (GRAPH-INTEGRITY.md)

- The library is pure: no statics, locks, atomics or shared budget handles.
  Everything is a deterministic function of its arguments.
- One inference context lives inside one body-check node and is dropped when
  the node finishes. Inference variables never appear in a published fact.
- The solver reads program knowledge only through the `TraitDb` trait. Its
  graph implementation must answer each query from a per-item fact:
  `impls_of(trait)` is the explicit "all impls of this trait" completeness
  fact, `inherent_impls(simplified self type)` is per type key, and
  `trait_data` / method signatures are per declaration. No query reads every
  item.
- Resource limits are a `Meter` owned by the context, sized from the body's
  own input. Recursion depth on obligations is bounded (`MAX_DEPTH`) and
  failure is `Limit`, distinct from type errors and unsupported input.
- Repeated trait goals across bodies should become one graph node per
  canonical goal (closed goals only). That replaces the `Mutex` memo tables in
  `ValidatedTraitWorld`, which are recorded integrity debt.

## Adoption plan

1. Library and unit tests (this commit series).
2. `TraitDb` adapter over existing per-declaration facts.
3. Frontend lowering of a body into `sema::hir` after name resolution, and a
   CHECK path that uses `check.rs`, compared program by program against the
   current checker and stock rustc on the corpus.
4. Delete the special-case solvers it subsumes (`local_literals`,
   `source_replay::CallInferenceState`, `type_check/*_hint.rs`, ...), with a
   line-count and corpus delta recorded for each deletion.

## Status (2026-10-21)

Shadow probe (`sema::adapter::tests::corpus_plan_stage`, ignored): programs
01-05, 07, 08 and 19 pass every body under the semantic core (8/20; was 01,
04, 07 at f1242cac3). CHECK still uses the old checker. Commits: native
range/array/slice `for` items, primitive intrinsics through the shared
`ir::intrinsics` table, builtin slice/array/str `len` (d3bf91370, 978db0784);
IEEE float `PartialEq`/`PartialOrd` (827280eae); format macro lowering
(568d3be95); `Iterator::fold` (307a11d36); `Result::map`/`map_err`
(88a08adf4); native array `.iter()` (4999d32b3). Remaining blockers, by kind:

- Library surface: `str::chars`/`split_whitespace`, `str::is_empty`,
  `[T]::iter`/`swap`/`to_vec`, `Iterator::map`/`cloned`.
- Lowering into `sema::hir`: opaque returns, associated consts.
- PLAN failures before checking: `dyn` outside a reference (13), `BTreeMap`
  (17), `Rc` (20).
- Old-checker gaps found while adding library methods (each recorded in its
  test): a user struct produced through a generic closure result gets two
  nominal identities; `.map_err(..)?` needs an annotation; a by-value
  `mut acc` closure parameter calling `&mut self` methods is rejected.
- `for` over a source iterator value is rejected by the native compiler at
  every commit checked back to 1b43e6a71 ("nominal source constraints cannot
  progress", nominal_realization/preparation.rs), including
  `for x in vec.iter()`; `tests/iterator_associated.rs`
  `native_vec_shared_iteration_matches_stock` and `unchanged_sieve_matches_stock`
  fail on main a27587346. `while let Some(c) = it.next()` over the same
  iterator works. A prelude `str::chars` (`StrMethods`, Self = `str`) compiles
  and runs through `.next()`, so trait methods on `str` are not the blocker
  recorded above; `for c in s.chars()` is. Native corpus at a27587346: 6/20
  (01, 02, 03, 05, 07, 08), unchanged.

## Cutover finding: the old checker gates library growth

Library methods are ordinary prelude source, so every addition must also pass
the old checker. A `StrMethods` trait for `chars`/`split_whitespace` (with
stock-differential cases) was withdrawn. Correction (later the same day):
the "slice generic arguments are unsupported" error came from declaring `len`
in the trait, which shadowed the builtin `str` length; a `chars`-only trait
compiles and runs through `.next()`. The remaining blocker is `for` over any
source iterator (see Status). Further library growth therefore either
relaxes those old-checker gates one by one or waits for step 3.

Step 3 is not a CHECK-only swap. The backend consumes `ir::CheckedFunction`,
which carries more than types: `call_evidence` (argument traces and region
actuals), `ref_contract`, `runtime_sites` and `body_substitution` proofs that
ownership and CFG lowering validate. `Typeck` produces expression/local types,
method resolutions, intrinsic and length sites only. The cutover therefore
needs either (a) a `Typeck` -> `CheckedFunction` lowering that reconstructs
that evidence from resolved types, or (b) the old checker consuming `Typeck`
as its inference oracle in place of its own partial unifiers, keeping its
evidence production. (b) is incremental, deletes special cases directly and
keeps the backend contract; it is the recommended first slice.

## Cutover prerequisite: per-item TraitDb (found 2026-10-21)

`WorldDb::build` reads every trait, impl, callable, constant and nominal in the
program (`for h in headers.*`, `for info in plan.sema_declarations()`). That is
fine for the ignored shadow probe, but calling it from a per-body CHECK node
is a disguised whole-program join, which GRAPH-INTEGRITY.md forbids. The
`TraitDb`/`ItemDb` traits were designed for per-item answers, but the adapter
does not provide them yet. Before any CHECK path consults `Typeck`:

1. Back `TraitDb`/`ItemDb` with per-item graph facts, demanded on access:
   `trait_data`/`fn_sig`/`adt` per declaration, `impls_of(trait)` as the explicit
   all-impls-of-a-trait completeness fact, `inherent_impls` per simplified type.
   Because demand is async, either prefetch per body (collect needed ids, request
   all, then await) or run the checker as a suspendable node step.
2. Then the oracle slice: in `block_with_expectation`, when `local_literals`
   leaves a `let` site unresolved, take its type from `Typeck` (sites key on the
   same `*const Stmt` address), converting nominals through `intern_nominal`.

Step 1 touches PlanCtx/trait-world queries (jaguar's lane while it is being
split) and the CHECK fan-out (lizard). It needs their agreement on the fact
shapes before code.

Coordinator ruling (shark, 2026-10-21): per-declaration header facts belong to
gorilla's SIGNATURES-per-item split; sema must consume those, not build a
parallel set. The oracle lives behind a per-body fact in frontend type_check,
never in the CHECK root arm (CHECK only aggregates per-item results). Proposed
to gorilla, shared with trait selection and coherence:

- `IMPLS_OF_TRAIT`: `Item<K>(trait DeclId)` to the ordered trait impls naming
  that trait (explicit completeness fact).
- `INHERENT_IMPLS_BY_SELF`: `Params<K>(tag, [payload])` over a simplified self
  type (Bool, Char, Str, Int(IntTy), Float(FloatTy), Adt(DeclId), Slice, Array,
  Ref, RawPtr, Tuple(arity)) to the ordered inherent impls, including
  constant-only impls. No display names.

Agreed with gorilla (2026-10-21): both are separate node kinds (reading one
per-item SIGNATURES header never waits on all impls). `IMPLS_OF_TRAIT` keys on
the trait DeclId only. Both orderings are canonical (module ordinal, then decl
index), never hash order. Both producers read the per-module impl inventory
(`MODULE_INDEX`, kind 122, arriving with gorilla's RESOLVE-per-module split),
not per-item facts. hare implements both in gorilla's lane once SIGNATURES per
item exists. Sequence: gorilla PARSE split, then RESOLVE per module (adds
MODULE_INDEX), then SIGNATURES per item; then these two facts; then sema
TraitDb over them; then the oracle slice.

## Not yet covered

Lifetimes/regions (handled by borrow checking, not here), higher-ranked
bounds, specialization, auto traits beyond Sized/Copy, trait objects with
associated type bindings, generic associated types, const generics, and
coherence/overlap checking. Each is an explicit unsupported error, never an
approximation.
