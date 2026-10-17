# Original-source item macros and checked generated functions

Status: this bounded original-source item client is accepted after nested-source
checkpoint `9d7356315`. Final hosted all-target check
`20261017-102443388-1584603`, complete regression
`20261017-102446184-1584828` (2,156 tests, 106 binaries, zero failed/ignored/filtered),
strict all-target lint `20261017-102503349-1585919`, formatting and diff checks pass.
This is not general item-macro or full Rust support.

## Admitted client

Original module-level macro_rules definitions may be selected by later
unqualified item-position calls. Each invocation must emit exactly one
non-generic free function in the existing supported function grammar. Its name
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
outside this slice. So are empty/multiple-item output, exports, path-qualified
macro calls and user macro calls inside emitted bodies. Existing builtin println
remains its supported path. Legal but unadmitted output must be Unsupported,
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
metadata. The item product is sealed only after exactly one function and EOF
have been parsed. Final scope validation checks all anchors and sealed products.

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

The stock-first corpus contains 29 sources: 22 stock-valid and seven rejected.
The native matrix contains 116 workers-1/8 CHECK/BUILD cells: ten runtime sources,
seven ordinary invalid sources and twelve explicit Unsupported boundaries, with
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
