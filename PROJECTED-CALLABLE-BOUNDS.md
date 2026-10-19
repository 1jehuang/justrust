# Projected callable bounds

## Status and frozen scope

This slice retains canonical associated projections inside trait type arguments,
including the argument tuple of a stored closure's `Fn(I::Out) -> i32` bound.
The native acceptance source uses a genuinely inferred stored closure and an
explicit `I` actual with an independently inferred callable actual. A second
case proves that an unrelated `Fn::Output = i32` requirement does not invent an
assignment for the caller's independent `I::Out` family.

This is not complete callable or Iterator support. Named function items as
first-class values, reference-bearing callable argument tuples, and closure
reference-result contracts remain separate unsupported boundaries. The full
source Iterator library is not claimed to pass by these focused consumers.

## Authority and lowering contract

- Source resolution authenticates every projected child, resolved trait member,
  and ordered argument before retaining a trait application.
- Header membership discovery closes immutable declared metadata, not executable
  type validation. Canonical predicate identity deduplicates the bounded closure.
  It never introduces a membership from a guessed implementation or a fake
  projection equality. Cyclic invalid trait declarations remain rejected.
- Each open projected argument retains its exact trait, item, self type, ordered
  actuals, and declaration-owned environment. Foreign worlds, item changes, and
  reordered actuals are not equivalent.
- Per-body inference defers only unresolved projection dependencies. Once those
  actual dependencies are known, ordinary sealed normalization supplies the
  concrete type. An unrelated unknown callable slot cannot force a wildcard
  equality or a fabricated type actual.
- Authoritative callable checking reconstructs projected formals from the same
  canonical inference slots and replays actual trait and method obligations.
- Numeric projection inputs default to i32/f64 only when the caller permits
  fallback and all ordinary and currently closed projected constraints have
  quiesced. Later known widths are applied before defaulting. Unknown nonnumeric
  generic slots never receive fabricated defaults.
- Closed carrier equality replay first validates the original occurrence rows,
  normalizes newly closed nested trait actuals with dependency-proof replay,
  and discharges the normalized obligations. The sealed result authority retains
  original source occurrences and reconstructs this operation on validation.
  Root associated items and ordered applications are never collapsed by name.
- Executable residual-projection rejection, reference-carrier restrictions,
  CHECK/codegen separation, resource diagnostics, and graph ceilings are unchanged.

## Acceptance gates

`tests/projected_callable_bounds.rs` compares stock Rust with native CHECK,
BUILD, and executable output at workers 1 and 8. It includes the real untyped
stored-closure consumers for both i32 and u8, independent equality noninterference,
and five invalid unused-body, missing-membership, wrong-owner, and cyclic-supertrait
controls. The original i32/u8 and boolean-mismatch sources also passed the unchanged
Wyvern acceptance harness at workers 1 and 8, receipt
`20261019-123149230-4107123`. A further unchanged source infers `I` from a later
ordinary owner argument before resolving the projected input and untyped closure,
with stock/native CHECK, BUILD, and run at workers 1 and 8.

`tests/projection_defaults.rs` additionally checks i32/f64 projection-receiver
fallback, a later u8 argument overriding an earlier unsuffixed integer, and
contradictory projected values against stock Rust at workers 1 and 8.

Source-backed private authority tests in `traits/universal_projections.rs` cover
an exact projected tuple through an applied supertrait, reordered arguments,
foreign members, foreign worlds, and foreign declaration environments.
`traits/parameter_equality.rs` tests closed carrier row replay, omitted rows and
origins, wrong ordered arguments, wrong owner/item, changed occurrence identity,
and foreign or universal facts substituted for a closed instance.

Required regression gates are those authority tests, the graph-integrity suite,
and an all-target compile check. Full native Iterator consumers remain separately
visible and are not substituted with a smaller library.
