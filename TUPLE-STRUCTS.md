# Native tuple structs: bounded implementation contract

Status: **accepted bounded milestone, October 17, 2026**. Full Rust compatibility
remains open.

Later extension: `GENERIC-TUPLE-STRUCTS.md` admits generic nonempty tuple structs
through exact nominal applications and universal checking. Its retained 36-source
matrix promotes only the original generic constructor witness, without changing
source bytes, order or output. The initial milestone evidence below is historical.
The accepted nominal unit-struct milestone is `e6e9386fd`, documented in
`UNIT-STRUCTS.md`. This milestone must preserve that contract and its tests.

## Positional inference regression repair (2026-10-19)

Body-local inference resolves the full canonical nominal path before treating a
positional pattern as an enum variant. Tuple-struct patterns retain their own
declaration identity, including qualified paths and nested transported values.
The authoritative checker still enforces shape, arity, visibility and nominal
identity, and invalid patterns in unused functions must still be rejected.

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

## Executed acceptance

The stock-first corpus has 36 sources: 19 stock-valid and 17 stock-invalid.
Native classification is 11 runtime, 17 ordinary invalid and eight explicit
Unsupported boundaries, yielding 144 CHECK/BUILD cells at workers 1 and 8.
Debug `20261017-104643024-1684824` and release bootstrap
`20261017-104950079-1698972` each pass 14 public tests, including eleven public
schema mutation/clone controls. Valid BUILD cells execute real native outputs
with exact stdout and exit checks, without compiler fallback.

An unchanged 120-cast test exposed a deterministic ordinary-stack abort. A
temporary same-source public-interface diagnostic measured successful parsing
followed by resolver overflow. Tiny cast dispatch now keeps constructor scratch
outside recursive cast frames, preserving operand-before-type resolution and
all caps. The diagnostic passed `20261017-104919040-1696804`, the original exact
regression passed `20261017-104922214-1696929`, and all 894 library tests passed
`20261017-104926107-1697375`. The temporary diagnostic file was removed, not
substituted for the original regression.

Final all-target check `20261017-105006278-1700363`, complete regression
`20261017-105009592-1700931` (2,209 tests, 110 binaries, zero failed/ignored/filtered),
strict lint `20261017-105023598-1701755`, formatting and diff checks pass.
Independent read-only review found no concrete bounded schema, privacy, source
identity, plain-field projection or frame-dispatch blocker. Sixteen exact
redacted failed/successful summaries are retained in
`data/runs/xps14/2026-10-17-tuple-structs.jsonl`. No full-Rust, public layout or
compiler speedup claim follows.
