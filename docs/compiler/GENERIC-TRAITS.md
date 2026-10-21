# Generic trait applications and checked obligation transport

Status: accepted bounded implementation on October 17, 2026. Full Rust
compatibility remains open.

## Method-argument inference composition (October 19, 2026)

Generic dot calls infer only the selected member's own method parameters. The
existing bounded receiver-depth, identity/autoref, inherent-before-trait and
receiver-mode search is shared with explicit method calls. Inference runs at
that exact candidate, never by separately stripping references or choosing the
first trait by spelling. An applicable but unresolved inherent candidate cannot
fall through to an unrelated trait candidate. Trait applications retain their
canonical declaration, Self and ordered owner actuals independently of method
actuals. Ordinary final selection still proves method arity, bounds, signature
and source authority before a call is constructed.

`generic_dot_methods`, `method_owner_inference`, `explicit_method_owner` and
`trait_method_callable_bounds` compare real CHECK/BUILD/run with stock at workers
1 and 8. Invalid unused calls retain wrong-owner/type/order/arity/bound and
ambiguity rejection. Canonical publication controls reject forged method actuals,
receiver operands and foreign binder handles. Trait method equality bounds have
an exact retained trait/item/module source context, while trait-header and
supertrait equality restrictions are unchanged.

Shorthand `I::Item` copies the selected bound's exact trait occurrence and ordered
type/region arguments into its qualifier, but not the bound's equality list.
The original bound retains every equality obligation. Explicit invalid qualified
equality syntax and unsatisfied or ambiguous bounds remain rejected
(`shorthand_bound_equalities`). Full native Iterator library activation remains
separate from these compiler helper gates.

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

A constructor used as a method receiver may use its existing standalone numeric
literal fallback to establish Self. This does not infer its type from method
arguments, return context or candidate impls, and does not change contextual
constructor evidence used by enclosing calls.

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

## Executed acceptance

The stock-first corpus retains 28 sources: 13 stock-valid and 15 stock-invalid.
Native outcomes are 12 runtime, 14 invalid, one stock-valid unsupported and one
stock-invalid unsupported. The two unsupported cases require choosing among
multiple trait applications without explicit UFCS, not erased arguments or a
first-impl choice. This boundary retains the queried trait declaration even when
selection returns a concrete implementation rather than a trait-item call.
Both early argument inference and final method selection preserve the same
explicit unsupported outcome. Explicit UFCS remains the supported way to select
one application; this does not claim general trait-argument inference.
Workers 1/8 CHECK/BUILD execute 112 cells and 24 real ELF
images per bootstrap profile, with compiler fallback disabled. Twenty-six public
authority tests retain original facts through argument, owner, binder, signature
and source mutations. The historical generic-trait source is promoted unchanged.

Final hosted gates pass all-target checking, 1,076 library tests, strict all-target
Clippy, formatting, and the complete unfiltered 2,740-test / 134-binary suite.
The release bootstrap gate passes 126 tests across the new corpus/authority and
retained trait/default/associated-item suites. Failed and passing receipts are
retained in `data/runs/xps14/2026-10-17-generic-traits.jsonl`.

Integration found and repaired a validated-facts mutex reentry, native instance
keys that omitted trait actuals, and recursive parser stack growth from an inline
generic trait-call payload. Complete application identity is encoded in version-4
native semantic keys; the parser now fallibly boxes that payload without raising
nesting limits or test stacks. Original nesting witnesses and isolated routes
pass. These are correctness findings, not measured compile-speed claims.
