# Named public reexports

Named `pub use` declarations use the same canonical item resolver as ordinary
imports. Grouped named leaves and terminal type-only `self` leaves retain their
existing namespace rules and expansion limits. A public alias does not create a
new function, nominal type, trait or implementation identity.

The resolver tracks whether each resolved binding may be publicly reexported,
separately from whether a requester may access it. Public aliases must preserve
both original-declaration and intermediary-binding visibility. Every unused
reexport is checked before callable signatures or executable publication, so an
unused alias cannot promote a private declaration. A same-spelling import can
bind both namespaces. If only one original is public, only that namespace is
exported, while the private binding remains usable from its owner. Enum variants
inherit export eligibility from the enum, rather than their ordinary in-crate
variant accessibility. Root-module aliases are exportable, but private `self`
and `super` modules cannot be promoted by aliasing.

A public item can be exposed through an alias even when its containing module
is private, where stock Rust permits that declaration. The alias does not make
unexported siblings or private fields public. Canonical callable/type authority
continues to refer to the original declaration and its original owner context.

## Validation

Independent executable stock-Rust conformance precedes native CHECK/BUILD runs
at workers 1 and 8. Graph tests compare native output bytes for direct calls and
public alias calls, and reject unused public aliases of private declarations.
Filesystem tests cover public function/module/type reexports from a private
external source module.

Restricted visibility is extended in VISIBILITY.md. Globs, anonymous imports,
full external-crate and edition
resolution, source maps and the rest of full Rust remain separate open work.
