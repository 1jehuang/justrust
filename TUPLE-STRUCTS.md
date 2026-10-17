# Native tuple structs: bounded implementation contract

Status: **in progress, not accepted**. Full Rust compatibility remains open.
The accepted nominal unit-struct milestone is `e6e9386fd`, documented in
`UNIT-STRUCTS.md`. This milestone must preserve that contract and its tests.

## Source and semantic authority

Admit nonempty, nongeneric original tuple-struct declarations with explicit
`StructShape::Tuple`. Fields retain actual `FieldName::Tuple(index)` identities,
contiguous from zero, and their declared visibility. No numeric `AstIdent` is
fabricated. Universal public-AST validation must reject malformed shape/index
combinations, including unused declarations, before planning effects.

A source `Expr::TupleStruct { name, fields }` represents the checked constructor
and its ordered argument expressions. Resolver rewriting requires an actual
canonical tuple-struct declaration and a checked original-use certificate.
Constructors are not function declarations or canonical callable bodies.
Type aliases are not callable constructors. Import aliases may select the real
constructor subject to type visibility and every constructor-field restriction.

Source schema `DeclId` and `StructShape::Tuple`, never diagnostic spelling or
runtime field count, authorize construction, positional projection and patterns.
A record's `.0` is invalid. A tuple struct's named projection is invalid.
Arity, nominal identity and field privacy remain mandatory.

## Patterns, ownership and transport

Explicit tuple-struct let patterns bind only their actual child patterns. They
must resolve the full canonical nominal path and use ordinary field projections,
not enum downcasts or primitive-unit patterns. Constructor-name shadowing must
follow independently measured stock behavior before local binding registration.
The existing scoped bare-unit-pattern boundary is not silently widened.

Runtime lowering reuses the checked nominal `StructTy` aggregate representation,
existing ownership paths and private native aggregate transport. No new IR shape
or public layout/FFI claim follows. Structs remain non-Copy absent actual facts.
Argument evaluation is once, in source order. Nested fields, calls, returns and
partial moves must retain the existing checked semantics.

## Explicit initial exclusions

Empty and generic tuple structs, first-class constructor values, constructor
parameter patterns, numeric brace constructors, tuple-struct match destructuring,
repr/derive and general Rust ABI/layout support remain typed Unsupported where
not already implemented. Valid Rust is not relabeled malformed syntax merely to
close this milestone. Record and unit construction retain their separate rules.

## Acceptance gates

- Stock-first pinned sources for construction, positional access, let patterns,
  aliases, shadowing, privacy, wrong arity, nominal mismatch, non-Copy moves and
  ordered side effects. Preserve all prior corpus sources and caps.
- Native CHECK and BUILD at workers 1 and 8 with unavailable fallback compiler,
  real executable stdout/exit checks, and no artifacts for rejected builds.
- Bootstrap debug and release profiles tested distinctly. These are not a claim
  of a native optimization-profile API.
- Public schema mutation controls require Contract errors before applications
  or obligations, with healthy budgets. Exact clones remain positive controls.
- New expression traversal in universal names, depth/work, clone preflight,
  canonical source inventory/replay and all active checker branches.
- Complete hosted all-target check, unchanged full regression, strict lint,
  formatting and diff checks. Retain exact failed and successful run receipts.

No acceptance or performance claim is made until those gates execute.
