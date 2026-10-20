# Match ergonomics, reference patterns and `ref` bindings

Status: implemented, stock-oracle tested (`tests/match_ergonomics.rs`).

## Semantics

- **Matching through dereferences.** `match *r { E::A(x) => .. }`, `if let`
  and `let-else` on `*r` or a projected place such as `r.field` switch on the
  referent's tag. Ownership analysis records the variant proof on the exact
  canonical loan target of the dereferenced place. An ambiguous referent (a
  carrier that may point at either of two locals) records no proof, so
  payload reads fail closed with `VariantNotDefinitelySelected` instead of
  being guessed.
- **Default binding modes (RFC 2005, edition 2024 rules).** A non-reference
  pattern matched against a `&T`/`&mut T` implicitly dereferences it and
  switches the default binding mode to `ref` / `ref mut` (shared wins once
  any shared reference is peeled). Bindings below it bind by reference.
  Explicit `&pat`, `ref`, and `mut x` inside an implicitly borrowing
  pattern are rejected exactly as rustc 2024 does.
- **`ref` / `ref mut` bindings and `&pat` / `&mut pat`.** `&pat` requires
  matching mutability and move mode, and dereferences the matched reference.

## Representation

- `ir::PatternBinding::borrow: Option<BorrowKind>`. `None` binds the
  selected place by value. `Some(kind)` gives the local type `&kind T`.
- Implicit and explicit dereferences are ordinary `Projection::Deref` steps
  in binding and decision-test paths. Decision paths admit `Deref`; aggregate
  domains admit `Ref` only when some binding actually dereferences it. A
  binding-free selection through a reference publishes `Wildcard`.
- CFG lowering (`bind_selected`) issues a fresh immediate `LoanSite` for
  each borrowing binding, so borrow and ownership analysis see a genuine loan
  of exactly the selected place (conflicts, mutability authority, and returns
  of pattern borrows are checked like any `&place`).
- Usefulness keeps references opaque for inhabitance but tests through them
  via `Deref` paths, so exhaustiveness of `match r { E::A(0) => .., .. }`
  is exact.

## Known gaps (other lanes)

- Borrowing rvalues (`&E::A(1)`), runtime-indexed places (`&xs[i]`) and
  nested reference borrows (`&&a`) are outside this lane.
- `match x { A => *r += 1, .. }` (compound assignment as an unbraced arm
  expression) is a parser gap.
