# Grouped module imports

Grouped `use` syntax is normalized into the existing identity-based import
resolver rather than treated as textual source replacement. Nested groups,
trailing commas and named aliases retain the containing source module. Ordinary
named leaves retain their admitted type/value namespace behavior. A terminal
`self` leaf imports only the type namespace, including when aliased.

Empty groups do not silently erase unresolved or inaccessible prefixes. Their
prefixes are retained and checked even when they introduce no aliases. All
unused import bindings still undergo canonical target resolution, privacy,
cycle and duplicate-namespace checks before callable signature publication.
Grouping cannot make a private item public or bypass an existing import cycle.

Use-tree traversal has separate depth, work and retained-path bounds. The
current per-declaration limits are depth 128, 4096 tree/segment work units and
1 MiB of logical path/name bytes. These are explicit expansion proxies, not
allocator/RSS measurements. Whole-program expansion is separately limited to
65,536 leaves/empty prefixes and 16 MiB of logical target/alias/module bytes,
checked before final retention. Limit diagnostics are explicit, not partial
successful imports. Existing whole-source limits remain independent.

## Validation

Independent executable stock-Rust fixtures and native CHECK/BUILD runs at
workers 1 and 8 cover admitted grouped import behavior and invalid programs.
Filesystem acceptance includes grouped imports across external source files and
a subsequent external module declaration, ensuring use-group braces do not
change source-loader module boundaries. No external compiler participates in
native compilation; stock Rust is only the test oracle.

This milestone does not implement glob resolution, reexports, restricted
visibility, anonymous `as _` imports, external-crate namespace/edition behavior or
all source maps.
Existing generic, trait, data and expression boundaries continue to apply.
Full source organization and full Rust support remain open in IMPLEMENTATION.md.
