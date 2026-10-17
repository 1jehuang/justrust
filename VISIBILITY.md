# Restricted visibility

Supported declaration kinds accept `pub(crate)`, `pub(super)`, `pub(self)` and
`pub(in path)`. Visibility is normalized to either unrestricted public access or
a canonical lexical module subtree. An empty restricted scope is crate-local,
not externally public. Module segment boundaries matter, so `a::b` is not an
ancestor of `a::bc`.

Scope paths must identify the declaring module or an actual lexical ancestor.
Aliases, sibling modules, descendants and missing modules cannot manufacture
visibility authority. Invalid scopes are checked even on unused declarations.
Explicit visibility remains distinguishable from inherited private syntax so
trait items, trait implementation methods and enum fields cannot gain illegal
visibility qualifiers through normalization.

The same normalized facts govern resolver access, named aliases, public and
restricted reexports, nominal types, fields and inherent method calls. Alias
visibility cannot widen its original declaration or an intermediary binding.
Mixed type/value namespace imports retain each namespace's effective scope.
Canonical declaration and executable identities are unchanged by aliasing.

IR nominal/field definitions and declaration schema metadata carry explicit
visibility rather than a public boolean. Trait implementation callable metadata
must retain the exact private scope of its lexical implementation owner.
Catalog metadata uses version 3. Retained scope strings participate in the
existing bounded source clone/publication accounting.

## Acceptance and limits

Independent executable stock-Rust fixtures precede native CHECK/BUILD at workers
1 and 8. Graph tests cover canonical image equality, nested trait implementation
owner scopes and unused invalid declarations. Real filesystem tests compare
stock/native behavior through nested external module, type and field scopes.

This is not full Rust support. Type aliases remain a separately identified
legal-but-unsupported declaration form, even when their visibility is valid.
Globs, block-local items, full cross-crate/edition resolution and source maps
remain separate open work. Existing type, ownership and runtime restrictions
continue to apply.
