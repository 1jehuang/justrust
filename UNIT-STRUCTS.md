# Native nominal unit structs

Status: bounded native semantics accepted after item-macro checkpoint `baa55be5a`.
Final hosted all-target check `20261017-103752636-1644345`, complete regression
`20261017-103755217-1644600` (2,179 tests, 108 binaries, zero failed/ignored/filtered),
strict all-target lint `20261017-103809005-1645706`, formatting and diff checks pass.
General unit patterns, Rust layout/FFI and full Rust remain open.

## Source and nominal identity

Admit nongeneric `struct Marker;` declarations as their actual retained
`StructShape::Unit`, distinct from `StructShape::Record`. Bare and braced
constructors, import aliases, ordinary let/match unit patterns and existing
aggregate composition must resolve through the exact retained source schema and
DeclId. An empty runtime field list is not constructor authority. A type alias
is a type, not a bare value constructor. `Marker()` is not a function call.
Empty braced record declarations are also admitted by the bounded October 20
extension below; empty tuple declarations remain Unsupported.

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

Generic unit declarations, public layout/FFI claims and general pattern or borrow
semantics remain outside this bounded milestone.
Constructor patterns in function parameter positions retain the existing
Unsupported boundary rather than being silently normalized into bindings.
No resource limits, stack environment or existing source corpus may be weakened.

## Empty braced records (October 20, 2026)

`struct Marker {}` retains `StructShape::Record`, not unit constructor authority.
`Marker {}` construction and braced patterns use the existing nominal record
checker and the same initialized private empty carrier. Bare `Marker` and
`Marker()` remain invalid. Distinct empty declarations have distinct nominal
identities, and zero fields do not grant Copy: ordinary moves, explicit valid
Copy implementations, references and bindingless patterns retain their existing
checked behavior. Empty records with unused type or lifetime parameters are
ordinary invalid declarations, including when unused; lifetime bounds cannot
make a fieldless record's lifetime used. Empty tuples and empty-record C ABI
transport remain outside this extension.

A construction-final immutable source receipt retains each declaration's module,
canonical path, exact shape and ordered field identities. Public AST clones
cannot change a unit into a record, erase nonempty fields or change nominal
identity and acquire planning authority. Capture charges its traversal and actual
storage before publication; no new graph node, shared mutable state or graph
ceiling is introduced. Existing IR/CFG validation and initialized native dummy
storage are reused rather than bypassed.

The final release-bootstrap oracle verifies stock rustc 1.98.1, commit
`48a229ceaefd4985c50990b14116b6d856af0985`, before compiling any source. The
extension matrix passes 21 stock-first cases and 84 native workers-1/8
CHECK/BUILD cells, with 12 executed ELF images: six runtime cases, ten ordinary
invalid cases (including unused bodies and E0392 type/lifetime parameters), and
five explicit boundaries (empty tuple plus active/unused direct/nested C ABI).
The original 26-source matrix also passes all 104 cells and 40 ELF executions
after promoting its unchanged empty-record source: 188 cells and 52 native ELF
executions in total. Seven source-shape authority controls pass, including
empty-record-to-unit/tuple forgeries. CHECK graph evidence requires no demanded
OBJECT, BUILD or MONO_PLAN, not merely no published output. CHECK and BUILD
capture must be complete, and BUILD analyses must be complete and pass.

Pinned matrix receipt: `20261020-054624710-4059683`. Final graph-demand and
source-authority receipts: `20261020-055058021-4066720` and
`20261020-055115541-4067125`. Graph integrity passes 14/14 with unchanged ceilings
(`20261020-054617323-4059725`), and strict all-target lint passes
(`20261020-055116132-4066619`). Earlier real failures are retained: an incorrectly
accepted empty C ABI layout (`20261020-053254268-4039575`) and an incorrectly
accepted unused lifetime (`20261020-054600759-4059226`). Their production fixes
preserve invalid/boundary classifications rather than weakening expectations.

## Executed real gates

The stock-first source corpus has 26 cases: 22 stock-valid and four rejected.
The native matrix has 104 workers-1/8 CHECK/BUILD cells: nineteen runtime cases,
four ordinary invalid cases and three explicit Unsupported boundaries. Debug
bootstrap run `20261017-103640031-1639251` and release bootstrap run
`20261017-103645840-1639250` each pass all eight public tests, including five
same-owner source-schema authority controls. Each matrix executes all 38 valid
BUILD outputs with exact stdout/exit and nonexistent compiler fallback paths.
Release private controls pass nineteen tests in `20261017-103553693-1635941`.

The full unchanged gate includes every earlier source-macro, alias, aggregate,
reference, integer and ownership corpus. Intermediate failures included a stale
resolver method name, two ParsedProgram field accesses, an obsolete zero-plan-
applications assertion for a now-real nominal application, a version-sensitive
stock diagnostic phrase and an assertion expecting a resolver boundary during
syntax parsing. Repairs retain exact sources, 26-case classifications and all
104 native cells. The stock error is pinned to E0423, with actual compiler -vV
recorded. The scoped bare pattern must parse then fail at resolution, not fabricate
a syntax failure. Fourteen exact redacted receipts, including failures, are in
`data/runs/xps14/2026-10-17-unit-structs.jsonl`.

The following requirements describe the executed bounded contract, not general
zero-sized or arbitrary pattern compatibility.

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
