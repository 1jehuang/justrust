# Source type aliases

Source type aliases are nonnominal declarations. Their targets expand into the
existing canonical type representation, rather than introducing a new nominal
identity, implementation, callable or native ABI. Monomorphic and first-order
type-parameter aliases compose supported scalar, tuple, array and nominal types.

Targets resolve in the alias declaration's module and type-parameter context.
Use-site arguments are substituted simultaneously, so a caller parameter with
the same spelling as an alias parameter is not captured. Imports and named
reexports preserve the alias binding's lexical visibility while target nominal
and field identities remain those of the original declaration.

Forward aliases are permitted. Unknown targets, invalid generic arity, unused
generic parameters and recursive aliases are rejected even when unused. Active
alias identities detect parameter-growing cycles. An iterative dependency
schedule resolves cached templates before consumers, so forward chains do not
recurse on the native stack. Expansion and cached target clones share a bounded
resolver ledger and depth limits, rather than starting a fresh allowance for
every recursive application.

Alias constructor and pattern paths must retain fixed and substituted target
generic arguments. Canonicalizing a name must not turn an alias for `S<i32>`
into unconstrained `S<T>`. Local values and same-spelled functions retain the
ordinary value namespace and shadowing rules. A type alias does not create a
tuple-constructor function or a module namespace.

## Acceptance and remaining boundaries

Executable stock-Rust cases are classified before native CHECK/BUILD at workers
1 and 8. Graph tests compare direct and aliased native image bytes and require
unused invalid declarations to reach semantic validation. Real filesystem
acceptance exercises generic aliases whose definition uses an imported target
in another external source file.

Omitted generic arguments in alias constructors and patterns require additional
context-driven inference and remain explicitly unsupported. Fixed and
explicitly substituted pattern arguments are preserved. Alias bounds, where
clauses, default parameters, lifetime/const alias parameters, associated forms,
unsized aliases and underlying type forms outside the existing compiler contract
remain explicitly unsupported. Existing nominal-bound declaration restrictions are
not alias conformance evidence. Stock Rust's deferred alias obligations must
not be replaced with invented eager generic-bound failures. Full Rust support,
source maps, globs and full cross-crate/edition behavior remain separate work.
