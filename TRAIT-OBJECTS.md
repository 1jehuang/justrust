# Trait objects and `impl Trait`

Corpus: `tests/fixtures/trait_objects/` (stock receipts in `stock.tsv`,
`scripts/trait_objects_oracle.sh`), test `tests/trait_objects.rs` (CHECK and
BUILD at workers 1 and 8 through `jr-rust`).

## Supported

- Argument-position `impl Trait` as anonymous bounded type parameters
  (E0562 outside parameters, E0107 on turbofish over anonymous parameters).
- `&dyn Trait` and `&mut dyn Trait` for nongeneric principal traits without
  associated types, including supertraits and default methods.
- Shared virtual methods through `&mut dyn Trait` create whole-object shared
  reborrows, preserve both carrier words, and keep returned loans live through
  their last use. Mutable calls resume afterward and are rejected while a
  returned shared loan remains live (stock-frozen positive/negative fixtures).
- `Ty::Dyn(DynTy { principal })` is a rigid unsized metadata leaf, admitted
  only behind a reference. The reference carrier is two words (data, vtable).
- Unsizing coercion `&T -> &dyn P` and `&mut T -> &mut dyn P` (`ExprKind::
  RefToDyn`, `cfg::Rvalue::RefToDyn`) at let annotations, call arguments,
  array elements and returns. Concrete or rigid generic `T: P` sources.
- Dispatch is `Callee::Virtual`. Slots come from `TraitFacts::vtable_methods`
  (supertraits first, declaration order). Vtables are
  `[drop_in_place, size, align, methods...]`, read-only, deduplicated.
  Instance planning demands every slot implementation and the destructor at
  each unsizing site for its exact substituted referent.
- Dyn compatibility (E0038): generic methods, `Self` in parameters or results,
  associated functions without receivers, `Sized`/`Clone` supertraits. Unused
  object types are checked too. By-value `self` calls on objects are E0161.
  Missing impls are E0277, `&dyn` to `&mut dyn` is E0308.

## Explicitly unsupported (boundary fixtures)

`dyn T<Assoc = X>`, `dyn A + 'static`/auto traits, `where Self: Sized`
methods, `&mut T` to `&dyn T` (reborrow then unsize), references to normal
Drop owners as objects, `Box<dyn Trait>` (needs Box), return-position
`impl Trait`.
