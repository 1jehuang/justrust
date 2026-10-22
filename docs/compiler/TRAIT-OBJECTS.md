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

- Closed nongeneric `dyn P` is a rigid leaf in nominal source recipes, so
  callables taking `&dyn P`/`&mut dyn P` pass graph nominal preparation
  (regression fixed 2026-10-21; generic/associated principals stay explicit
  `unsupported: generic dynamic source nominal recipe`).

## Overloaded Deref receivers (2026-10-21, `tests/deref_index_traits.rs`)

- `*p`, `&*p`, `&mut *p` on nominal `p` select `Deref::deref`/`DerefMut::deref_mut`
  in both the local solver (`Kind::Deref`) and checked lowering. `DerefMut` and
  `impl DerefMut for Box<T>` are in the prelude.
- Method and field autoderef: `p.m()` / `p.f` read through one exact
  `Deref::Target` step when `p` has no direct candidate (incl. `Box<W>`).
  Each step is a proven trait call recorded at the receiver's inventoried
  coercion site. Required/provided return-origin contracts re-derive over
  proven-equal types when the required nominal is graph-realized.
- Remaining: autoderef fields directly inside `format!` args (explicit
  unsupported diagnostic), by-value receivers through Deref, multi-step chains.

## Explicitly unsupported (boundary fixtures)

`dyn T<Assoc = X>`, `dyn A + 'static`/auto traits, `where Self: Sized`
methods, `&mut T` to `&dyn T` (reborrow then unsize), references to normal
Drop owners as objects, `Box<dyn Trait>` (needs Box), return-position
`impl Trait`.

## Recovery validation (2026-10-19)

The recovered implementation at `90318bbe9` was clean and locally checkpointed
before integration. `91d6b18e6` combines it with per-caller admission, root-instance
planning, temporary borrows, range loops, scalar formatting, the prelude and
primitive methods. Existing frozen trait-object receipts were preserved; the two
new shared-reborrow receipts were recorded against stock rustc 1.98.1 first.

- Hosted `justrust check`: passed (`20261018-192631489-65112`).
- Hosted `justrust clippy --all-targets -- -D warnings`: passed
  (`20261018-192640170-62554`).
- Local trait objects, scalar formatting, prelude, loops, graph integrity and
  primitive-method suites: 34 passed (`20261018-192638905-72349`).
- Full local `justrust test --local --no-fail-fast`: 3291 passed, 16 reported
  failures including child-process duplicates, across 178 binaries
  (`20261018-192719891-121427`). Remaining failures concern formatting-related
  earlier-boundary expectations and prelude graph counts in associated types,
  guarded matches, panic, tuples, modules, literal-output authority and source
  configuration. They were reported to the owning lanes, not hidden by relaxing
  dyn fixtures or graph gates. This is not a fully passing composed suite.

Selected exported, path-redacted receipts are in
`data/runs/xps14/2026-10-dyn-recovery.jsonl`, separate from unrelated run history.
No compiler speedup or full Rust compatibility claim follows from these checks.
