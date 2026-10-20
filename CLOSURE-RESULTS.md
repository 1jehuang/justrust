# Closure result carrier contracts

Status: bounded source-check prerequisite implemented. The authentic compose-shaped
source is checked, lowered and ownership-validated in all three callable modes.
Reference-bearing result substitutions and native opaque composition remain
unsupported. This is not complete closure-result support.

## First bounded contract

A retained higher-order closure may expose one rigid root result family only
when its original tail is a checked required call with exact existing
parameter-equality result authority. Its parent-minted shape retains an immutable
receipt binding owner, provider, lexical site, original tail ExprId, required
member, ordered trait/method type-witness recipes and argument occurrences.
Recipes contain only canonical ParamIds, Unit and Tuple nodes. They retain no
Ty, TraitCall or Signature. Actual calls and signatures are reconstructed from
the authenticated defining-owner environment and ordered actual bindings.
The receipt is data, not independent callable or result-flow authority.

Every result consumer replays the required-call view in its actual caller world,
environment and phase. The ordinary closure shape/source factory and final
publication replay remain mandatory. Mapping a closure preserves the source
receipt unchanged. Consumption reconstructs each witness through the exact
ordered owner bindings and independently replays the actual call signature.

Existing restricted equality-result obligations survive through closure body,
call and instance admission. A concrete reference-bearing result substitution is
explicitly unsupported. Fixed/nested reference results remain unsupported until
canonical InputCarrier origins and per-mode RefContract rows are implemented.
A rigid parameter is never interpreted as carrier-empty.

The first source-flow slice admits only a tail required call and straight-line
value/call/tuple/field/borrow argument expressions. Early returns, conditional
returns, loops and unknown result producers reject explicitly. No default-empty
provider contract can authorize a result, and no union of arbitrary inputs may
stand in for actual result-flow proof.

## Graph and payload requirements

No new graph operation, global inventory, mutable fact or shared budget.
Receipts contain immutable canonical IDs and finite source type-witness recipes
only. Witness construction/reconstruction charges work, retained bytes and depth
to the existing task budget; source-flow worklists use the exact input edge
inventory for allocation accounting.
No CheckedFunction, CFG, publication, world, facts, SchemaCx or PlanBudget is
reachable from the new receipt. Existing canonical publication and closure seals
are borrowed/replayed, not copied into receipts. CHECK must remain free of
representation, dispatch, mono and codegen demand.

## Acceptance evidence and remaining gates

The focused compiler tests under `frontend/closures/result_contract_tests.rs`
prepare the real prelude and check the actual retained higher-order source. They
replay its sealed source receipt and carrier view, lower its CFG and run ordinary
ownership validation for Fn, FnMut and FnOnce. This is compiler-interface evidence,
not an end-to-end engine worker-1/8 or native execution claim.

Verified on the frozen implementation:
- All three callable modes check/lower/ownership-validate the compose-shaped body.
- Forged owner/provider/site/tail/member/ordered actuals/arguments/result reject.
- Missing or replaced source-call receipts and copied shape seals reject.
- Changed caller world, environment, schema phase and receiver mode reject.
- Direct and nested reference-bearing result substitutions reject.
- Inconsistent reordered owner actuals reject.
- Full graph-integrity gate passes with unchanged ratchets and stack limits.

Authentic LOCAL nightly-2026-10-08 receipts:
- `20261019-175918386-1433107`: all test targets type-check after bounded
  witness/source-edge accounting.
- `20261019-175938401-1437177`: positive all-mode consumer passed; the first
  authority attempt failed only because the test requested schema facts for a
  synthetic callable rather than the original required trait member.
- `20261019-180049624-1441754`: corrected dedicated authority controls passed.
- `20261019-180351238-1453660`: current-parent graph integrity, 14/14 passed.

The existing final source/publication proof remains mandatory. A complete engine
CHECK worker-1/8 and stock/native higher-order return campaign is not claimed by
these focused tests.

No native opaque composition, reference-returning closure, or full corpus-11
claim follows from the source slice.
