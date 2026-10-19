# Primitive associated calls and byte-to-char conversion

Status: implemented bounded slice. All 21 focused source, authority, graph and
unchanged graph-integrity gates passed on 2026-10-19. Final runtime/universal
projection parent integration passed `check --tests`, three call-authority tests
including sealed projection/foreign-environment controls, and all 14 unchanged
integrity gates.

## Frozen contract

Unqualified primitive type syntax can own an associated call after ordinary
source type namespace lookup finds no binding. Integer and float types, bool,
char and str use the same canonical `Type` representation as type annotations.
Existing modules, aliases, imports, ambiguity and visibility are resolved first.
As in stock Rust, an empty `mod char {}` permits unqualified `char::from` to
retry primitive syntax only after the complete source value path misses. An
existing module member wins, including wrong-kind members that must diagnose.
Qualified module paths are not reinterpreted as primitive syntax. Alias owners
are expanded by existing canonical type resolution and cycle/argument checks.

This syntax grants no method authority. The existing associated-call checker
selects the actual trait declaration and implementation, validates arguments
and result types, and retains ordinary source-call evidence. There is no
compiler recognition of a method named `from`.

The embedded ordinary source library declares `std::convert::From<T>` and
reexports that declaration through the prelude. Its `From<u8> for char`
implementation returns `value as char`, reusing the checked lossless byte cast.
Every value 0..=255 maps to the same Unicode scalar, including NUL and Latin-1
values 128..=255. This operation is not UTF-8 decoding. No non-byte integer,
reference or bool conversion is invented. Other From/Into/TryFrom implementations
and complete Rust standard conversion library support remain separate work.

Implicit primitive associated calls requiring argument coercion to a preselected
trait actual are explicitly Unsupported until argument-owned trait inference is
implemented. In particular, `char::from(loop {})` cannot silently select
`From<u8>` when stock requires `From<!>`. Explicit
`<char as From<u8>>::from(loop {})` has genuine fixed trait arguments and keeps
the existing Never coercion. This boundary is independent of method spelling.

Checked call IR also validates its stored result and post-coercion argument
types against the already authenticated canonical instantiated signature, using
bounded exact type comparisons. Universal projection operands are authenticated
in the exact sealed environment before world-owned canonical equality. Same-width types, foreign callees and invented
ordered actuals cannot substitute for that authority.

## IR, storage and graph shape

Primitive syntax lowers to the existing `AssociatedCall` AST with its exact
primitive owner. Trait selection emits the ordinary checked call to the source
implementation. Its existing integer-to-char cast and native scalar layout are
unchanged. No IR variants, intrinsics, node kinds, shared mutable facts,
whole-program joins, storage conventions or graph integrity baselines are added.
The conversion source is one ordinary immutable per-file prelude graph input.
CHECK does not demand object generation or linking.

## Acceptance gates

- `tests/char_from.rs`: unchanged stock/native CHECK/BUILD/run sources at workers
  1 and 8, all 256 bytes, direct calls, std/core UFCS, generic bounds, aliases,
  independent primitive associated methods, foreign trait/module shadows,
  invalid source conversions and unused invalid bodies.
- `tests/char_from_authority.rs`: forged call result/argument types reject under
  the original validated callable facts and CFG publication checks.
- `tests/char_from_graph.rs`: ordinary prelude source graph inputs and no codegen
  demand during CHECK at workers 1 and 8.
- Unchanged `tests/graph_integrity.rs` must pass before publication.
- Registry worker probes unchanged nix-base32 0.2.0. Clearing char::from lookup
  is a bounded library milestone, not a claim that the whole crate compiles.
