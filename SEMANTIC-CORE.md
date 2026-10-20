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

## Not yet covered

Lifetimes/regions (handled by borrow checking, not here), higher-ranked
bounds, specialization, auto traits beyond Sized/Copy, trait objects with
associated type bindings, generic associated types, const generics, and
coherence/overlap checking. Each is an explicit unsupported error, never an
approximation.
