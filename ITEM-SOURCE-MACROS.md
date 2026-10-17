# Original-source item macros and checked generated functions

Status: implementation in progress after accepted nested-source checkpoint
`9d7356315`. This is a bounded implementation contract, not native acceptance
evidence. No item-position support is claimed until the complete gates pass.

## Intended client

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

Legitimately admitted emitted Function ASTs are retained in the same Program
before immutable declaration registry and PlanCtx construction. They retain the
same name/module/scope owners and exact checked lexical associations. Existing
canonical source publication may audit that final AST: its private fresh
OriginalBodyId is issued only by the retained-plan capability path, never from an
expansion span or an adapter. No Recipe/builtin provider substitutes for a body.

Publication must use the exact retained Program, declaration, facts/world/budget
and occurrence inventory. A checked mutable AST copy is not canonical publication
authority. No new provider enum name by itself can repair a missing capability.

## Gates before acceptance

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
