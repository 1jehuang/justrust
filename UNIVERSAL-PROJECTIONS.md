# Universal associated projection families

## Status and contract

The canonical compiler retains rigid associated types in universally checked
callables, provided trait methods and owning impl assignments. This is not a
claim of full Rust or complete Iterator support. Closed executable instances
must normalize all residual projections before layout, drop or emission.

An open projection identifies its resolved trait, declaring associated item,
Self type and ordered actuals in the exact authoritative declaration environment.
Membership is replayed from immutable declaration predicates. Raw syntax,
foreign worlds, changed binders, missing bounds and identical-looking foreign
identities do not authorize execution.

## IR and ownership

`CarrierFamily::Projection` and `CarrierChannel::ProjectionTail` preserve an
opaque family rather than inventing a parameter, layout or empty origin set.
`CarrierContractView::family_actual` distinguishes an authenticated open family
from a closed scalar with genuinely zero carriers. Substitution records only
final canonically normalized nominal identities and replays normalization
proofs. Closure children and dyn actuals/equalities retain their own identities.

Open values are conservative non-Copy ownership leaves unless an independent
canonical Copy proof succeeds. Moves preserve exact family origins. Pattern
equality needs sealed universal facts and canonical type comparison. Structural
schema substitution never grants raw executable authority.

## Acceptance

`tests/iterator_universal.rs` compares CHECK, BUILD and native execution with
stock rustc at workers 1 and 8 for a rigid associated identity and an Iterator
provided `last` method. Invalid unused bodies, absent membership and an unused
double move reject under CHECK and BUILD at both worker counts.

Focused projection tests cover family identity, open versus closed carrier
realization, wrong ordered actuals, foreign item/trait/world/caller authority,
pattern admission, ownership and prior closed-projection regressions. Graph
integrity and full-program gates remain mandatory and may not be loosened.
Real library map/filter/collect and generic IntoIterator consumers are separate
integration gates, not implied by the minimal provided-method acceptance.

The ordinary `impl<I: Iterator> IntoIterator for I` assignment
`type Item = <I as Iterator>::Item` now preserves an authenticated universal
family. The unchanged concrete loop oracle passes stock/native CHECK, BUILD and
execution at workers 1 and 8. Assignment validation rejects residual projections
in Schema/Closed contexts and rejects foreign worlds and changed ordered actuals.
Applied associated declaration bounds in a generic loop remain a separate gate.
