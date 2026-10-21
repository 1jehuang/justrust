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
01-05 and 07 pass every body under the semantic core (was 01, 04, 07), from
native range/array/slice `for` items, primitive intrinsics through the shared
`ir::intrinsics` table, builtin slice/array/str `len` and IEEE float
`PartialEq`/`PartialOrd` (commits d3bf91370, 827280eae, 978db0784). CHECK still
uses the old checker. Corpus remaining blockers, by kind:

- Library surface: `str::chars`/`split_whitespace`, `[T]::iter`/`swap`/`to_vec`,
  `Iterator::map`/`fold`/`cloned`, `Result::map_err`.
- Lowering into `sema::hir`: opaque returns, associated consts. Format macros
  and literal output lower since 568d3be95 (bodies: 14 10->11/13, 15 4->5/7,
  18 7->9/11, 19 18->20/21; 19 now needs only `Iterator::fold`).
- PLAN failures before checking: `dyn` outside a reference (13), `HashMap` (17, 20).

## Cutover finding: the old checker gates library growth

Library methods are ordinary prelude source, so every addition must also pass
the old checker. A `StrMethods` trait for `chars`/`split_whitespace` (with
stock-differential cases) was withdrawn: trait methods with a `str` Self type
fail `ir::reject_references` ("slice generic arguments are unsupported") in
trait/impl validation (`traits.rs`, `predicate_subjects.rs`), and the trait
also shadowed the working builtin `str` length. Further library growth
therefore either relaxes those old-checker gates one by one or waits for step 3.

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

## Not yet covered

Lifetimes/regions (handled by borrow checking, not here), higher-ranked
bounds, specialization, auto traits beyond Sized/Copy, trait objects with
associated type bindings, generic associated types, const generics, and
coherence/overlap checking. Each is an explicit unsupported error, never an
approximation.
