# Reference-typed Self in generic trait methods

Status: implemented for SIZED ordinary actuals. Not full Rust support.

## Frozen contract

Supported: a trait method with its own type binder implemented for a
reference Self, `impl<T: Tr> Tr for &T { fn m<S: B>(&self, s: &mut S) }`,
where every generic actual (T, S, owner args) is sized. Calls through `&T`,
`&&T` and on T itself behave as stock Rust.

Mechanism: `ValidatedTraitWorld::validate_impl` compares an impl method with
the trait item's expected signature. For a method with its own binder that
expectation comes from `alpha_rename_method_schema`
(`src/compiler/traits/generic_methods.rs`). After all of its existing checks
(goal ref, owner/arity, per-actual metadata, raw instantiation, row counts,
raw-contract equality, RHS substitution for every param and the result,
predicate substitution), it now realizes the reference contract for the full
ordered owner + method actuals with the existing `realize_ref_contract` kernel,
the same one ordinary call instantiation (`instantiate_in`) already used.
Per-layer rows come only from the item's own signature-origin tails; fixed
rows and outlives edges are preserved; no lifetime rows are invented. No IR,
fact, graph node, contract validator or lifetime-elision rule changed.

## Explicit boundaries (tested, not supported)

| Boundary | Observed native result | Test |
|---|---|---|
| Unsized generic actual, `T := str` behind `impl<T: ?Sized + Tr> Tr for &T` | rejected: `slice generic arguments are unsupported` (`ir::reject_references` on bare `str` as a generic actual) | `unsized_str_generic_actual_is_an_explicit_boundary` |
| Method returning `&Self` with Self = `&T` | rejected: `invalid noescape reference contract: reference result position coverage mismatch` (impl's own source contract, `canonical_ref_contract`, gives the `&Self` result one row while `&&T` has two) | `reference_self_returning_self_is_rejected_out_of_scope` |
| Borrow-expression receiver on a reference method actual, `let r = &v; (&r).feed(&x)` | rejected: `unsupported unresolved receiver type` (stock accepts) | `reference_method_actual_borrow_receiver_is_rejected_debt` |

The second and third are DEBT, not part of this slice. The second is in
source-contract result elision for reference Self; its stock-E0515 control is
rejected for that reason, so it is not evidence of lifetime soundness. The
third is receiver resolution of a borrow expression. The DISTINCT accepted
source `let rr = &r; rr.feed(&x)` (typed-local receiver,
`reference_method_actual_typed_receiver`) is a workaround shape, not a fix of
the original source, which stays rejected.

The borrow-receiver rejection was observed only on the candidate tree. No
baseline (unpatched) run of it exists, so it is not claimed to be pre-existing.

## Gates

- `tests/reference_self_generic_methods.rs`: stock-differential CHECK/BUILD/run
  at workers 1 and 8 for sized `&T`/`&&T` forwarding, zero-layer Self and a
  reference method actual; plus the two boundary tests above.
- `tests/reference_self_generic_method_authority.rs`: real validated-facts
  path; forged inner-row drop, raw single-layer contract, and inner/outer
  region alias on the retained `&T` trait call are rejected by checked-IR
  validation and CFG ingress.
- Regression: `generic_trait_methods`, `graph_integrity`. `trait_ref_self`
  has a known failing case, `dyn_dispatch_and_inferred_usize_index`
  (E0308 on `[&a, &b]: [&dyn Shape; 2]`), that also fails on unpatched
  e14d42ddf with nightly-2026-10-08 (rustc 1d81eb4ad), run
  20261019-224129135-2538528. It is a baseline failure outside this slice.
  Neither of its trait methods has its own binder, so the changed function
  is not reached for it.

## Evidence log

- Region-contract rejection on the original Feed probe removed (binary
  0bda35d4); the probe then stops at the unsized boundary.
- `sized_reference_self_forwarding`: stock and native CHECK/BUILD/run at
  workers 1 and 8, output `[7, 1000, 7, 7]` (run 20261019-221044441-2394737).
- Unsized boundary test: passed in the same run.
- `zero_layer_self_unchanged` (output `[2, 3]`): passed in run
  20261019-222108087-2434789 (that run's other case was superseded by the
  split typed/borrow receiver tests).
- `reference_method_actual_typed_receiver` (output `2`): run
  20261019-222639900-2458410.
- Authority 4/4: run 20261019-222759980-2463463.
- Borrow-receiver debt rejection: run 20261019-222913993-2468860.
- Toolchain caveat: the receipts above ran on the pinned jr-ac20de5e7921
  toolchain, not nightly-2026-10-08 (1d81eb4ad). An earlier
  corrected-toolchain attempt tested a stale unpatched engine (preserved file
  mtime) and is void.
- Corrected-toolchain chain on parent e14d42ddf, `JUSTRUST_TOOLCHAIN=nightly-2026-10-08`
  (rustc 1d81eb4ad), local `-j1`, after touching the changed source; the
  check and refself stages report "Rebuilt because: edited
  src/compiler/traits/generic_methods.rs (justrust-engine)":
  `check --tests` ok (20261019-224802669-2588215);
  `reference_self_generic_methods` + `reference_self_generic_method_authority`
  10/10 (20261019-224813789-2591816); `generic_trait_methods` 2/2
  (20261019-225109155-2608592); `graph_integrity` 14/14
  (20261019-225530345-2631617).
- Not run on the corrected toolchain: `trait_ref_self` as a whole file (its
  dyn-array case is the documented baseline failure), the 20-program corpus.
