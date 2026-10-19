# Original-source item macros and checked generated functions

Status: this bounded original-source item client is accepted after nested-source
checkpoint `9d7356315`. Final hosted all-target check
`20261017-102443388-1584603`, complete regression
`20261017-102446184-1584828` (2,156 tests, 106 binaries, zero failed/ignored/filtered),
strict all-target lint `20261017-102503349-1585919`, formatting and diff checks pass.
This is not general item-macro or full Rust support.

The zero/many-function follow-up below is independently validated on detached
checkpoint `942b537dc`. Its scoped gates do not substitute for the coordinator's
composed acceptance gate or imply that the frozen primary checkout was changed.

## Admitted client

Original module-level macro_rules definitions may be selected by later
unqualified item-position calls. Each invocation may emit zero or more
non-generic free functions in the existing supported function grammar. Each name
may be captured or generated. Its parameters, local bindings and uses must retain
real checked identities, including captured-root versus generated-marked names.
The existing ordered tt/ident/literal engine remains the matcher authority.

Source textual scope is separate from module item hoisting. Entering an inline
module captures the actual parent macro frontier at that point. Later parent
definitions cannot retroactively become visible. Child definitions shadow only
where their checked textual frontier is visible. A selected definition and its
frontiers are sealed before expansion, with no retry of an older definition on
matching or transcription failure.

Generated attributes, modules, imports, traits, impls and macro definitions are
outside this slice. So are exports, path-qualified macro calls and user macro
calls inside emitted bodies. Existing builtin println remains its supported
path. Legal but unadmitted output must be Unsupported,
not fabricated invalid-Rust diagnostics or stock fallback.

## Separate module proof and function authority

A module item expansion must not borrow a dummy function scope or manufacture a
function-bound expression admission proof. Checked module textual frontiers and
a private ItemExpansionAdmission retain the actual source module owner, name
session, budget, selected definition/call points and expansion record.

Token ingress may share a private Expression/Item proof sum only for checked
span/context transport. Every generated occurrence belongs to the current
expansion. The bounded original-definition context remains Root or one
semi-transparent mark with Root parent. Public root token admission is unchanged.

Each parsed function receives a real fresh function scope. Its association seals
this expansion, exact function-name occurrence and actual module. A module proof
does not authorize arbitrary scopes in the same module. Declaration/header/body
validation uses explicit occurrence roles, including unused functions and public
metadata. Every item product is sealed only after the complete output and EOF
have been parsed. One inline invocation-anchor scalar is then closed with the
exact product count. `Some(0)` certifies consumed empty output, while `None` is
always incomplete. Double close and products appended after close reject.
Final scope validation checks all closed anchors, exact counts and sealed
products. The retained function inventory must still cover each product exactly
once, including unused siblings.

Generated local lookup first respects exact-context lexical bindings. An item
mark with no local binding returns NoLocalBinding, never a fallback into a
captured Root parameter or a fabricated definition-site function. Two emitted
functions cannot share binding authority just because their tokens share a mark.

## Inactive syntax and child IO

The source loader still validates complete original lexemes before attribute
filtering and module observations. Disabled undefined macro calls are syntactically
valid and must not require expansion or macro registration. The loader uses an
explicit sealed, nonplannable inactive-syntax certificate, not an executable
Program or a fake expression value. Unexpanded syntax cannot enter active body
publication. Malformed inactive lexical/delimiter/item syntax still rejects.

This slice does not evaluate generated cfg/cfg_attr or discover generated modules.
It rejects generated attributes/modules before declaration registration or child
lookup. A disabled original invocation does not expand. An active invocation
emitting an unadmitted module is Unsupported with zero generated-child
observations. Generated tokens are never printed, relexed, spliced into prepared
source or sent back through source-file discovery.

## Retained callable publication

