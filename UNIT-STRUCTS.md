# Native nominal unit structs

Status: implementation in progress after item-macro checkpoint `baa55be5a`.
This document is a bounded contract, not an acceptance or full Rust claim.

## Source and nominal identity

Admit nongeneric `struct Marker;` declarations as their actual retained
`StructShape::Unit`, distinct from `StructShape::Record`. Bare and braced
constructors, import aliases, ordinary let/match unit patterns and existing
aggregate composition must resolve through the exact retained source schema and
DeclId. An empty runtime field list is not constructor authority. A type alias
is a type, not a bare value constructor. `Marker()` is not a function call.
Empty record declarations remain a separate Unsupported boundary.

Resolved types remain `Ty::Struct` with their nominal application and declaration
identity, never primitive `Ty::Unit`. Unit structs are not implicitly Copy.
Ordinary bindings and call arguments move them. A bindingless unit pattern or
wildcard does not move its scrutinee merely because it was inspected. Pattern
lowering may use an existing checked wildcard only after exact nominal domain,
source shape, visibility and exhaustiveness checks, not as a typing shortcut.

## Explicit private carrier

The private Linux x86-64 ABI pads every zero-field nominal Struct carrier to one
initialized eight-byte dummy word. It keeps the existing aggregate snapshot
pointer argument and sret return convention. Frame allocation, copies, nested
field/array stride and result transport use that same carrier size. Construction
initializes the word before a snapshot is exposed. References point into actual
owned frame storage, not a null or shared mutable sentinel.

This internal carrier does not claim Rust `size_of`, public representation,
extern ABI or general zero-sized layout compatibility. There is no separate
native optimization profile exposed by `jr-rust`; debug/release acceptance here
means executing the compiler and harness built in both bootstrap profiles.

## Scoped pattern boundary

The current parser eagerly issues lexical BindingIds for bare pattern names in
functions with macro scopes. Reclassifying such a candidate as a constructor
later cannot erase its already-observed frontier associations. This slice must
reject that ambiguous bare unit pattern with typed Unsupported before publication.
Qualified and braced unit patterns can use their genuine nonbinding syntax.
General declaration-aware binder candidate finalization remains open work.

Generic unit declarations, empty record declarations, public layout/FFI claims
and general pattern or borrow semantics remain outside this bounded milestone.
No resource limits, stack environment or existing source corpus may be weakened.

## Required real gates

Measure stock source cases before native expectations. Exercise bare/braced and
alias construction, nominal mismatch, invalid tuple calls, move failures versus
bindingless reuse, privacy and textual lookup. Execute native CHECK and BUILD
with workers one/eight, exact executable output and no stock compiler fallback.
Include interleaved scalar/unit arguments and returns, nested aggregates, arrays
and existing shared/mutable reference transport. Run the same new matrix with
debug and release bootstrap binaries, plus complete unchanged regression,
all-target check, strict lint, formatting and exact scoped receipts.

Public malformed source-schema controls must fail before planning effects while
an exact same-owner clone remains valid. Private CFG/ownership controls must
retain the nominal root move bit for a zero-field product and reject invalid
field projections and arity, not make Copy depend on vacuous field iteration.
