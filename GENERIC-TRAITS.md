# Generic trait applications and checked obligation transport

Status: pending implementation acceptance. Full Rust compatibility remains open.

## Source and binder contract

Admit existing reference-free type parameters on source traits and genuine typed
trait applications in impl heads, bounds, supertraits and explicit UFCS. Reuse
`TraitPath::Source(GenericPath)`: each actual type belongs to its original path
segment and the checked canonical original-use certificate remains authoritative.
Builtin trait identities stay closed and require zero arguments. Trait imports
name declarations, never partially applied trait values. Only the final trait
segment may carry arguments in this slice.

`TraitDecl` retains genuine `type_params` and `bounds`. The authoritative trait
binder orders Self at index zero, followed by original source parameters. The
existing total parameter cap includes Self and is not increased. Item/default
schemas retain that exact owner scope. Method-generic parameters, lifetime/const
trait arguments, associated types, Iterator/for and new argument inference remain
outside this milestone. Non-Self reference-bearing trait actuals remain an
explicit unsupported boundary, not silently erased topology.

## Proof and dispatch contract

The existing `ir::TraitRef { declaration, self_ty, args }` is the single complete
application identity. All arguments are checked even in unused or malformed
applications. Source lowering, assumptions, goal keys, coherence, selection,
required signatures, defaults, concrete targets and sealed evidence retain the
same declaration, Self and ordered actuals. Never reconstruct an empty argument
list from a declaration-only lookup at a proof boundary.

Coherence and impl matching unify Self and all trait actuals with one shared
substitution. The same Self may implement the same trait with distinct concrete
actuals. Impl parameters may be constrained by Self or trait actuals, not merely
by predicates. Supertrait and trait-parameter bound implications substitute the
complete owner binder, including the implication subject type, before proving
requirements. Only canonical builtin Copy/Sized identities grant their existing
permissions.

Required/default method instantiation uses the exact full [Self, actuals] vector.
Default provider selection, signatures and body instances must agree on that
vector. Existing region/receiver topology and universal body checking remain
mandatory. A favorable concrete caller cannot validate a bad generic body.
Explicit UFCS carries its written trait actuals; dot selection considers complete
checked applications from authoritative impls/assumptions and must be unique.
No first-impl choice, printable-name permission or new implicit trait-argument
inference is admitted.

## Resources and acceptance

Keep existing name/module/source owners, world snapshots, ParamIds, obligation
phases and proof provenance. Charge actual argument vectors, substitutions,
candidate scans and comparisons before work/allocation, retain actual capacities,
and release backing before leases. Existing limits stay unchanged. Resource or
allocation failure must not become NotProven, ambiguity, successful dispatch or
an unsupported fallback.

Freeze stock-first same-Self/different-actual UFCS, rigid generic bound/impl,
provided default, supertrait forwarding and universal move witnesses. Include
wrong arity/type, missing obligation, overlap, ambiguity, unused bad bodies and
foreign argument/binder/proof replay controls. Require workers 1/8 CHECK/BUILD
without compiler fallback, real ELF execution in debug/release bootstrap profiles,
strict lint/formatting and complete unfiltered regressions. Preserve historical
source witnesses byte-for-byte when promoting an exact newly admitted outcome.
No Iterator, associated-type, full-trait-system or full Rust claim follows.