Canonical resolution must preserve a checked original-use certificate separately
from the target semantic path. A generated call or type use can legitimately
resolve to an original declaration, but the declaration's source occurrence is
not authority for the generated use. The private certificate binds the exact
canonical target and original use occurrences, collapses to the first use rather
than forming recursive chains, and retains its charged storage. Generic paths
check exact target occurrences before reusing a certificate. Public structural
edits cannot mint or silently transfer one. Target-only parent/slice helpers are
restricted to registry queries, never source AST replacement.

Legitimately admitted emitted Function ASTs are retained in the same Program
before immutable declaration registry and PlanCtx construction. They retain the
same name/module/scope owners and exact checked lexical associations. Existing
canonical source publication may audit that final AST: its private fresh
OriginalBodyId is issued only by the retained-plan capability path, never from an
expansion span or an adapter. No Recipe/builtin provider substitutes for a body.

Publication must use the exact retained Program, declaration, facts/world/budget
and occurrence inventory. A checked mutable AST copy is not canonical publication
authority. No new provider enum name by itself can repair a missing capability.

At this historical checkpoint the parser retained real unit-struct syntax as an
explicit Unit shape so that earlier alias-constructor validation was not bypassed
by an Unsupported fence. Unit shapes had no native value/constructor/ABI admission:
planning rejected them after universal name validation and resolution. Empty
record syntax retained its existing Unsupported boundary. That was deferred
syntax, not unit-struct semantic support.
UNIT-STRUCTS.md records the later bounded native unit-struct implementation;
its layout and scoped-pattern exclusions remain explicit.

## Executed acceptance gates

The original stock-first corpus contained 29 sources: 22 stock-valid and seven rejected.
The original single-function checkpoint's native matrix contained 116
workers-1/8 CHECK/BUILD cells: ten runtime sources, seven ordinary invalid sources
and twelve explicit Unsupported boundaries, with
real executable stdout/exit and no compiler fallback. All earlier source, literal
and nested macro corpora remain unchanged. Thirteen public authority controls
exercise real parsed product/scope/name/body mutations, removals, foreign owners
and exact generic-target certificate binding before planning effects.

The complete gate includes unchanged backend, wide-integer, alias, source-loader
and recursive-macro witnesses. Intermediate runs exposed real debug-stack
overflows, target-only query misuse, an inactive-character/lifetime dispatch bug,
and an `if !condition` shortcut incorrectly treated as a macro invocation. Repairs
reduce retained parser frames, preserve checked semantic query boundaries and
reuse typed token/name predicates. No stack environment, resource cap, old source
witness or acceptance assertion was weakened. Unit diagnostic controls require
the exact established function-context prefix. Twenty-three redacted run records,
including failed attempts, are retained in
`data/runs/xps14/2026-10-17-item-source-macros.jsonl`.

The wide-integer oracle still takes about 12.3 seconds; strict lint reports a
106k-line engine rebuild. These are correctness-run observations, not measured
before/after speedups.

Stock-first source cases cover emitted-function execution, generated locals,
captured parameter versus marked local identity, item names, module inheritance,
source ordering, privacy, duplicate emitted declarations and unused invalid
bodies. Separate public typed controls pin syntax and Unsupported boundaries.
Workers 1 and 8 must pass actual native CHECK and BUILD, executable stdout/exit,
no compiler fallback and no-artifact failure checks. Observation tests verify
zero generated-child IO for inactive and rejected output.

Private controls reject foreign module/session/ledger/proof/product associations,
wrong function-name occurrences, unsealed products, poisoned budgets and attempts
to promote inactive syntax. Complete unchanged regressions, all-target check,
strict lint, formatting and exact scoped receipts are required. Full Rust,
general item macros and performance remain open claims.

## Zero/many-function follow-up (2026-10-19 UTC)

The scoped parser change is confined to `Parser::item_macro_invocation`. One
child parser consumes the single native expansion buffer to its actual EOF,
using the existing function parser once per product. It does not regenerate
source text, clone a whole Program, create per-function source snapshots, or
reparse the whole source file. Empty output follows the same explicit close.
Generated attributes, const functions, generic functions and other item kinds
remain excluded. No function-head, const, token adapter or graph-kind changes
are part of this increment.

