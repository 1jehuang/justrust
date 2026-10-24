# Checked callable reference inputs

## Status and frozen scope

Closed calls to checked stored closures accept shared and mutable references inside
the ordinary callable argument tuple when the result has no reference carriers.
This includes scalar-result `Fn(&i32)` and `FnMut(&mut i32)` consumers. It does not
admit reference results, arbitrary projected method references, named function
items, or complete Iterator support.

## Authority and IR contract

- The selected proof must be an exact closed closure implementation in this
  world and the canonical empty closed environment, with the same complete goal.
- The required trait must be the source-authenticated callable trait for the
  selected receiver mode. The closure's parent, source site, provider, ordered
  owner actuals, capture modes, and checked signature remain authoritative.
- Ordinary `closure_target` and `discharge_in` replay the actual checked adapter
  ABI before input admission. Required and provided parameter and result types
  must agree exactly. No guessed tuple layout or root-reference surrogate is used.
- Only parameter 1, the checked argument tuple, receives this capability. Its
  ordered type must equal both the selected trait actual and the checked closure
  arguments. Closed tuple reference carriers retain their structural Field paths
  and fixed reference positions. Unresolved families are not replaced with zero
  carriers.
- Ordinary declaration-derived carrier contracts, projected call evidence, source
  argument evaluation, reborrow effects, and borrow-plan validation remain in the
  execution path. This capability creates no origin rows or return contract.
- A result with any carrier does not receive input-only admission. The general
  projected-reference gate and the explicit closure reference-result boundary
  remain unchanged. Shared inputs cannot be strengthened to mutable inputs.

## Acceptance gates

### Ordinary generic scalar consumers

An ordinary source function whose sole parameter is its sole own type
parameter can also consume an inferred shared or mutable scalar reference when
its instantiated result has no reference carriers.
The source replay independently reconstructs integer, bool and char root types
from original parameters, annotations or typed literals, checks the exact borrow
place and kind, and replays the one-slot
input constraint without result-context inference or numeric fallback. Existing
annotated reference bindings retain their original source type. An inferred
mutable-reference argument is moved, not implicitly reborrowed. Explicit type
arguments retain the normal mutable reborrow behavior. Nested reference
and nominal inputs are not admitted by this rule. All normal callee, lifetime,
effect and ownership checks remain required.

`callable_reference_inputs::ordinary_generic_scalar_reference_inputs_match_stock`
checks shared/mutable inputs and scalar results against stock at workers 1 and 8.
The sibling rejection test covers immutable mutation, live shared aliases, a
missing Copy obligation and a local reference escape. The source replay mutation
test rejects changed borrow kind, place, mutable capability, initializer value
and type, and source annotations.

`tests/callable_reference_inputs.rs` compares stock Rust with native CHECK, BUILD,
and executable results at workers 1 and 8 for real shared and mutable callable
inputs. Invalid controls cover an unused wrong-argument body, shared-to-mutable
strengthening, overlapping mutable arguments, and a borrowed argument escaping
into a captured local.

The source-backed private authority test in `traits/callable_inputs.rs` constructs
a checked parent publication and verifies the exact tuple capability. It rejects
reordered arguments, receiver/argument swaps, stronger borrow kinds, changed
provider result signatures, foreign or universal proof environments, changed
receiver modes, changed source sites, foreign closures, and foreign worlds.

Mandatory promotion gates are those native and authority tests, graph integrity,
and a current-parent all-test compile check. Method-bound equality admission and
generic dot-method inference are separate coordinated slices, not prerequisites
silently bundled with this feature.
