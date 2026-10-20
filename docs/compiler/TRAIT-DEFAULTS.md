# Source trait default method bodies

Source-provided trait methods are executable source declarations, not catalog
recipes or permission attached to a signature-only trait item. Each default has
a separate callable identity, an empty method binder and the enclosing trait's
`Self` binder. Its semantic path retains the trait owner, independently of the
containing module used for name lookup and privacy.

## Checking and execution

Every provided source body enters the ordinary universal source-body, CFG,
initialization/move and borrow checking pipeline, even when there is no impl or
all implementations override it. The callable environment includes the owning
`Self: Trait` requirement and the supported declared bounds. The trait world
validates the provider's owner, receiver, ordered parameters, exact signature
and equivalent closed obligations before publishing callable facts. Signature
items themselves remain nonexecutable. Trait `Self` is not implicitly `Sized`.
Executable by-value parameter/result positions require an actual `Sized` proof,
for example from a declared supertrait, while borrowed receivers do not gain an
invented bound. Direct or transitive declared Sized supertraits also permit
`Self` inside tuple and fixed-array types, including empty arrays. Truly unsized
aggregate positions remain rejected. Graph acceptance additionally requires
invalid default moves and loans to reach failed ownership analysis rather than
being rejected by an unrelated unsupported-type fence.

Omitted impl methods select their provided callable with the actual concrete
`Self` argument. Explicit implementations take precedence. Calls in generic
bodies remain deferred trait-item calls until instance selection, so a default
can call required methods or other defaults and preserve per-implementation
dispatch. Closed native instances still need a real implementation satisfying
the owning trait. The universal assumption cannot manufacture an impl.

Source-body recipe discovery includes defaults, including unused bodies.
Canonical catalog placeholders that have no executable recipe remain unsupported.
This feature does not make arbitrary catalog-provided methods executable.

## Acceptance scope

The executable conformance corpus compares stock Rust and native CHECK/BUILD at
workers 1 and 8, with bounded execution, ELF checks and no executable publication
on failure. It covers omitted/default dispatch, override precedence, generic
callers and impls, multiple implementations, recursion, static and borrowed
receivers, default-only float/tuple Clone recipes, and invalid unused or overridden
bodies. Trait-world tests separately exercise provider metadata validation,
concrete substitution, deferred calls and missing-impl rejection. Instance tests
check distinct trait owners and declaration-discovery-order independence.

The existing restrictions on trait declarations, method generics, associated
types, reference-containing generic values and broader lifetime behavior remain
in force. This is a bounded trait implementation milestone, not complete traits,
standard-library support or full Rust support.