Each output keeps a distinct declaration occurrence, module-qualified path and
fresh function scope. Existing `nonmodule_item` registration records each item;
the exact retained Functions are moved into the Program only after complete
consumption, all seals and collision preflight. Each push has paid capacity
admission. Existing declaration-registry construction then issues distinct
`DeclId`/`CallableSource::Free(index)` entries, with the exact original AST still
available for checking. The expansion ID is shared provenance, not callable
identity. A source-snapshot test checks unchanged original bytes and one source
file, separate declaration spans/scopes and shared expansion provenance.

`ItemAnchor` adds only `Option<usize>` completion metadata to the existing paid
anchor element. It introduces no retained vector, resource ledger, mutable
receipt, new mutable graph-fact carrier, or graph node. Only the builder closes
the latest open anchor, after checking owner/proof identity, every product seal
and exact count. Pop/push reuses its already admitted capacity. On failure no
ScopeArena is published, and freeze rechecks complete exact counts. Nested item
expansion remains unsupported, making this latest-anchor discipline explicit.
After freeze the close scalar is immutable. Existing final coverage remains a
bijection, so dropping or duplicating an unused sibling does not erase its
obligation.

Thirteen added sources were measured against stock `rustc 1.98.1` before the
parser change. Exact source bytes, hashes, compiler identity, full diagnostics
and executable results are in `data/runs/macro-many/stock-2026-10-19.json`.
The expanded corpus has 42 stock sources: 27 compile/run and 15 reject. Native
acceptance has 168 workers-1/8 CHECK/BUILD cells and executes 32 images. Native
outcomes are 16 runtime sources, 14 ordinary invalid sources and 12 explicit
phase boundaries. Eleven boundaries are stock-valid excluded syntax. The
remaining boundary is trailing `@`, which stock rejects but the existing native
token adapter rejects earlier as unsupported punctuation. It is not advertised
as syntax-phase parity. Trailing `+` instead reaches the child parser and
rejects as Syntax at the exact expanded token. A malformed second function also
rejects as Syntax. Both diagnostics retain original line/column plus definition
and invocation provenance. No valid first function silently hides leftover
output.

Unused bad first, second and repetition-emitted siblings are independently
rejected in both native modes with workers 1 and 8. The test requires the actual
`expected Int(I32), got Bool` semantic diagnostic, not merely any nonzero exit,
resource failure, contract failure or Unsupported result. Failure leaves no
artifact. Same-expansion mutation controls reject scope/name substitution,
one/two-way body binding-authority transfers, missing one/all siblings and
duplicate products. Private controls require explicit zero close, all seals,
exact count, distinct occurrences and no append/double close. No general
hygiene, procedural-macro, export or `$crate` consumer claim is added.

Scoped passing runs (exported with failed attempts in
`data/runs/macro-many/2026-10-19.jsonl`):

- Final all-target check: `20261018-205007291-3027561`.
- Seven item-scope unit tests: `20261018-204630429-2895379`.
- Twenty-four item-source/authority tests, including strengthened exact semantic
  and expansion-location assertions: `20261018-205007804-3027974`.
- Seven unchanged graph-integrity tests: `20261018-204638764-2898783`.
- Final strict all-target clippy: `20261018-205018924-3038606`.
- Twenty-three unchanged matcher/local/nested/literal tests:
  `20261018-204734049-2906299`.

The initial hosted check passed. A subsequent hosted connection timed out;
these scoped acceptance runs used the stable client's documented automatic
local fallback, confirmed by `justrust show`. Native compiler fallback remains
blocked by the acceptance harness. The isolated target directory is separate
from stable clients and other lanes. No install, primary-checkout mutation,
composed full-suite acceptance or speed claim is made here.
